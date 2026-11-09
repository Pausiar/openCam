//! Android por USB sin pasos manuales: localiza `adb`, elige el móvil y crea (o reutiliza) un
//! `adb forward tcp:N tcp:4747`. El escritorio se conecta luego a `127.0.0.1:N`.
//! Requisitos en el móvil: depuración USB activada y el aviso de la huella RSA aceptado.

use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Móvil visto por `adb devices`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub serial: String,
    /// `device` (listo), `unauthorized`, `offline`, `recovery`...
    pub state: String,
}

impl Device {
    pub fn ready(&self) -> bool {
        self.state == "device"
    }
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "adb.exe"
    } else {
        "adb"
    }
}

/// Carpetas donde suele estar `platform-tools`, además del PATH.
fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    // Junto al ejecutable (p. ej. si el instalador copia platform-tools).
    if let Some(dir) = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        dirs.push(dir.join("platform-tools"));
        dirs.push(dir);
    }
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(v) = env::var_os(var) {
            dirs.push(PathBuf::from(v).join("platform-tools"));
        }
    }
    if let Some(v) = env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(v).join("Android/Sdk/platform-tools"));
    }
    if let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")) {
        let home = PathBuf::from(home);
        dirs.push(home.join("Library/Android/sdk/platform-tools")); // macOS
        dirs.push(home.join("Android/Sdk/platform-tools")); // Linux
    }
    if let Some(path) = env::var_os("PATH") {
        dirs.extend(env::split_paths(&path));
    }
    dirs
}

/// Ruta de `adb`: `override_path`, la variable `ADB`, o la primera de [`candidate_dirs`].
pub fn find(override_path: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = override_path {
        return Some(PathBuf::from(p));
    }
    if let Some(p) = env::var_os("ADB").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    candidate_dirs()
        .into_iter()
        .map(|d| d.join(exe_name()))
        .find(|p| p.is_file())
}

fn run(adb: &Path, args: &[&str]) -> io::Result<Output> {
    let mut cmd = Command::new(adb);
    cmd.args(args);
    #[cfg(windows)]
    {
        // Sin ventana de consola parpadeando cuando lo lanza una interfaz gráfica.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("no se pudo ejecutar {}: {e}", adb.display()),
        )
    })?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr);
        return Err(io::Error::other(format!(
            "adb {}: {}",
            args.join(" "),
            msg.trim()
        )));
    }
    Ok(out)
}

/// Interpreta la salida de `adb devices`.
pub fn parse_devices(out: &str) -> Vec<Device> {
    out.lines()
        .filter(|l| !l.starts_with("List of devices") && !l.starts_with('*'))
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some(Device {
                serial: it.next()?.to_string(),
                state: it.next()?.to_string(),
            })
        })
        .collect()
}

pub fn devices(adb: &Path) -> io::Result<Vec<Device>> {
    let out = run(adb, &["devices"])?;
    Ok(parse_devices(&String::from_utf8_lossy(&out.stdout)))
}

/// Puerto local de un forward ya existente `serial tcp:N tcp:remote` en `adb forward --list`.
pub fn parse_forward_list(out: &str, serial: &str, remote: u16) -> Option<u16> {
    let want = format!("tcp:{remote}");
    out.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let (s, local, r) = (it.next()?, it.next()?, it.next()?);
        if s == serial && r == want {
            local.strip_prefix("tcp:")?.parse().ok()
        } else {
            None
        }
    })
}

/// Elige el móvil: el de `serial` si se indica, si no el único listo. Los errores explican qué
/// hacer en el móvil (autorizar la depuración, conectar el cable...).
pub fn pick(devs: &[Device], serial: Option<&str>) -> io::Result<Device> {
    if let Some(s) = serial {
        let d = devs.iter().find(|d| d.serial == s).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("adb no ve el dispositivo {s}"),
            )
        })?;
        return check_ready(d);
    }
    match devs.iter().find(|d| d.ready()) {
        Some(d) => Ok(d.clone()),
        None => match devs.first() {
            Some(d) => check_ready(d),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no hay ningún Android por USB (¿cable conectado y depuración USB activada?)",
            )),
        },
    }
}

fn check_ready(d: &Device) -> io::Result<Device> {
    match d.state.as_str() {
        "device" => Ok(d.clone()),
        "unauthorized" => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{}: acepta en el móvil el aviso «¿Permitir depuración USB?»",
                d.serial
            ),
        )),
        s => Err(io::Error::other(format!("{}: estado adb «{s}»", d.serial))),
    }
}

/// Garantiza un forward hacia `remote` en el móvil y devuelve el puerto local.
pub fn forward(adb: &Path, serial: &str, remote: u16) -> io::Result<u16> {
    if let Ok(out) = run(adb, &["-s", serial, "forward", "--list"]) {
        if let Some(p) = parse_forward_list(&String::from_utf8_lossy(&out.stdout), serial, remote) {
            return Ok(p);
        }
    }
    let remote_arg = format!("tcp:{remote}");
    // `tcp:0`: adb elige un puerto libre y lo imprime (evita choques con varios móviles).
    let out = run(adb, &["-s", serial, "forward", "tcp:0", &remote_arg])?;
    if let Ok(p) = String::from_utf8_lossy(&out.stdout).trim().parse() {
        return Ok(p);
    }
    // adb antiguo que no imprime el puerto: puerto fijo.
    let local = format!("tcp:{remote}");
    run(adb, &["-s", serial, "forward", &local, &remote_arg])?;
    Ok(remote)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_devices() {
        let out = "* daemon not running; starting now at tcp:5037\n\
                   * daemon started successfully\n\
                   List of devices attached\n\
                   R58M123ABC\tdevice\n\
                   emulator-5554\tunauthorized\n\n";
        let d = parse_devices(out);
        assert_eq!(
            d,
            vec![
                Device {
                    serial: "R58M123ABC".into(),
                    state: "device".into()
                },
                Device {
                    serial: "emulator-5554".into(),
                    state: "unauthorized".into()
                },
            ]
        );
        assert!(d[0].ready() && !d[1].ready());
    }

    #[test]
    fn parses_forward_list() {
        let out = "R58M tcp:41234 tcp:4747\nR58M tcp:5000 tcp:9999\nOTRO tcp:6000 tcp:4747\n";
        assert_eq!(parse_forward_list(out, "R58M", 4747), Some(41234));
        assert_eq!(parse_forward_list(out, "OTRO", 4747), Some(6000));
        assert_eq!(parse_forward_list(out, "NADA", 4747), None);
    }

    #[test]
    fn picks_device() {
        let devs = parse_devices("A\tunauthorized\nB\tdevice\n");
        assert_eq!(pick(&devs, None).unwrap().serial, "B");
        let e = pick(&devs, Some("A")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(pick(&[], None).unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(
            pick(&devs, Some("Z")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }
}
