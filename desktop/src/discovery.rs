//! Descubrimiento mDNS de teléfonos OpenCam (`_opencam._tcp`, puerto 4747).
//! TXT: `platform=android|ios`, `name=<dispositivo>`.

use std::io;
use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent};

pub const SERVICE_TYPE: &str = "_opencam._tcp.local.";

#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub name: String,
    /// `ip:puerto` listo para `TcpStream::connect`.
    pub addr: String,
    /// `android`, `ios` o `unknown` si falta el TXT.
    pub platform: String,
}

fn err(e: mdns_sd::Error) -> io::Error {
    io::Error::other(format!("mDNS: {e}"))
}

/// Busca teléfonos durante `timeout` y devuelve los resueltos (sin duplicados por dirección).
pub fn browse(timeout: Duration) -> io::Result<Vec<Found>> {
    browse_until(timeout, false)
}

/// Como [`browse`], pero vuelve en cuanto resuelve el primer teléfono.
pub fn browse_first(timeout: Duration) -> io::Result<Option<Found>> {
    Ok(browse_until(timeout, true)?.into_iter().next())
}

fn browse_until(timeout: Duration, first_only: bool) -> io::Result<Vec<Found>> {
    let daemon = ServiceDaemon::new().map_err(err)?;
    let rx = daemon.browse(SERVICE_TYPE).map_err(err)?;
    let end = Instant::now() + timeout;
    let mut out: Vec<Found> = Vec::new();
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        let Ok(ev) = rx.recv_timeout(left) else { break };
        let ServiceEvent::ServiceResolved(s) = ev else {
            continue;
        };
        let mut ips: Vec<_> = s.get_addresses_v4().into_iter().collect(); // shortcut: solo IPv4; añadir v6 si hace falta
        ips.sort();
        let Some(ip) = ips.first() else { continue };
        let f = Found {
            name: s
                .get_property_val_str("name")
                .unwrap_or_else(|| s.get_fullname())
                .to_string(),
            addr: format!("{ip}:{}", s.get_port()),
            platform: s
                .get_property_val_str("platform")
                .unwrap_or("unknown")
                .to_string(),
        };
        if !out.iter().any(|o| o.addr == f.addr) {
            out.push(f);
            if first_only {
                break;
            }
        }
    }
    let _ = daemon.shutdown();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdns_sd::ServiceInfo;

    /// Anuncia y descubre en loopback; necesita multicast (ignorado por defecto: `cargo test -- --ignored`).
    #[test]
    #[ignore = "requiere multicast mDNS"]
    fn announce_and_discover() {
        let d = ServiceDaemon::new().unwrap();
        let props = [("platform", "android"), ("name", "Pixel Test")];
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            "opencam-test",
            "opencam-test.local.",
            "127.0.0.1",
            4747,
            &props[..],
        )
        .unwrap();
        d.register(info).unwrap();
        let found = browse(Duration::from_secs(5)).unwrap();
        let _ = d.shutdown();
        let f = found
            .iter()
            .find(|f| f.name == "Pixel Test")
            .unwrap_or_else(|| panic!("{found:?}"));
        assert_eq!(
            (f.addr.as_str(), f.platform.as_str()),
            ("127.0.0.1:4747", "android")
        );
    }
}
