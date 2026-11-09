//! Elige por dónde llega el móvil. Por defecto (`auto`) prueba, en orden:
//! 1. Android por USB: `adb forward` automático (ver `adb.rs`).
//! 2. iPhone por USB: túnel de usbmuxd (macOS/Linux).
//! 3. Wi-Fi: el primer móvil que se anuncie por mDNS (`_opencam._tcp`).
//!
//! Cada candidato se da por bueno solo si completa el handshake: un `adb forward` acepta la
//! conexión TCP aunque la app del móvil no esté escuchando, así que conectar no basta.

use std::io;
use std::time::Duration;

use opencam_core::protocol::DEFAULT_PORT;

use crate::adb;
use crate::cli::Cli;
use crate::client::{handshake_with, Client, Pairing, PairingError};
use crate::discovery;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MDNS_TIMEOUT: Duration = Duration::from_secs(3);

/// Un camino de conexión: devuelve el cliente aún sin negociar y la etiqueta del camino.
type Attempt = fn(&Cli) -> io::Result<(Client, String)>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// USB (Android, luego iPhone) y, si no hay, Wi-Fi por mDNS.
    Auto,
    /// Android por USB con `adb forward`.
    Usb,
    /// Primer móvil anunciado por mDNS en la red local.
    Wifi,
    /// iPhone por USB (usbmuxd).
    Iphone,
    /// `host:puerto` explícito.
    Addr(String),
}

impl Target {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "" | "auto" => Self::Auto,
            "usb" | "android" | "adb" => Self::Usb,
            "wifi" | "wi-fi" | "mdns" => Self::Wifi,
            "iphone" | "ios" => Self::Iphone,
            _ => Self::Addr(s.to_string()),
        }
    }
}

/// Conexión ya negociada: el cliente, el JSON de CAPS y una etiqueta legible del camino usado.
pub struct Connected {
    pub client: Client,
    pub caps: String,
    pub via: String,
}

/// Conecta según `cli.addr` y completa el handshake (con emparejamiento si el móvil lo pide).
/// En `auto`, un error de emparejamiento corta la búsqueda: es el móvil correcto y hace falta
/// el código, probar otro camino no lo arregla.
pub fn open(
    cli: &Cli,
    config: &str,
    handshake_timeout: Duration,
    pairing: Pairing,
) -> io::Result<Connected> {
    let Pairing { store, code } = pairing;
    let mut negotiate = |mut client: Client, via: String| -> io::Result<Connected> {
        let p = Pairing {
            store: &mut *store,
            code,
        };
        let caps = handshake_with(&mut client, config, handshake_timeout, p)?;
        Ok(Connected { client, caps, via })
    };
    match Target::parse(&cli.addr) {
        Target::Addr(a) => negotiate(Client::connect(a.as_str(), CONNECT_TIMEOUT)?, a),
        Target::Usb => usb_android(cli).and_then(|(c, v)| negotiate(c, v)),
        Target::Iphone => iphone(cli).and_then(|(c, v)| negotiate(c, v)),
        Target::Wifi => wifi().and_then(|(c, v)| negotiate(c, v)),
        Target::Auto => {
            let mut errors = Vec::new();
            let tries: [(&str, Attempt); 3] = [
                ("USB Android", usb_android),
                ("USB iPhone", iphone),
                ("Wi-Fi", |_| wifi()),
            ];
            for (name, attempt) in tries {
                match attempt(cli).and_then(|(c, v)| negotiate(c, v)) {
                    Ok(c) => return Ok(c),
                    Err(e) if PairingError::from_io(&e).is_some() => return Err(e),
                    Err(e) => errors.push(format!("{name}: {e}")),
                }
            }
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "no se encontró ningún móvil con OpenCam abierto [{}]",
                    errors.join("; ")
                ),
            ))
        }
    }
}

