//! Cliente de usbmuxd (macOS y Linux): localiza un iPhone por USB y abre un túnel TCP hasta el
//! puerto OPCM de la app. Protocolo: cabecera de 16 bytes little-endian + cuerpo plist XML.
//! Tras un `Connect` correcto, el mismo socket Unix pasa a transportar el stream del iPhone.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;

use plist::{Dictionary, Value};

const DEFAULT_SOCKET: &str = "/var/run/usbmuxd";
const MESSAGE_PLIST: u32 = 8;
const HEADER_LEN: usize = 16;

/// Ruta del socket: `USBMUXD_SOCKET_ADDRESS` (`/ruta` o `UNIX:/ruta`) o `/var/run/usbmuxd`.
/// shortcut: solo sockets Unix; `HOST:PUERTO` (usbmuxd por TCP) da error claro, soportarlo si hace falta.
fn socket_path() -> io::Result<String> {
    match std::env::var("USBMUXD_SOCKET_ADDRESS") {
        Ok(v) if !v.is_empty() => match v.strip_prefix("UNIX:") {
            Some(p) => Ok(p.to_string()),
            None if v.starts_with('/') => Ok(v),
            None => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("USBMUXD_SOCKET_ADDRESS={v}: solo se admiten sockets Unix (/ruta o UNIX:/ruta)"),
            )),
        },
        _ => Ok(DEFAULT_SOCKET.to_string()),
    }
}

fn open(path: &str) -> io::Result<UnixStream> {
    UnixStream::connect(path)
        .map_err(|e| io::Error::new(e.kind(), format!("no se pudo abrir usbmuxd en {path}: {e} (¿está en marcha?)")))
}

/// Dispositivo USB visto por usbmuxd.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub id: u64,
    /// UDID / número de serie (`Properties.SerialNumber`).
    pub udid: String,
}

/// Socket conectado al puerto `device_port` del iPhone USB con ese UDID/serie (o el primero).
pub fn connect_iphone_with(device_port: u16, udid: Option<&str>) -> io::Result<UnixStream> {
    connect_at(&socket_path()?, device_port, udid)
}

/// Lista los dispositivos iOS conectados por USB.
pub fn devices() -> io::Result<Vec<Device>> {
    list_devices(&socket_path()?)
}

fn connect_at(path: &str, device_port: u16, udid: Option<&str>) -> io::Result<UnixStream> {
    let devices = list_devices(path)?;
    let dev = match udid {
        Some(u) => devices.iter().find(|d| d.udid == u).ok_or_else(|| {
            let have: Vec<_> = devices.iter().map(|d| d.udid.as_str()).collect();
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no hay ningún iPhone con UDID {u} (conectados: {have:?})"),
            )
        })?,
        None => devices
            .first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no hay ningún iPhone conectado por USB"))?,
    };

    let mut stream = open(path)?;
    let mut req = Dictionary::new();
    req.insert("MessageType".into(), Value::String("Connect".into()));
    req.insert("ClientVersionString".into(), Value::String("opencam-desktop".into()));
    req.insert("ProgName".into(), Value::String("opencam-desktop".into()));
    req.insert("DeviceID".into(), Value::Integer(dev.id.into()));
    // usbmuxd espera el puerto en orden de red (big-endian) dentro de un entero.
    req.insert("PortNumber".into(), Value::Integer(device_port.to_be().into()));
    let reply = exchange(&mut stream, &req)?;
    match reply.get("Number").and_then(Value::as_unsigned_integer) {
        Some(0) => Ok(stream),
        Some(code) => {
            let why = match code {
                2 => "dispositivo no encontrado",
                3 => "conexión rechazada: ¿la app OpenCam está abierta y escuchando?",
                5 => "petición no válida",
                _ => "error desconocido",
            };
            Err(io::Error::other(format!("usbmuxd Connect falló (código {code}): {why}")))
        }
        None => Err(io::Error::new(io::ErrorKind::InvalidData, "respuesta de usbmuxd no válida")),
    }
}

fn list_devices(path: &str) -> io::Result<Vec<Device>> {
    let mut stream = open(path)?;
    let mut req = Dictionary::new();
    req.insert("MessageType".into(), Value::String("ListDevices".into()));
    req.insert("ClientVersionString".into(), Value::String("opencam-desktop".into()));
    req.insert("ProgName".into(), Value::String("opencam-desktop".into()));
    let reply = exchange(&mut stream, &req)?;
    let list = reply
        .get("DeviceList")
        .and_then(Value::as_array)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "falta DeviceList"))?;
    Ok(list
        .iter()
        .filter_map(|d| {
            let d = d.as_dictionary()?;
            let udid = d
                .get("Properties")
                .and_then(Value::as_dictionary)
                .and_then(|p| p.get("SerialNumber"))
                .and_then(Value::as_string)
                .unwrap_or_default()
                .to_string();
            Some(Device { id: d.get("DeviceID")?.as_unsigned_integer()?, udid })
        })
        .collect())
}

