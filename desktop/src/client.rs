//! Cliente OPCM del escritorio: se conecta al móvil (puerto 4747), negocia y recibe vídeo.
//!
//! Flujo: HELLO → CAPS → CONFIG → VIDEO continuo, con PING cada segundo para medir el RTT.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use opencam_core::protocol::{Header, MsgType, ProtoError, HEADER_LEN};

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
        w.stream.write_all(&header.encode())?;
        w.stream.write_all(payload)?;
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
        };
        Ok(Self {
            reader: stream,
            tx: Sender(Arc::new(Mutex::new(writer))),
            epoch,
        })
    }

    /// Microsegundos desde el arranque del cliente; son el `timestamp_us` de los PING.
    pub fn now_us(&self) -> u64 {
        self.epoch.elapsed().as_micros() as u64
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
        let mut payload = vec![0u8; header.payload_len as usize];
        self.reader.read_exact(&mut payload)?;
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

/// Handshake completo: HELLO del escritorio, espera CAPS y envía CONFIG.
/// Un móvil mudo no bloquea: cada lectura caduca a los `timeout`.
pub fn handshake(client: &mut Client, config_json: &str, timeout: Duration) -> io::Result<String> {
    client.reader.set_read_timeout(Some(timeout))?;
    let result = handshake_inner(client, config_json);
    let _ = client.reader.set_read_timeout(None);
    result.map_err(|e| match e.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {
            io::Error::new(io::ErrorKind::TimedOut, "timeout en el handshake")
        }
        _ => e,
    })
}

fn handshake_inner(client: &mut Client, config_json: &str) -> io::Result<String> {
    client.send(
        MsgType::Hello,
        0,
        br#"{"proto":1,"platform":"desktop","name":"OpenCam"}"#,
    )?;
    let mut caps = None;
    // El móvil responde HELLO y CAPS; se ignoran el resto de tipos mientras tanto.
    for _ in 0..4 {
        let msg = client.recv()?;
        if msg.msg_type == MsgType::Caps.code() {
            caps = Some(String::from_utf8_lossy(&msg.payload).into_owned());
            break;
        }
        if msg.msg_type == MsgType::Error.code() {
            let body = String::from_utf8_lossy(&msg.payload);
            return Err(io::Error::other(format!(
                "el móvil respondió ERROR: {body}"
            )));
        }
    }
    let caps =
        caps.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "el móvil no envió CAPS"))?;
    client.send(MsgType::Config, 0, config_json.as_bytes())?;
    Ok(caps)
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
}
