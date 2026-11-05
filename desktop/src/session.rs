//! Bucle de sesión: conecta, negocia y sirve; si se pierde la conexión, reconecta con backoff.
// shortcut: backoff local; pasará al de core::reconnect cuando exista.

use std::io;
use std::thread::sleep;
use std::time::Duration;

use crate::cli::Cli;
use crate::client::{handshake, Client};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const BACKOFF_MIN: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(5);

/// Cómo termina una conexión servida.
pub enum End {
    /// Fin normal: no se reconecta (solo lo usan los tests; en producción la sesión no termina).
    #[cfg_attr(not(test), allow(dead_code))]
    Done,
    /// Conexión perdida: se reconecta salvo `--no-reconnect`.
    Lost(io::Error),
    /// Error local (p. ej. la cámara virtual): reconectar no lo arregla.
    Fatal(io::Error),
}

fn connect(cli: &Cli) -> io::Result<Client> {
    if cli.addr == "iphone" {
        #[cfg(unix)]
        return Client::from_stream(Box::new(crate::usbmuxd::connect_iphone(4747)?));
        #[cfg(not(unix))]
        return Err(io::Error::other("`iphone` solo está disponible en macOS y Linux"));
    }
    Client::connect(cli.addr.as_str(), CONNECT_TIMEOUT)
}

/// Repite conectar + handshake + `serve` hasta un fin normal o fatal (o un único intento con
/// `--no-reconnect`). El backoff se reinicia tras cada handshake correcto.
pub fn run(cli: &Cli, mut serve: impl FnMut(&mut Client) -> End) -> io::Result<()> {
    let config = cli.config_json();
    let mut delay = BACKOFF_MIN;
    loop {
        let attempt = connect(cli).and_then(|mut c| {
            let caps = handshake(&mut c, &config, HANDSHAKE_TIMEOUT)?;
            Ok((c, caps))
        });
        let err = match attempt {
            Ok((mut client, caps)) => {
                delay = BACKOFF_MIN;
                println!("Conectado a {}. CAPS: {caps}", cli.addr);
                client.spawn_pinger();
                let end = serve(&mut client);
                client.close();
                match end {
                    End::Done => return Ok(()),
                    End::Fatal(e) => return Err(e),
                    End::Lost(e) => e,
                }
            }
            Err(e) => e,
        };
        if !cli.reconnect {
            return Err(err);
        }
        eprintln!("Conexión perdida ({err}); reintentando en {} ms", delay.as_millis());
        sleep(delay);
        delay = (delay * 2).min(BACKOFF_MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencam_core::protocol::{Header, MsgType, HEADER_LEN};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    fn read_type(s: &mut TcpStream) -> u8 {
        let mut raw = [0u8; HEADER_LEN];
        s.read_exact(&mut raw).unwrap();
        let h = Header::decode(&raw).unwrap();
        s.read_exact(&mut vec![0u8; h.payload_len as usize]).unwrap();
        h.msg_type
    }

    fn write_type(s: &mut TcpStream, t: MsgType, p: &[u8]) {
        let h = Header { msg_type: t.code(), flags: 0, seq: 0, timestamp_us: 0, payload_len: p.len() as u32 };
        s.write_all(&h.encode()).unwrap();
        s.write_all(p).unwrap();
    }

    /// Móvil falso: dos conexiones con handshake completo; la primera se cierra a mitad de sesión.
    #[test]
    fn reconnects_and_repeats_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let cli = Cli { addr: listener.local_addr().unwrap().to_string(), ..Cli::default() };
        let phone = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut s, _) = listener.accept().unwrap();
                assert_eq!(read_type(&mut s), MsgType::Hello.code());
                write_type(&mut s, MsgType::Hello, b"{}");
                write_type(&mut s, MsgType::Caps, b"{}");
                assert_eq!(read_type(&mut s), MsgType::Config.code());
                // al terminar la iteración se cierra el socket
            }
        });
        let mut served = 0;
        run(&cli, |c| {
            served += 1;
            loop {
                match c.recv() {
                    Err(e) if served == 1 => return End::Lost(e),
                    Err(_) => return End::Done,
                    Ok(_) => {}
                }
            }
        })
        .unwrap();
        assert_eq!(served, 2);
        phone.join().unwrap();
    }

    #[test]
    fn no_reconnect_fails_once() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let cli = Cli { addr: listener.local_addr().unwrap().to_string(), reconnect: false, ..Cli::default() };
        drop(listener); // nadie escucha
        assert!(run(&cli, |_| End::Done).is_err());
    }
}
