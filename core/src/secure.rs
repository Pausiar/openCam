//! Emparejamiento y cifrado de la conexión OPCM (ver docs/PROTOCOL.md, «Seguridad»).
//!
//! - Intercambio de claves ECDH P-256 efímero en cada conexión (secreto hacia adelante).
//! - La clave precompartida (PSK) es el código de 6 dígitos que muestra el móvil o, tras el
//!   primer emparejamiento, una clave de 32 bytes que ambos lados guardan.
//! - HKDF-SHA256 deriva las claves de sesión; AUTH confirma con HMAC que los dos lados
//!   conocen la PSK; después cada payload va cifrado con AES-256-GCM y la cabecera OPCM
//!   como dato autenticado.
//!
//! Las mismas primitivas existen en Android (JCA) e iOS (CryptoKit). Los vectores de prueba
//! de `tests::vector` se repiten en los tests de Android para comprobar que interoperan.

use std::fmt;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use p256::{PublicKey, SecretKey};
use sha2::{Digest, Sha256};

use crate::protocol::{Header, HEADER_LEN};

/// Etiqueta GCM añadida a cada payload cifrado.
pub const TAG_LEN: usize = 16;
/// Punto P-256 sin comprimir (0x04 || X || Y).
pub const PUBLIC_KEY_LEN: usize = 65;
/// Antes de autenticarse nadie puede hacer reservar más memoria que esto.
pub const MAX_UNAUTH_PAYLOAD: u32 = 64 * 1024;
/// Intentos fallidos con el mismo código antes de que el móvil genere otro.
pub const MAX_CODE_ATTEMPTS: u32 = 5;

const INFO: &[u8] = b"opencam-v1";
const CODE_SALT: &[u8] = b"opencam-pair-code:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecureError {
    BadPublicKey,
    BadCode,
    AuthFailed,
    Decrypt,
    CounterExhausted,
    Random,
}

impl fmt::Display for SecureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SecureError::BadPublicKey => "clave pública inválida",
            SecureError::BadCode => "el código de emparejamiento debe tener 6 dígitos",
            SecureError::AuthFailed => "autenticación fallida (código o clave incorrectos)",
            SecureError::Decrypt => "mensaje cifrado inválido o manipulado",
            SecureError::CounterExhausted => "contador de cifrado agotado",
            SecureError::Random => "no hay fuente de aleatoriedad",
        })
    }
}

impl std::error::Error for SecureError {}

/// Rol en la conexión: cada uno cifra con su propia clave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Desktop,
    Phone,
}

impl Role {
    fn label(self) -> &'static [u8] {
        match self {
            Role::Desktop => b"desktop",
            Role::Phone => b"phone",
        }
    }
}

/// Bytes aleatorios del sistema operativo.
pub fn random_bytes<const N: usize>() -> Result<[u8; N], SecureError> {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).map_err(|_| SecureError::Random)?;
    Ok(b)
}

/// Normaliza un código tecleado («123 456», «123-456») a sus 6 dígitos.
pub fn normalize_code(code: &str) -> Result<String, SecureError> {
    let digits: String = code
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    if digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_digit()) {
        Ok(digits)
    } else {
        Err(SecureError::BadCode)
    }
}

/// PSK a partir del código de emparejamiento.
pub fn code_psk(code: &str) -> Result<[u8; 32], SecureError> {
    let code = normalize_code(code)?;
    let mut h = Sha256::new();
    h.update(CODE_SALT);
    h.update(code.as_bytes());
    Ok(h.finalize().into())
}

/// Par de claves ECDH efímero.
pub struct KeyPair {
    secret: SecretKey,
}

impl KeyPair {
    pub fn generate() -> Result<Self, SecureError> {
        // Un escalar aleatorio de 32 bytes es válido salvo con probabilidad ~2^-128.
        loop {
            if let Ok(k) = Self::from_bytes(&random_bytes::<32>()?) {
                return Ok(k);
            }
        }
    }

    /// Solo para vectores de prueba.
    pub fn from_bytes(b: &[u8; 32]) -> Result<Self, SecureError> {
        SecretKey::from_slice(b)
            .map(|secret| Self { secret })
            .map_err(|_| SecureError::BadPublicKey)
    }

    pub fn public_bytes(&self) -> [u8; PUBLIC_KEY_LEN] {
        let p = self.secret.public_key().to_sec1_bytes();
        let mut out = [0u8; PUBLIC_KEY_LEN];
        out.copy_from_slice(&p);
        out
    }
}

