//! Cliente OPCM del escritorio: se conecta al móvil (puerto 4747), negocia y recibe vídeo.
//!
//! Flujo: HELLO → (AUTH cifrado si el móvil lo soporta) → CAPS → CONFIG → VIDEO continuo,
//! con PING cada segundo para medir el RTT.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use std::fmt;

use opencam_core::protocol::{Header, MsgType, ProtoError, HEADER_LEN, MAX_PAYLOAD};
use opencam_core::secure::{
    code_psk, from_hex, to_hex, Cipher, KeyPair, Role, SessionKeys, MAX_UNAUTH_PAYLOAD,
};

use crate::pairing::PairingStore;

pub struct Message {
    pub msg_type: u8,
    pub flags: u16,
    pub timestamp_us: u64,
    pub payload: Vec<u8>,
}

/// Escritor compartido: un solo `seq` y exclusión mutua entre el hilo principal y el de PING.
struct Writer {
    stream: Box<dyn Stream>,
    seq: u32,
    epoch: Instant,
    /// Cifrado de salida, activo tras AUTH.
    cipher: Option<Cipher>,
}

/// Manejador clonable para enviar mensajes desde cualquier hilo.
#[derive(Clone)]
pub struct Sender(Arc<Mutex<Writer>>);

impl Sender {
    pub fn send(&self, msg_type: MsgType, flags: u16, payload: &[u8]) -> io::Result<()> {
        let mut w = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let header = Header {
            msg_type: msg_type.code(),
            flags,
            seq: w.seq,
            timestamp_us: w.epoch.elapsed().as_micros() as u64,
            payload_len: payload.len() as u32,
        };
        w.seq = w.seq.wrapping_add(1);
        if let Some(c) = w.cipher.as_mut() {
            let (h, ct) = c
                .seal(header, payload)
                .map_err(|e| io::Error::other(e.to_string()))?;
            w.stream.write_all(&h)?;
            w.stream.write_all(&ct)?;
        } else {
            w.stream.write_all(&header.encode())?;
            w.stream.write_all(payload)?;
        }
        w.stream.flush()
    }

    /// Cierra el socket para todos los clones (el lector y el hilo de PING fallan al instante).
    pub fn close(&self) {
        let _ = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stream
            .shutdown();
    }
}

/// Conexión OPCM ya establecida con el móvil.
pub struct Client {
    reader: Box<dyn Stream>,
    tx: Sender,
    epoch: Instant,
    /// JSON de CAPS recibido en el handshake (vacío antes).
    pub caps: String,
    /// Cifrado de entrada, activo tras AUTH.
    rx_cipher: Option<Cipher>,
    /// Tamaño máximo de payload: bajo hasta terminar el handshake.
    max_payload: u32,
    /// `true` si la sesión va cifrada y autenticada.
    secure: bool,
}

