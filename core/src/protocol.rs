//! Cabecera y framing del protocolo OPCM v1 (ver docs/PROTOCOL.md).

use std::fmt;

pub const MAGIC: [u8; 4] = *b"OPCM";
pub const VERSION: u8 = 1;
pub const HEADER_LEN: usize = 24;
pub const MAX_PAYLOAD: u32 = 16 * 1024 * 1024;
pub const DEFAULT_PORT: u16 = 4747;
pub const FLAG_KEYFRAME: u16 = 0x0001;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgType {
    Hello,
    Caps,
    Config,
    Video,
    Ping,
    Pong,
    KeyframeRequest,
    Audio,
    Error,
}

impl MsgType {
    pub fn code(self) -> u8 {
        match self {
            MsgType::Hello => 0x01,
            MsgType::Caps => 0x02,
            MsgType::Config => 0x03,
            MsgType::Video => 0x10,
            MsgType::Ping => 0x20,
            MsgType::Pong => 0x21,
            MsgType::KeyframeRequest => 0x30,
            MsgType::Audio => 0x40,
            MsgType::Error => 0x7F,
        }
    }

    /// Devuelve `None` para tipos desconocidos: el receptor los ignora.
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0x01 => MsgType::Hello,
            0x02 => MsgType::Caps,
            0x03 => MsgType::Config,
            0x10 => MsgType::Video,
            0x20 => MsgType::Ping,
            0x21 => MsgType::Pong,
            0x30 => MsgType::KeyframeRequest,
            0x40 => MsgType::Audio,
            0x7F => MsgType::Error,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub msg_type: u8,
    pub flags: u16,
    pub seq: u32,
    pub timestamp_us: u64,
    pub payload_len: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProtoError {
    BadMagic,
    UnsupportedVersion(u8),
    PayloadTooLarge(u32),
}

impl fmt::Display for ProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtoError::BadMagic => write!(f, "magic inválido"),
            ProtoError::UnsupportedVersion(v) => write!(f, "versión no soportada: {v}"),
            ProtoError::PayloadTooLarge(n) => write!(f, "payload demasiado grande: {n} bytes"),
        }
    }
}

impl std::error::Error for ProtoError {}

impl Header {
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[0..4].copy_from_slice(&MAGIC);
        b[4] = VERSION;
        b[5] = self.msg_type;
        b[6..8].copy_from_slice(&self.flags.to_be_bytes());
        b[8..12].copy_from_slice(&self.seq.to_be_bytes());
        b[12..20].copy_from_slice(&self.timestamp_us.to_be_bytes());
        b[20..24].copy_from_slice(&self.payload_len.to_be_bytes());
        b
    }

    pub fn decode(b: &[u8; HEADER_LEN]) -> Result<Self, ProtoError> {
        if b[0..4] != MAGIC {
            return Err(ProtoError::BadMagic);
        }
        if b[4] != VERSION {
            return Err(ProtoError::UnsupportedVersion(b[4]));
        }
        let payload_len = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
        if payload_len > MAX_PAYLOAD {
            return Err(ProtoError::PayloadTooLarge(payload_len));
        }
        Ok(Header {
            msg_type: b[5],
            flags: u16::from_be_bytes([b[6], b[7]]),
            seq: u32::from_be_bytes([b[8], b[9], b[10], b[11]]),
            timestamp_us: u64::from_be_bytes([b[12], b[13], b[14], b[15], b[16], b[17], b[18], b[19]]),
            payload_len,
        })
    }

    pub fn is_keyframe(&self) -> bool {
        self.flags & FLAG_KEYFRAME != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_header() {
        let h = Header { msg_type: 0x10, flags: FLAG_KEYFRAME, seq: 7, timestamp_us: 1_234_567, payload_len: 42 };
        let decoded = Header::decode(&h.encode()).unwrap();
        assert_eq!(decoded, h);
        assert!(decoded.is_keyframe());
    }

    #[test]
    fn rejects_bad_magic() {
        let mut b = Header { msg_type: 1, flags: 0, seq: 0, timestamp_us: 0, payload_len: 0 }.encode();
        b[0] = b'X';
        assert_eq!(Header::decode(&b), Err(ProtoError::BadMagic));
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut b = Header { msg_type: 1, flags: 0, seq: 0, timestamp_us: 0, payload_len: 0 }.encode();
        b[4] = 2;
        assert_eq!(Header::decode(&b), Err(ProtoError::UnsupportedVersion(2)));
    }

    #[test]
    fn rejects_oversized_payload() {
        let mut b = Header { msg_type: 1, flags: 0, seq: 0, timestamp_us: 0, payload_len: 0 }.encode();
        b[20..24].copy_from_slice(&(MAX_PAYLOAD + 1).to_be_bytes());
        assert_eq!(Header::decode(&b), Err(ProtoError::PayloadTooLarge(MAX_PAYLOAD + 1)));
    }

    #[test]
    fn msg_type_codes_roundtrip() {
        for code in [0x01, 0x02, 0x03, 0x10, 0x20, 0x21, 0x30, 0x40, 0x7F] {
            assert_eq!(MsgType::from_code(code).unwrap().code(), code);
        }
        assert_eq!(MsgType::from_code(0xEE), None);
    }
}