/// Claves de una sesión ya acordada.
pub struct SessionKeys {
    desktop_to_phone: [u8; 32],
    phone_to_desktop: [u8; 32],
    confirm: [u8; 32],
    /// Clave a largo plazo que ambos guardan tras emparejar con código.
    pub pairing_key: [u8; 32],
}

/// Resumen de los dos HELLO: ata las claves a lo que cada lado anunció.
pub fn transcript_hash(desktop_hello: &[u8], phone_hello: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update((desktop_hello.len() as u32).to_be_bytes());
    h.update(desktop_hello);
    h.update((phone_hello.len() as u32).to_be_bytes());
    h.update(phone_hello);
    h.finalize().into()
}

impl SessionKeys {
    pub fn derive(
        own: &KeyPair,
        peer_public: &[u8],
        psk: &[u8; 32],
        desktop_hello: &[u8],
        phone_hello: &[u8],
    ) -> Result<Self, SecureError> {
        if peer_public.len() != PUBLIC_KEY_LEN || peer_public[0] != 0x04 {
            return Err(SecureError::BadPublicKey);
        }
        let peer =
            PublicKey::from_sec1_bytes(peer_public).map_err(|_| SecureError::BadPublicKey)?;
        let shared = p256::ecdh::diffie_hellman(own.secret.to_nonzero_scalar(), peer.as_affine());
        let mut info = INFO.to_vec();
        info.extend_from_slice(&transcript_hash(desktop_hello, phone_hello));
        let hk = Hkdf::<Sha256>::new(Some(psk), shared.raw_secret_bytes());
        let mut okm = [0u8; 128];
        hk.expand(&info, &mut okm)
            .expect("128 bytes es una longitud válida para HKDF-SHA256");
        let part = |i: usize| -> [u8; 32] { okm[i * 32..(i + 1) * 32].try_into().unwrap() };
        Ok(Self {
            desktop_to_phone: part(0),
            phone_to_desktop: part(1),
            confirm: part(2),
            pairing_key: part(3),
        })
    }

    /// Payload de AUTH que envía `role`.
    pub fn auth_tag(&self, role: Role) -> [u8; 32] {
        let mut m = <Hmac<Sha256> as Mac>::new_from_slice(&self.confirm).unwrap();
        m.update(role.label());
        m.finalize().into_bytes().into()
    }

    /// Comprueba en tiempo constante el AUTH recibido de `role`.
    pub fn verify_auth(&self, role: Role, tag: &[u8]) -> Result<(), SecureError> {
        let mut m = <Hmac<Sha256> as Mac>::new_from_slice(&self.confirm).unwrap();
        m.update(role.label());
        m.verify_slice(tag).map_err(|_| SecureError::AuthFailed)
    }

    /// Cifradores (envío, recepción) para `role`.
    pub fn ciphers(&self, role: Role) -> (Cipher, Cipher) {
        let d2p = Cipher::new(&self.desktop_to_phone);
        let p2d = Cipher::new(&self.phone_to_desktop);
        match role {
            Role::Desktop => (d2p, p2d),
            Role::Phone => (p2d, d2p),
        }
    }
}

/// AES-256-GCM en un sentido. El nonce es un contador implícito: un mensaje repetido,
/// reordenado o perdido rompe el descifrado y la conexión se cierra.
pub struct Cipher {
    aead: Aes256Gcm,
    counter: u64,
}

impl Cipher {
    pub fn new(key: &[u8; 32]) -> Self {
        Self {
            aead: Aes256Gcm::new(key.into()),
            counter: 0,
        }
    }

    fn next_nonce(&mut self) -> Result<[u8; 12], SecureError> {
        let mut n = [0u8; 12];
        n[4..].copy_from_slice(&self.counter.to_be_bytes());
        self.counter = self
            .counter
            .checked_add(1)
            .ok_or(SecureError::CounterExhausted)?;
        Ok(n)
    }