impl Client {
    pub fn connect(addr: impl ToSocketAddrs, timeout: Duration) -> io::Result<Self> {
        let addr = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::other("dirección no válida"))?;
        let stream = TcpStream::connect_timeout(&addr, timeout)?;
        stream.set_nodelay(true)?;
        Self::from_stream(Box::new(stream))
    }

    /// Envuelve un stream ya abierto (p. ej. túnel de usbmuxd).
    pub fn from_stream(stream: Box<dyn Stream>) -> io::Result<Self> {
        let epoch = Instant::now();
        let writer = Writer {
            stream: stream.try_clone_boxed()?,
            seq: 0,
            epoch,
            cipher: None,
        };
        Ok(Self {
            reader: stream,
            tx: Sender(Arc::new(Mutex::new(writer))),
            epoch,
            caps: String::new(),
            rx_cipher: None,
            max_payload: MAX_UNAUTH_PAYLOAD,
            secure: false,
        })
    }

    /// Microsegundos desde el arranque del cliente; son el `timestamp_us` de los PING.
    pub fn now_us(&self) -> u64 {
        self.epoch.elapsed().as_micros() as u64
    }

    /// `true` si la conexión va cifrada (móvil con emparejamiento).
    pub fn is_secure(&self) -> bool {
        self.secure
    }

    fn enable_encryption(&mut self, tx: Cipher, rx: Cipher) {
        self.tx.0.lock().unwrap_or_else(|e| e.into_inner()).cipher = Some(tx);
        self.rx_cipher = Some(rx);
        self.secure = true;
    }

    pub fn sender(&self) -> Sender {
        self.tx.clone()
    }

    pub fn send(&self, msg_type: MsgType, flags: u16, payload: &[u8]) -> io::Result<()> {
        self.tx.send(msg_type, flags, payload)
    }

    pub fn close(&self) {
        self.tx.close();
    }

    /// PING cada segundo desde un hilo aparte; termina cuando el socket falla o se cierra.
    pub fn spawn_pinger(&self) {
        let tx = self.sender();
        std::thread::spawn(move || {
            while tx.send(MsgType::Ping, 0, &[]).is_ok() {
                std::thread::sleep(Duration::from_secs(1));
            }
        });
    }

    /// Lee un mensaje completo. Valida magic, versión y tamaño antes de reservar memoria.
    /// Ante una versión no soportada envía ERROR `unsupported_version` y cierra la conexión.
    pub fn recv(&mut self) -> io::Result<Message> {
        let mut raw = [0u8; HEADER_LEN];
        self.reader.read_exact(&mut raw)?;
        let header = match Header::decode(&raw) {
            Ok(h) => h,
            Err(e) => {
                if let ProtoError::UnsupportedVersion(v) = e {
                    let body = format!(
                        r#"{{"code":"unsupported_version","message":"versión {v} no soportada; se espera 1"}}"#
                    );
                    let _ = self.send(MsgType::Error, 0, body.as_bytes());
                    self.close();
                }
                return Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string()));
            }
        };
        if header.payload_len > self.max_payload {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "payload de {} bytes antes del handshake",
                    header.payload_len
                ),
            ));
        }
        let mut payload = vec![0u8; header.payload_len as usize];
        self.reader.read_exact(&mut payload)?;
        if let Some(c) = self.rx_cipher.as_mut() {
            payload = c
                .open(&raw, &payload)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        }
        Ok(Message {
            msg_type: header.msg_type,
            flags: header.flags,
            timestamp_us: header.timestamp_us,
            payload,
        })
    }
}

/// Stream bidireccional con clonación, común a TCP (Android, Wi-Fi) y Unix (túnel usbmuxd).
pub trait Stream: Read + Write + Send {
    fn try_clone_boxed(&self) -> io::Result<Box<dyn Stream>>;
    fn set_read_timeout(&self, dur: Option<Duration>) -> io::Result<()>;
    fn shutdown(&self) -> io::Result<()>;
}

impl Stream for TcpStream {
    fn try_clone_boxed(&self) -> io::Result<Box<dyn Stream>> {
        Ok(Box::new(self.try_clone()?))
    }
    fn set_read_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
        TcpStream::set_read_timeout(self, dur)
    }
    fn shutdown(&self) -> io::Result<()> {
        TcpStream::shutdown(self, Shutdown::Both)
    }
}

#[cfg(unix)]
impl Stream for UnixStream {
    fn try_clone_boxed(&self) -> io::Result<Box<dyn Stream>> {
        Ok(Box::new(self.try_clone()?))
    }
    fn set_read_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
        UnixStream::set_read_timeout(self, dur)
    }
    fn shutdown(&self) -> io::Result<()> {
        UnixStream::shutdown(self, Shutdown::Both)
    }
}

/// Fallos de emparejamiento: reintentar no sirve, hace falta que el usuario actúe.
/// Viajan dentro de un `io::Error` de tipo `PermissionDenied`; la UI puede hacer
/// `err.get_ref().and_then(|e| e.downcast_ref::<PairingError>())`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingError {
    /// El móvil pide el código que muestra en pantalla.
    NeedCode,
    /// El código no coincide con el del móvil.
    BadCode,
    /// El móvil recuerda este PC pero el PC perdió la clave (o al revés).
    KeyMismatch,
    /// El móvil no demostró conocer el código: posible suplantación.
    PhoneNotAuthenticated,
    /// El móvil exige emparejamiento y el mensaje no se entendió.
    Rejected(String),
}