/// Envía un mensaje plist y lee la respuesta.
fn exchange(stream: &mut UnixStream, body: &Dictionary) -> io::Result<Dictionary> {
    let mut xml = Vec::new();
    plist::to_writer_xml(&mut xml, body).map_err(|e| io::Error::other(e.to_string()))?;

    let total = (HEADER_LEN + xml.len()) as u32;
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(&total.to_le_bytes());
    header.extend_from_slice(&1u32.to_le_bytes()); // versión 1
    header.extend_from_slice(&MESSAGE_PLIST.to_le_bytes());
    header.extend_from_slice(&1u32.to_le_bytes()); // tag
    stream.write_all(&header)?;
    stream.write_all(&xml)?;

    let mut raw = [0u8; HEADER_LEN];
    stream.read_exact(&mut raw)?;
    let len = u32::from_le_bytes(raw[0..4].try_into().unwrap()) as usize;
    if !(HEADER_LEN..=HEADER_LEN + 1024 * 1024).contains(&len) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "longitud de respuesta fuera de rango"));
    }
    let mut payload = vec![0u8; len - HEADER_LEN];
    stream.read_exact(&mut payload)?;
    let value: Value =
        plist::from_bytes(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    value
        .into_dictionary()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "respuesta plist no es un diccionario"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    fn read_msg(s: &mut UnixStream) -> Dictionary {
        let mut h = [0u8; HEADER_LEN];
        s.read_exact(&mut h).unwrap();
        let len = u32::from_le_bytes(h[0..4].try_into().unwrap()) as usize;
        let mut b = vec![0u8; len - HEADER_LEN];
        s.read_exact(&mut b).unwrap();
        plist::from_bytes::<Value>(&b).unwrap().into_dictionary().unwrap()
    }

    fn reply(s: &mut UnixStream, d: Dictionary) {
        let mut xml = Vec::new();
        plist::to_writer_xml(&mut xml, &d).unwrap();
        let mut m = ((HEADER_LEN + xml.len()) as u32).to_le_bytes().to_vec();
        m.extend_from_slice(&1u32.to_le_bytes());
        m.extend_from_slice(&MESSAGE_PLIST.to_le_bytes());
        m.extend_from_slice(&1u32.to_le_bytes());
        m.extend(xml);
        s.write_all(&m).unwrap();
    }

    fn device(id: u64, udid: &str) -> Value {
        let mut p = Dictionary::new();
        p.insert("SerialNumber".into(), Value::String(udid.into()));
        let mut d = Dictionary::new();
        d.insert("DeviceID".into(), Value::Integer(id.into()));
        d.insert("Properties".into(), Value::Dictionary(p));
        Value::Dictionary(d)
    }

    /// Falso usbmuxd: dos dispositivos; `Connect` responde `connect_code`. Devuelve el DeviceID/puerto recibidos.
    fn fake(name: &str, connect_code: u64) -> (String, std::thread::JoinHandle<Vec<(u64, u64)>>) {
        let path = std::env::temp_dir().join(format!("opencam-mux-{name}-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let l = UnixListener::bind(&path).unwrap();
        let h = std::thread::spawn(move || {
            let mut seen = vec![];
            for _ in 0..2 {
                let (mut s, _) = l.accept().unwrap();
                let req = read_msg(&mut s);
                let mut r = Dictionary::new();
                match req.get("MessageType").and_then(Value::as_string) {
                    Some("ListDevices") => {
                        r.insert("DeviceList".into(), Value::Array(vec![device(1, "AAA"), device(2, "BBB")]));
                    }
                    _ => {
                        let n = |k| req.get(k).and_then(Value::as_unsigned_integer).unwrap();
                        seen.push((n("DeviceID"), n("PortNumber")));
                        r.insert("Number".into(), Value::Integer(connect_code.into()));
                    }
                }
                reply(&mut s, r);
            }
            seen
        });
        (path.to_string_lossy().into(), h)
    }

    #[test]
    fn list_and_connect_by_udid() {
        let (path, h) = fake("ok", 0);
        assert!(connect_at(&path, 4747, Some("BBB")).is_ok());
        assert_eq!(h.join().unwrap(), vec![(2, 4747u16.to_be() as u64)]);
    }

    #[test]
    fn connect_error_code() {
        let (path, h) = fake("err", 3);
        let e = connect_at(&path, 4747, None).unwrap_err().to_string();
        assert!(e.contains("código 3"), "{e}");
        assert_eq!(h.join().unwrap()[0].0, 1); // sin UDID: primer dispositivo
    }

    #[test]
    fn unknown_udid_and_listing() {
        let (path, h) = fake("udid", 0);
        let e = connect_at(&path, 4747, Some("ZZZ")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
        assert!(e.to_string().contains("AAA"));
        let _ = UnixStream::connect(&path).map(drop); // libera el accept restante
        let _ = h.join();
    }

    #[test]
    fn missing_socket_is_clear() {
        let e = connect_at("/nonexistent/mux", 4747, None).unwrap_err().to_string();
        assert!(e.contains("/nonexistent/mux"), "{e}");
    }
}
