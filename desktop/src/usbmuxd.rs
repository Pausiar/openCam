//! Cliente de usbmuxd (macOS y Linux): localiza un iPhone por USB y abre un túnel TCP hasta el
//! puerto OPCM de la app. Protocolo: cabecera de 16 bytes little-endian + cuerpo plist XML.
//! Tras un `Connect` correcto, el mismo socket Unix pasa a transportar el stream del iPhone.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;

use plist::{Dictionary, Value};

const USBMUXD_SOCKET: &str = "/var/run/usbmuxd";

const MESSAGE_PLIST: u32 = 8;
const HEADER_LEN: usize = 16;

/// Devuelve un socket conectado al puerto `device_port` del primer iPhone USB encontrado.
pub fn connect_iphone(device_port: u16) -> io::Result<UnixStream> {
    let device_id = *list_devices()?
        .first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no hay ningún iPhone conectado por USB"))?;

    let mut stream = UnixStream::connect(USBMUXD_SOCKET)?;
    let mut req = Dictionary::new();
    req.insert("MessageType".into(), Value::String("Connect".into()));
    req.insert("ClientVersionString".into(), Value::String("opencam-desktop".into()));
    req.insert("ProgName".into(), Value::String("opencam-desktop".into()));
    req.insert("DeviceID".into(), Value::Integer(device_id.into()));
    // usbmuxd espera el puerto en orden de red (big-endian) dentro de un entero.
    req.insert("PortNumber".into(), Value::Integer(device_port.to_be().into()));
    let reply = exchange(&mut stream, &req)?;
    match reply.get("Number").and_then(Value::as_unsigned_integer) {
        Some(0) => Ok(stream),
        Some(code) => Err(io::Error::other(format!("usbmuxd Connect falló (código {code})"))),
        None => Err(io::Error::new(io::ErrorKind::InvalidData, "respuesta de usbmuxd no válida")),
    }
}

/// Lista los IDs de los dispositivos iOS conectados por USB.
fn list_devices() -> io::Result<Vec<u64>> {
    let mut stream = UnixStream::connect(USBMUXD_SOCKET)?;
    let mut req = Dictionary::new();
    req.insert("MessageType".into(), Value::String("ListDevices".into()));
    let reply = exchange(&mut stream, &req)?;
    let list = reply
        .get("DeviceList")
        .and_then(Value::as_array)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "falta DeviceList"))?;
    Ok(list
        .iter()
        .filter_map(|d| d.as_dictionary()?.get("DeviceID")?.as_unsigned_integer())
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