impl fmt::Display for PairingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PairingError::NeedCode => write!(
                f,
                "el móvil pide emparejar: escribe el código de 6 dígitos que muestra (--code)"
            ),
            PairingError::BadCode => write!(f, "código de emparejamiento incorrecto"),
            PairingError::KeyMismatch => write!(
                f,
                "el emparejamiento guardado no coincide; vuelve a emparejar con --code"
            ),
            PairingError::PhoneNotAuthenticated => write!(
                f,
                "el móvil no se autenticó; puede que otro equipo se esté haciendo pasar por él"
            ),
            PairingError::Rejected(m) => write!(f, "el móvil rechazó la conexión: {m}"),
        }
    }
}

impl std::error::Error for PairingError {}

impl PairingError {
    fn into_io(self) -> io::Error {
        io::Error::new(io::ErrorKind::PermissionDenied, self)
    }

    /// Extrae el error de emparejamiento de un `io::Error`, si lo es.
    pub fn from_io(e: &io::Error) -> Option<&PairingError> {
        e.get_ref().and_then(|e| e.downcast_ref::<PairingError>())
    }
}

/// Opciones de seguridad del handshake.
pub struct Pairing<'a> {
    pub store: &'a mut PairingStore,
    /// Código tecleado por el usuario; obliga a emparejar de nuevo aunque haya clave.
    pub code: Option<&'a str>,
}

/// Handshake con un almacén de emparejamientos en memoria y sin código: solo vale para
/// móviles antiguos sin cifrado. La aplicación usa `handshake_with`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn handshake(client: &mut Client, config_json: &str, timeout: Duration) -> io::Result<String> {
    let mut store = PairingStore::in_memory();
    handshake_with(
        client,
        config_json,
        timeout,
        Pairing {
            store: &mut store,
            code: None,
        },
    )
}

/// Handshake completo: HELLO del escritorio, AUTH si el móvil lo pide, espera CAPS y
/// envía CONFIG. Un móvil mudo no bloquea: cada lectura caduca a los `timeout`.
pub fn handshake_with(
    client: &mut Client,
    config_json: &str,
    timeout: Duration,
    pairing: Pairing,
) -> io::Result<String> {
    client.reader.set_read_timeout(Some(timeout))?;
    let result = handshake_inner(client, config_json, pairing);
    let _ = client.reader.set_read_timeout(None);
    client.max_payload = MAX_PAYLOAD;
    result.map_err(|e| match e.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {
            io::Error::new(io::ErrorKind::TimedOut, "timeout en el handshake")
        }
        _ => e,
    })
}

/// Busca `"clave":valor` en un JSON plano y devuelve el valor sin comillas.
/// Basta para los campos simples del HELLO (números y cadenas hex sin escapes).
pub fn json_field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\"");
    let mut rest = &json[json.find(&pat)? + pat.len()..];
    rest = rest.trim_start().strip_prefix(':')?.trim_start();
    if let Some(r) = rest.strip_prefix('"') {
        r.find('"').map(|end| &r[..end])
    } else {
        let end = rest
            .find(|c: char| c == ',' || c == '}' || c.is_whitespace())
            .unwrap_or(rest.len());
        Some(&rest[..end])
    }
}

fn phone_error(payload: &[u8]) -> io::Error {
    let body = String::from_utf8_lossy(payload);
    match json_field(&body, "code") {
        Some("bad_code") => PairingError::BadCode.into_io(),
        Some("bad_key") => PairingError::KeyMismatch.into_io(),
        Some("pairing_required") => {
            PairingError::Rejected(json_field(&body, "message").unwrap_or("").into()).into_io()
        }
        _ => io::Error::other(format!("el móvil respondió ERROR: {body}")),
    }
}

