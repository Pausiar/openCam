//! Cliente OPCM del escritorio: se conecta al móvil (puerto 4747), negocia y recibe vídeo.
//!
//! Flujo: HELLO → CAPS → CONFIG → VIDEO continuo, con PING cada segundo para medir latencia.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use opencam_core::protocol::{Header, MsgType, HEADER_LEN};

pub struct Message {
    pub msg_type: u8,
    pub flags: u16,
    pub timestamp_us: u64,
    pub payload: Vec<u8>,
}

/// Conexión OPCM ya establecida con el móvil.
pub struct Client {
    stream: Box<dyn Stream>,
    seq: u32,
    epoch: Instant,
}

impl Client {
    pub fn connect(addr: impl ToSocketAddrs, timeout: Duration) -> io::Result<Self> {
        let addr = addr.to_socket_addrs()?.next().ok_or_else(|| io::Error::other("dirección no válida"))?;
        let stream = TcpStream::connect_timeout(&addr, timeout)?;
        stream.set_nodelay(true)?;
        Ok(Self { stream: Box::new(stream), seq: 0, epoch: Instant::now() })
    }

    /// Envuelve un stream ya abierto (p. ej. túnel de usbmuxd).
    pub fn from_stream(stream: Box<dyn Stream>) -> Self {
        Self { stream, seq: 0, epoch: Instant::now() }
    }

    /// Microsegundos desde el arranque del cliente; son el `timestamp_us` de los PING.
    pub fn now_us(&self) -> u64 {
        self.epoch.elapsed().as_micros() as u64
    }

    /// Origen de tiempo compartido con hilos que envían PING.
    pub fn epoch(&self) -> Instant {
        self.epoch
    }

    pub fn send(&mut self, msg_type: MsgType, flags: u16, payload: &[u8]) -> io::Result<()> {
        let header = Header {
            msg_type: msg_type.code(),
            flags,
            seq: self.seq,
            timestamp_us: self.now_us(),
            payload_len: payload.len() as u32,
        };
        self.seq = self.seq.wrapping_add(1);
        self.stream.write_all(&header.encode())?;
        self.stream.write_all(payload)?;
        self.stream.flush()
    }

    /// Lee un mensaje completo. Valida magic, versión y tamaño antes de reservar memoria.
    pub fn recv(&mut self) -> io::Result<Message> {
        let mut raw = [0u8; HEADER_LEN];
        self.stream.read_exact(&mut raw)?;
        let header = Header::decode(&raw).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let mut payload = vec![0u8; header.payload_len as usize];
        self.stream.read_exact(&mut payload)?;
        Ok(Message { msg_type: header.msg_type, flags: header.flags, timestamp_us: header.timestamp_us, payload })
    }

    /// Clona el socket para enviar PING desde otro hilo sin bloquear la lectura.
    pub fn try_clone_stream(&self) -> io::Result<Box<dyn Stream>> {
        self.stream.try_clone_boxed()
    }
}

/// Stream bidireccional con clonación, común a TCP (Android, Wi-Fi) y Unix (túnel usbmuxd).
pub trait Stream: Read + Write + Send {
    fn try_clone_boxed(&self) -> io::Result<Box<dyn Stream>>;
}

impl Stream for TcpStream {
    fn try_clone_boxed(&self) -> io::Result<Box<dyn Stream>> {
        Ok(Box::new(self.try_clone()?))
    }
}

#[cfg(unix)]
impl Stream for UnixStream {
    fn try_clone_boxed(&self) -> io::Result<Box<dyn Stream>> {
        Ok(Box::new(self.try_clone()?))
    }
}

/// Handshake completo: HELLO del escritorio, espera CAPS y envía CONFIG.
pub fn handshake(client: &mut Client, config_json: &str) -> io::Result<String> {
    client.send(MsgType::Hello, 0, br#"{"proto":1,"platform":"desktop","name":"OpenCam"}"#)?;
    let mut caps = None;
    // El móvil responde HELLO y CAPS; se ignoran el resto de tipos mientras tanto.
    for _ in 0..4 {
        let msg = client.recv()?;
        if msg.msg_type == MsgType::Caps.code() {
            caps = Some(String::from_utf8_lossy(&msg.payload).into_owned());
            break;
        }
    }
    let caps = caps.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "el móvil no envió CAPS"))?;
    client.send(MsgType::Config, 0, config_json.as_bytes())?;
    Ok(caps)
}

/// Latencia en microsegundos de un PONG: `ahora − timestamp_us` del PING original.
pub fn pong_latency_us(client: &Client, pong: &Message) -> u64 {
    client.now_us().saturating_sub(pong.timestamp_us)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    /// Simula el móvil: responde HELLO+CAPS, recibe CONFIG y devuelve el PING como PONG.
    #[test]
    fn handshake_and_pong_roundtrip() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let phone = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let read_msg = |s: &mut TcpStream| -> (u8, u64, Vec<u8>) {
                let mut raw = [0u8; HEADER_LEN];
                s.read_exact(&mut raw).unwrap();
                let h = Header::decode(&raw).unwrap();
                let mut p = vec![0u8; h.payload_len as usize];
                s.read_exact(&mut p).unwrap();
                (h.msg_type, h.timestamp_us, p)
            };
            let write_msg = |s: &mut TcpStream, t: MsgType, ts: u64, p: &[u8]| {
                let h = Header { msg_type: t.code(), flags: 0, seq: 0, timestamp_us: ts, payload_len: p.len() as u32 };
                s.write_all(&h.encode()).unwrap();
                s.write_all(p).unwrap();
            };

            let (t, _, _) = read_msg(&mut s);
            assert_eq!(t, MsgType::Hello.code());
            write_msg(&mut s, MsgType::Hello, 0, br#"{"proto":1,"platform":"android"}"#);
            write_msg(&mut s, MsgType::Caps, 0, br#"{"platform":"android"}"#);

            let (t, _, cfg) = read_msg(&mut s);
            assert_eq!(t, MsgType::Config.code());
            assert!(String::from_utf8_lossy(&cfg).contains("\"codec\":\"h264\""));

            let (t, ts, _) = read_msg(&mut s);
            assert_eq!(t, MsgType::Ping.code());
            write_msg(&mut s, MsgType::Pong, ts, &[]);
        });

        let mut client = Client::connect(addr, Duration::from_secs(2)).unwrap();
        let caps = handshake(&mut client, r#"{"codec":"h264"}"#).unwrap();
        assert!(caps.contains("android"));

        client.send(MsgType::Ping, 0, &[]).unwrap();
        let pong = client.recv().unwrap();
        assert_eq!(pong.msg_type, MsgType::Pong.code());
        assert!(pong.timestamp_us <= client.now_us());
        phone.join().unwrap();
    }
}