    /// Cifra `plaintext`. Ajusta `header.payload_len` y devuelve la cabecera codificada
    /// (que va en claro y autenticada) junto al payload cifrado.
    pub fn seal(
        &mut self,
        mut header: Header,
        plaintext: &[u8],
    ) -> Result<([u8; HEADER_LEN], Vec<u8>), SecureError> {
        header.payload_len = (plaintext.len() + TAG_LEN) as u32;
        let aad = header.encode();
        let nonce = self.next_nonce()?;
        let ct = self
            .aead
            .encrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| SecureError::Decrypt)?;
        Ok((aad, ct))
    }

    /// Descifra un payload recibido con su cabecera tal y como llegó.
    pub fn open(
        &mut self,
        header: &[u8; HEADER_LEN],
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, SecureError> {
        let nonce = self.next_nonce()?;
        self.aead
            .decrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: ciphertext,
                    aad: header,
                },
            )
            .map_err(|_| SecureError::Decrypt)
    }
}

/// Hexadecimal en minúsculas (claves públicas e ids en el JSON de HELLO).
pub fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::MsgType;

    fn hdr(t: MsgType) -> Header {
        Header {
            msg_type: t.code(),
            flags: 0,
            seq: 3,
            timestamp_us: 99,
            payload_len: 0,
        }
    }

    fn pair(psk_d: &[u8; 32], psk_p: &[u8; 32]) -> (SessionKeys, SessionKeys) {
        let d = KeyPair::generate().unwrap();
        let p = KeyPair::generate().unwrap();
        let (hd, hp) = (b"hello-d".as_slice(), b"hello-p".as_slice());
        (
            SessionKeys::derive(&d, &p.public_bytes(), psk_d, hd, hp).unwrap(),
            SessionKeys::derive(&p, &d.public_bytes(), psk_p, hd, hp).unwrap(),
        )
    }

    #[test]
    fn same_code_agrees_and_encrypts_both_ways() {
        let psk = code_psk("123456").unwrap();
        let (kd, kp) = pair(&psk, &psk);
        kp.verify_auth(Role::Desktop, &kd.auth_tag(Role::Desktop))
            .unwrap();
        kd.verify_auth(Role::Phone, &kp.auth_tag(Role::Phone))
            .unwrap();
        assert_eq!(kd.pairing_key, kp.pairing_key);

        let (mut d_tx, mut d_rx) = kd.ciphers(Role::Desktop);
        let (mut p_tx, mut p_rx) = kp.ciphers(Role::Phone);
        for i in 0..3u8 {
            let (h, ct) = d_tx.seal(hdr(MsgType::Config), &[i; 10]).unwrap();
            assert_eq!(Header::decode(&h).unwrap().payload_len as usize, ct.len());
            assert_eq!(ct.len(), 10 + TAG_LEN);
            assert_eq!(p_rx.open(&h, &ct).unwrap(), vec![i; 10]);
            let (h, ct) = p_tx.seal(hdr(MsgType::Video), b"frame").unwrap();
            assert_eq!(d_rx.open(&h, &ct).unwrap(), b"frame");
        }
    }

    #[test]
    fn wrong_code_fails_auth() {
        let (kd, kp) = pair(&code_psk("123456").unwrap(), &code_psk("123457").unwrap());
        assert_eq!(
            kp.verify_auth(Role::Desktop, &kd.auth_tag(Role::Desktop)),
            Err(SecureError::AuthFailed)
        );
        // Un tag de un rol no vale por el otro (no se puede reflejar).
        assert!(kd
            .verify_auth(Role::Phone, &kd.auth_tag(Role::Desktop))
            .is_err());
    }

    #[test]
    fn tampering_replay_and_reorder_are_rejected() {
        let psk = [7u8; 32];
        let (kd, kp) = pair(&psk, &psk);
        let (mut tx, _) = kd.ciphers(Role::Desktop);
        let (_, mut rx) = kp.ciphers(Role::Phone);
        let (h1, c1) = tx.seal(hdr(MsgType::Stats), b"uno").unwrap();
        let (h2, c2) = tx.seal(hdr(MsgType::Stats), b"dos").unwrap();

        // Cabecera alterada (otro tipo de mensaje): falla.
        let mut bad_h = h1;
        bad_h[5] = MsgType::Config.code();
        let (_, mut rx2) = kp.ciphers(Role::Phone);
        assert_eq!(rx2.open(&bad_h, &c1), Err(SecureError::Decrypt));
        // Payload alterado: falla.
        let mut bad_c = c1.clone();
        bad_c[0] ^= 1;
        let (_, mut rx3) = kp.ciphers(Role::Phone);
        assert_eq!(rx3.open(&h1, &bad_c), Err(SecureError::Decrypt));
        // Reordenado: falla.
        let (_, mut rx4) = kp.ciphers(Role::Phone);
        assert!(rx4.open(&h2, &c2).is_err());

        assert_eq!(rx.open(&h1, &c1).unwrap(), b"uno");
        // Repetido: falla.
        assert!(rx.open(&h1, &c1).is_err());
    }

    #[test]
    fn transcript_binds_hellos() {
        let psk = [1u8; 32];
        let d = KeyPair::generate().unwrap();
        let p = KeyPair::generate().unwrap();
        let a = SessionKeys::derive(&d, &p.public_bytes(), &psk, b"a", b"b").unwrap();
        let b = SessionKeys::derive(&p, &d.public_bytes(), &psk, b"a", b"B").unwrap();
        assert!(b
            .verify_auth(Role::Desktop, &a.auth_tag(Role::Desktop))
            .is_err());
        // Los límites entre HELLO importan: ("ab","") ≠ ("a","b").
        assert_ne!(transcript_hash(b"ab", b""), transcript_hash(b"a", b"b"));
    }

    #[test]
    fn rejects_invalid_public_keys() {
        let k = KeyPair::generate().unwrap();
        let psk = [0u8; 32];
        for bad in [vec![], vec![4u8; 65], vec![2u8; 33], {
            let mut v = k.public_bytes().to_vec();
            v[64] ^= 1; // fuera de la curva
            v
        }] {
            assert_eq!(
                SessionKeys::derive(&k, &bad, &psk, b"", b"").err(),
                Some(SecureError::BadPublicKey)
            );
        }
    }

    #[test]
    fn code_normalization() {
        assert_eq!(normalize_code(" 123-456 ").unwrap(), "123456");
        assert_eq!(normalize_code("123 456").unwrap(), "123456");
        for bad in ["12345", "1234567", "12a456", ""] {
            assert_eq!(normalize_code(bad), Err(SecureError::BadCode));
        }
        assert_eq!(code_psk("123 456"), code_psk("123456"));
        assert_ne!(code_psk("123456"), code_psk("654321"));
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(to_hex(&[0, 0xab, 0x10]), "00ab10");
        assert_eq!(from_hex("00ab10").unwrap(), vec![0, 0xab, 0x10]);
        assert_eq!(from_hex("0"), None);
        assert_eq!(from_hex("zz"), None);
        assert_eq!(from_hex("ñ1"), None);
    }

    /// Vector fijo compartido con `SecureTest.kt` (Android): si cambia, cambia el protocolo.
    #[test]
    fn vector() {
        let d = KeyPair::from_bytes(&[0x11; 32]).unwrap();
        let p = KeyPair::from_bytes(&[0x22; 32]).unwrap();
        let psk = code_psk("123456").unwrap();
        let k =
            SessionKeys::derive(&d, &p.public_bytes(), &psk, b"{\"d\":1}", b"{\"p\":1}").unwrap();
        let (mut tx, _) = k.ciphers(Role::Desktop);
        let (h, ct) = tx.seal(hdr(MsgType::Config), b"hola").unwrap();
        let got = [
            to_hex(&d.public_bytes()),
            to_hex(&psk),
            to_hex(&k.auth_tag(Role::Desktop)),
            to_hex(&k.auth_tag(Role::Phone)),
            to_hex(&k.pairing_key),
            to_hex(&h),
            to_hex(&ct),
        ];
        assert_eq!(got, VECTOR);
    }

    pub(crate) const VECTOR: [&str; 7] = [
        // clave pública del escritorio (privada = 0x11 × 32)
        "040217e617f0b6443928278f96999e69a23a4f2c152bdf6d6cdf66e5b80282d4ed194a7debcb97712d2dda3ca85aa8765a56f45fc758599652f2897c65306e5794",
        // PSK del código 123456
        "ea64913738e9cba75bb0b11f5762959a7a65e75f4dadc994cbe65c98a65e35ae",
        // AUTH del escritorio
        "5c91459ea4bd095b6654962bb8c378c74f913b7dc625787be52d40d58fd697d6",
        // AUTH del móvil
        "26e992cdf4b6ce8226a89703bfcc829c31df2aa5073a04c81da1442c330489c3",
        // clave de emparejamiento
        "9083aaa25d1afa6d4c90608dac80551c531debb56de6a4af6bb2bcbe06ef6fe8",
        // cabecera del primer CONFIG cifrado («hola»)
        "4f50434d0103000000000003000000000000006300000014",
        // payload cifrado
        "98d8b0415dcaf17c08ad6e0d3f0ff53487dc0976",
    ];
}