fn bad_data(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

fn handshake_inner(client: &mut Client, config_json: &str, p: Pairing) -> io::Result<String> {
    let keys = KeyPair::generate().map_err(|e| io::Error::other(e.to_string()))?;
    let code = p
        .code
        .map(code_psk)
        .transpose()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    let hello = format!(
        r#"{{"proto":1,"platform":"desktop","name":"OpenCam","secure":1,"client_id":"{}","pub":"{}","auth":"{}"}}"#,
        p.store.client_id(),
        to_hex(&keys.public_bytes()),
        if code.is_some() { "code" } else { "key" },
    );
    client.send(MsgType::Hello, 0, hello.as_bytes())?;

    let mut caps = None;
    // El móvil responde HELLO (+ AUTH si es seguro) y CAPS; el resto se ignora mientras tanto.
    for _ in 0..6 {
        let msg = client.recv()?;
        match MsgType::from_code(msg.msg_type) {
            Some(MsgType::Caps) => {
                caps = Some(String::from_utf8_lossy(&msg.payload).into_owned());
                break;
            }
            Some(MsgType::Error) => return Err(phone_error(&msg.payload)),
            Some(MsgType::Hello) if !client.secure => {
                let phone_hello = String::from_utf8_lossy(&msg.payload).into_owned();
                if json_field(&phone_hello, "secure") == Some("1") {
                    authenticate(client, &keys, hello.as_bytes(), &phone_hello, code, p.store)?;
                } else {
                    eprintln!(
                        "Aviso: este móvil no cifra la conexión; actualiza la app de OpenCam."
                    );
                }
            }
            _ => {}
        }
    }
    let caps = caps.ok_or_else(|| bad_data("el móvil no envió CAPS"))?;
    client.send(MsgType::Config, 0, config_json.as_bytes())?;
    Ok(caps)
}

/// Acuerdo de claves y confirmación mutua con AUTH; deja el cliente cifrando.
fn authenticate(
    client: &mut Client,
    keys: &KeyPair,
    desktop_hello: &[u8],
    phone_hello: &str,
    code: Option<[u8; 32]>,
    store: &mut PairingStore,
) -> io::Result<()> {
    let phone_id = json_field(phone_hello, "phone_id")
        .filter(|id| crate::pairing::is_id(id))
        .ok_or_else(|| bad_data("HELLO del móvil sin phone_id"))?
        .to_string();
    let peer = json_field(phone_hello, "pub")
        .and_then(from_hex)
        .ok_or_else(|| bad_data("HELLO del móvil sin clave pública"))?;
    let (psk, with_code) = match (json_field(phone_hello, "auth"), code) {
        (Some("code"), Some(c)) => (c, true),
        (Some("code"), None) => return Err(PairingError::NeedCode.into_io()),
        (Some("key"), _) => match store.key_for(&phone_id) {
            Some(k) => (k, false),
            None => return Err(PairingError::KeyMismatch.into_io()),
        },
        _ => return Err(bad_data("modo de autenticación desconocido")),
    };
    let session = SessionKeys::derive(keys, &peer, &psk, desktop_hello, phone_hello.as_bytes())
        .map_err(|e| bad_data(&e.to_string()))?;
    client.send(MsgType::Auth, 0, &session.auth_tag(Role::Desktop))?;
    let mut authenticated = false;
    for _ in 0..4 {
        let msg = client.recv()?;
        match MsgType::from_code(msg.msg_type) {
            Some(MsgType::Auth) => {
                session
                    .verify_auth(Role::Phone, &msg.payload)
                    .map_err(|_| PairingError::PhoneNotAuthenticated.into_io())?;
                authenticated = true;
                break;
            }
            Some(MsgType::Error) => return Err(phone_error(&msg.payload)),
            _ => {}
        }
    }
    if !authenticated {
        return Err(PairingError::PhoneNotAuthenticated.into_io());
    }
    let (tx, rx) = session.ciphers(Role::Desktop);
    client.enable_encryption(tx, rx);
    if with_code {
        if let Err(e) = store.remember(&phone_id, session.pairing_key) {
            eprintln!("Aviso: no se pudo guardar el emparejamiento: {e}");
        }
    }
    Ok(())
}

/// RTT en microsegundos de un PONG: `ahora − timestamp_us` del PING original.
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
                let h = Header {
                    msg_type: t.code(),
                    flags: 0,
                    seq: 0,
                    timestamp_us: ts,
                    payload_len: p.len() as u32,
                };
                s.write_all(&h.encode()).unwrap();
                s.write_all(p).unwrap();
            };

            let (t, _, _) = read_msg(&mut s);
            assert_eq!(t, MsgType::Hello.code());
            write_msg(
                &mut s,
                MsgType::Hello,
                0,
                br#"{"proto":1,"platform":"android"}"#,
            );
            write_msg(&mut s, MsgType::Caps, 0, br#"{"platform":"android"}"#);

            let (t, _, cfg) = read_msg(&mut s);
            assert_eq!(t, MsgType::Config.code());
            assert!(String::from_utf8_lossy(&cfg).contains("\"codec\":\"h264\""));

            let (t, ts, _) = read_msg(&mut s);
            assert_eq!(t, MsgType::Ping.code());
            write_msg(&mut s, MsgType::Pong, ts, &[]);
        });

        let mut client = Client::connect(addr, Duration::from_secs(2)).unwrap();
        let caps = handshake(&mut client, r#"{"codec":"h264"}"#, Duration::from_secs(2)).unwrap();
        assert!(caps.contains("android"));

        client.send(MsgType::Ping, 0, &[]).unwrap();
        let pong = client.recv().unwrap();
        assert_eq!(pong.msg_type, MsgType::Pong.code());
        assert!(pong.timestamp_us <= client.now_us());
        phone.join().unwrap();
    }

    #[test]
    fn handshake_times_out_with_silent_phone() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let phone = thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_millis(600));
            drop(s);
        });
        let mut client = Client::connect(addr, Duration::from_secs(2)).unwrap();
        let t = Instant::now();
        let err = handshake(&mut client, "{}", Duration::from_millis(200)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(t.elapsed() < Duration::from_millis(550));
        phone.join().unwrap();
    }

    #[test]
    fn unsupported_version_gets_error_and_close() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let phone = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut raw = [0u8; HEADER_LEN];
            s.read_exact(&mut raw).unwrap(); // HELLO
            let hello_len = Header::decode(&raw).unwrap().payload_len as usize;
            s.read_exact(&mut vec![0u8; hello_len]).unwrap();
            let mut bad = Header {
                msg_type: MsgType::Hello.code(),
                flags: 0,
                seq: 0,
                timestamp_us: 0,
                payload_len: 0,
            }
            .encode();
            bad[4] = 2;
            s.write_all(&bad).unwrap();
            s.read_exact(&mut raw).unwrap();
            let h = Header::decode(&raw).unwrap();
            assert_eq!(h.msg_type, MsgType::Error.code());
            let mut p = vec![0u8; h.payload_len as usize];
            s.read_exact(&mut p).unwrap();
            assert!(String::from_utf8_lossy(&p).contains("unsupported_version"));
            // El escritorio cierra: la siguiente lectura ve EOF.
            assert_eq!(s.read(&mut raw).unwrap(), 0);
        });
        let mut client = Client::connect(addr, Duration::from_secs(2)).unwrap();
        let err = handshake(&mut client, "{}", Duration::from_secs(2)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        phone.join().unwrap();
    }

    // --- Emparejamiento y cifrado contra un móvil falso que implementa el lado del teléfono ---

    use opencam_core::secure::{Cipher as PhoneCipher, SessionKeys as Keys};

    fn raw_read(s: &mut TcpStream) -> ([u8; HEADER_LEN], Vec<u8>) {
        let mut raw = [0u8; HEADER_LEN];
        s.read_exact(&mut raw).unwrap();
        let mut p = vec![0u8; Header::decode(&raw).unwrap().payload_len as usize];
        s.read_exact(&mut p).unwrap();
        (raw, p)
    }

    fn raw_write(s: &mut TcpStream, t: MsgType, p: &[u8], c: Option<&mut PhoneCipher>) {
        let h = Header {
            msg_type: t.code(),
            flags: 0,
            seq: 0,
            timestamp_us: 5,
            payload_len: p.len() as u32,
        };
        match c {
            Some(c) => {
                let (h, ct) = c.seal(h, p).unwrap();
                s.write_all(&h).unwrap();
                s.write_all(&ct).unwrap();
            }
            None => {
                s.write_all(&h.encode()).unwrap();
                s.write_all(p).unwrap();
            }
        }
    }

    /// Qué hace el móvil falso con el AUTH del escritorio.
    #[derive(Clone, Copy)]
    enum Phone {
        /// Comprueba el código/clave y responde como un móvil real.
        Honest,
        /// Responde con un AUTH inválido (no conoce el código).
        Impostor,
    }

    /// Un móvil con código `code` y, si `known_key`, clave guardada para este escritorio.
    /// Devuelve la clave de emparejamiento acordada (si la hubo).
    fn fake_secure_phone(
        listener: TcpListener,
        code: &'static str,
        known_key: Option<[u8; 32]>,
        mode: Phone,
    ) -> thread::JoinHandle<Option<[u8; 32]>> {
        thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let (_, hello) = raw_read(&mut s);
            let hello_s = String::from_utf8(hello.clone()).unwrap();
            assert_eq!(json_field(&hello_s, "secure"), Some("1"));
            let wants_key = json_field(&hello_s, "auth") == Some("key");
            let use_key = wants_key && known_key.is_some();
            let kp = KeyPair::generate().unwrap();
            let phone_hello = format!(
                r#"{{"proto":1,"platform":"android","secure":1,"phone_id":"a1b2c3d4","pub":"{}","auth":"{}"}}"#,
                to_hex(&kp.public_bytes()),
                if use_key { "key" } else { "code" }
            );
            raw_write(&mut s, MsgType::Hello, phone_hello.as_bytes(), None);
            let psk = if use_key {
                known_key.unwrap()
            } else {
                code_psk(code).unwrap()
            };
            let peer = from_hex(json_field(&hello_s, "pub").unwrap()).unwrap();
            let keys = Keys::derive(&kp, &peer, &psk, &hello, phone_hello.as_bytes()).unwrap();
            let (t, tag) = raw_read(&mut s);
            assert_eq!(t[5], MsgType::Auth.code());
            if keys.verify_auth(Role::Desktop, &tag).is_err() {
                let code = if use_key { "bad_key" } else { "bad_code" };
                let body = format!(r#"{{"code":"{code}","message":"no"}}"#);
                raw_write(&mut s, MsgType::Error, body.as_bytes(), None);
                return None;
            }
            match mode {
                Phone::Impostor => {
                    raw_write(&mut s, MsgType::Auth, &[0u8; 32], None);
                    return None;
                }
                Phone::Honest => {
                    raw_write(&mut s, MsgType::Auth, &keys.auth_tag(Role::Phone), None)
                }
            }
            let (mut tx, mut rx) = keys.ciphers(Role::Phone);
            raw_write(
                &mut s,
                MsgType::Caps,
                br#"{"platform":"android"}"#,
                Some(&mut tx),
            );
            let (h, ct) = raw_read(&mut s);
            assert_eq!(h[5], MsgType::Config.code());
            let cfg = rx.open(&h, &ct).unwrap();
            assert_eq!(cfg, br#"{"codec":"h264"}"#);
            // En el cable la CONFIG no va en claro.
            assert!(!ct.windows(5).any(|w| w == b"codec"));
            let (h, ct) = raw_read(&mut s);
            assert_eq!(h[5], MsgType::Ping.code());
            rx.open(&h, &ct).unwrap();
            raw_write(
                &mut s,
                MsgType::Video,
                b"\x00\x00\x00\x01frame",
                Some(&mut tx),
            );
            Some(keys.pairing_key)
        })
    }

    fn connect_with(
        addr: std::net::SocketAddr,
        store: &mut PairingStore,
        code: Option<&str>,
    ) -> io::Result<(Client, String)> {
        let mut c = Client::connect(addr, Duration::from_secs(2))?;
        let caps = handshake_with(
            &mut c,
            r#"{"codec":"h264"}"#,
            Duration::from_secs(2),
            Pairing { store, code },
        )?;
        Ok((c, caps))
    }

    #[test]
    fn pairs_with_code_then_reconnects_with_saved_key() {
        let mut store = PairingStore::in_memory();

        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let phone = fake_secure_phone(l, "123456", None, Phone::Honest);
        let (mut c, caps) = connect_with(addr, &mut store, Some("123-456")).unwrap();
        assert!(c.is_secure());
        assert!(caps.contains("android"));
        c.send(MsgType::Ping, 0, &[]).unwrap();
        let v = c.recv().unwrap();
        assert_eq!(v.msg_type, MsgType::Video.code());
        assert_eq!(v.payload, b"\x00\x00\x00\x01frame");
        let key = phone.join().unwrap().unwrap();
        assert_eq!(store.key_for("a1b2c3d4"), Some(key));

        // Segunda vez: sin código, con la clave guardada.
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let phone = fake_secure_phone(l, "999999", Some(key), Phone::Honest);
        let (mut c, _) = connect_with(addr, &mut store, None).unwrap();
        assert!(c.is_secure());
        c.send(MsgType::Ping, 0, &[]).unwrap();
        assert_eq!(c.recv().unwrap().msg_type, MsgType::Video.code());
        phone.join().unwrap().unwrap();
    }

    fn pairing_err(r: io::Result<(Client, String)>) -> PairingError {
        let e = r.err().expect("el handshake debía fallar");
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
        PairingError::from_io(&e).cloned().unwrap()
    }

    #[test]
    fn asks_for_code_and_reports_wrong_code() {
        let mut store = PairingStore::in_memory();
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let phone = thread::spawn(move || {
            // Solo responde HELLO: el escritorio corta antes de AUTH por falta de código.
            let (mut s, _) = l.accept().unwrap();
            raw_read(&mut s);
            let kp = KeyPair::generate().unwrap();
            let h = format!(
                r#"{{"secure":1,"phone_id":"a1b2c3d4","pub":"{}","auth":"code"}}"#,
                to_hex(&kp.public_bytes())
            );
            raw_write(&mut s, MsgType::Hello, h.as_bytes(), None);
        });
        assert_eq!(
            pairing_err(connect_with(addr, &mut store, None)),
            PairingError::NeedCode
        );
        phone.join().unwrap();

        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let phone = fake_secure_phone(l, "123456", None, Phone::Honest);
        assert_eq!(
            pairing_err(connect_with(addr, &mut store, Some("000000"))),
            PairingError::BadCode
        );
        assert!(phone.join().unwrap().is_none());
        assert_eq!(store.key_for("a1b2c3d4"), None);
    }

    #[test]
    fn rejects_phone_that_does_not_know_the_code() {
        let mut store = PairingStore::in_memory();
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let phone = fake_secure_phone(l, "123456", None, Phone::Impostor);
        assert_eq!(
            pairing_err(connect_with(addr, &mut store, Some("123456"))),
            PairingError::PhoneNotAuthenticated
        );
        phone.join().unwrap();
        assert_eq!(store.key_for("a1b2c3d4"), None);
    }

    #[test]
    fn oversized_payload_before_handshake_is_rejected() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let phone = thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            raw_read(&mut s);
            let h = Header {
                msg_type: MsgType::Hello.code(),
                flags: 0,
                seq: 0,
                timestamp_us: 0,
                payload_len: MAX_UNAUTH_PAYLOAD + 1,
            };
            s.write_all(&h.encode()).unwrap();
            thread::sleep(Duration::from_millis(200));
        });
        let mut c = Client::connect(addr, Duration::from_secs(2)).unwrap();
        let e = handshake(&mut c, "{}", Duration::from_secs(2)).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
        phone.join().unwrap();
    }

    #[test]
    fn json_field_extracts_simple_values() {
        let j = r#"{"secure": 1,"pub":"0a0b", "auth" : "code","n":12}"#;
        assert_eq!(json_field(j, "secure"), Some("1"));
        assert_eq!(json_field(j, "pub"), Some("0a0b"));
        assert_eq!(json_field(j, "auth"), Some("code"));
        assert_eq!(json_field(j, "n"), Some("12"));
        assert_eq!(json_field(j, "nope"), None);
        assert_eq!(json_field(r#"{"x":"#, "x"), Some(""));
    }
}