fn usb_android(cli: &Cli) -> io::Result<(Client, String)> {
    let adb_path = adb::find(cli.adb.as_deref()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no se encontró adb (instala Android platform-tools o usa --adb <ruta>)",
        )
    })?;
    let dev = adb::pick(&adb::devices(&adb_path)?, cli.udid.as_deref())?;
    let port = adb::forward(&adb_path, &dev.serial, DEFAULT_PORT)?;
    let client = Client::connect(("127.0.0.1", port), CONNECT_TIMEOUT)?;
    Ok((
        client,
        format!("USB Android {} (127.0.0.1:{port})", dev.serial),
    ))
}

#[cfg(unix)]
fn iphone(cli: &Cli) -> io::Result<(Client, String)> {
    let stream = crate::usbmuxd::connect_iphone_with(DEFAULT_PORT, cli.udid.as_deref())?;
    Ok((Client::from_stream(Box::new(stream))?, "USB iPhone".into()))
}

#[cfg(not(unix))]
fn iphone(_: &Cli) -> io::Result<(Client, String)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "el iPhone por USB solo está disponible en macOS y Linux",
    ))
}

fn wifi() -> io::Result<(Client, String)> {
    let f = discovery::browse_first(MDNS_TIMEOUT)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "ningún móvil anunciado por mDNS (¿misma red Wi-Fi?)",
        )
    })?;
    let client = Client::connect(f.addr.as_str(), CONNECT_TIMEOUT)?;
    Ok((client, format!("Wi-Fi {} ({})", f.name, f.addr)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_targets() {
        assert_eq!(Target::parse("auto"), Target::Auto);
        assert_eq!(Target::parse("USB"), Target::Usb);
        assert_eq!(Target::parse("android"), Target::Usb);
        assert_eq!(Target::parse("wifi"), Target::Wifi);
        assert_eq!(Target::parse("iphone"), Target::Iphone);
        assert_eq!(
            Target::parse("192.168.1.20:4747"),
            Target::Addr("192.168.1.20:4747".into())
        );
    }

    /// `usb` de punta a punta con un `adb` falso (script) y un móvil falso que negocia.
    #[cfg(unix)]
    #[test]
    fn usb_uses_adb_forward_and_handshakes() {
        use opencam_core::protocol::{Header, MsgType, HEADER_LEN};
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};
        use std::os::unix::fs::PermissionsExt;

        fn read_type(s: &mut TcpStream) -> u8 {
            let mut raw = [0u8; HEADER_LEN];
            s.read_exact(&mut raw).unwrap();
            let h = Header::decode(&raw).unwrap();
            s.read_exact(&mut vec![0u8; h.payload_len as usize])
                .unwrap();
            h.msg_type
        }
        fn write_type(s: &mut TcpStream, t: MsgType, p: &[u8]) {
            let h = Header {
                msg_type: t.code(),
                flags: 0,
                seq: 0,
                timestamp_us: 0,
                payload_len: p.len() as u32,
            };
            s.write_all(&h.encode()).unwrap();
            s.write_all(p).unwrap();
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let dir = std::env::temp_dir().join(format!("opencam-fake-adb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("adb");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\n\
                 case \"$*\" in\n\
                 devices) printf 'List of devices attached\\nR58M\\tdevice\\n' ;;\n\
                 *'forward --list') ;;\n\
                 *'forward tcp:0 tcp:4747') echo {port} ;;\n\
                 *) exit 1 ;;\n\
                 esac\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let phone = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            assert_eq!(read_type(&mut s), MsgType::Hello.code());
            write_type(&mut s, MsgType::Hello, b"{}");
            write_type(&mut s, MsgType::Caps, br#"{"codecs":["h264"]}"#);
            assert_eq!(read_type(&mut s), MsgType::Config.code());
        });
        let cli = Cli {
            addr: "usb".into(),
            adb: Some(script.to_string_lossy().into_owned()),
            ..Cli::default()
        };
        let mut store = crate::pairing::PairingStore::in_memory();
        let pairing = Pairing {
            store: &mut store,
            code: None,
        };
        let c = open(&cli, "{}", Duration::from_secs(5), pairing).unwrap();
        assert!(
            c.via.contains("R58M") && c.via.contains(&port.to_string()),
            "{}",
            c.via
        );
        assert!(c.caps.contains("h264"));
        phone.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
