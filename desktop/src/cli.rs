//! Argumentos de línea de órdenes, sin dependencias externas.

pub const HELP: &str = "\
opencam-desktop: recibe vídeo OPCM del móvil y lo expone como cámara virtual

USO:
    opencam-desktop                         abre la interfaz gráfica
    opencam-desktop [DIRECCION] [OPCIONES]  modo línea de órdenes

DIRECCION (o --addr):
    auto          busca solo: Android por USB (adb), iPhone por USB y luego Wi-Fi (mDNS)
    usb           Android por USB; hace `adb forward` automáticamente
    wifi          primer móvil anunciado por mDNS en la red local
    iphone        iPhone por USB (usbmuxd, macOS/Linux)
    host:puerto   dirección fija, p. ej. 192.168.1.20:4747
    Por defecto: auto

OPCIONES:
    --addr <dir>       igual que el argumento posicional
    --device <ruta>    dispositivo v4l2loopback (Linux) [/dev/video10]
    --width <px>       ancho [1920]
    --height <px>      alto [1080]
    --fps <n>          fotogramas por segundo [30]
    --bitrate <kbps>   tasa de bits [8000]
    --phone-settings   usa la resolución, fps y bitrate elegidos en el móvil (no los pide)
    --sink <tipo>      v4l2 (Linux), windows, y4m o none (solo recibe)
                       [v4l2 en Linux, windows en Windows, y4m en macOS]
    --output <ruta>    archivo de salida de --sink y4m; `-` es stdout [-]
    --list             lista teléfonos (mDNS) e iPhones USB y sale
    --discover         igual que `wifi`
    --udid <id>        elige el móvil USB: UDID del iPhone o número de serie de adb
    --serial <id>      igual que --udid
    --adb <ruta>       ruta de adb si no está en el PATH ni en el SDK de Android
    --no-reconnect     salir al perder la conexión en vez de reconectar
    --gui              abre la interfaz gráfica (lo mismo que sin argumentos)
    -h, --help         muestra esta ayuda
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkKind {
    V4l2,
    Windows,
    Y4m,
    /// Sin salida: solo recibe (la interfaz gráfica muestra la vista previa).
    None,
}

impl SinkKind {
    pub fn default_for_os() -> Self {
        if cfg!(target_os = "linux") {
            Self::V4l2
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Y4m
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cli {
    pub addr: String,
    pub device: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    /// No pedir resolución/fps/bitrate: el móvil usa los suyos.
    pub phone_settings: bool,
    pub reconnect: bool,
    pub sink: SinkKind,
    pub output: String,
    pub list: bool,
    pub discover: bool,
    pub udid: Option<String>,
    pub adb: Option<String>,
    pub gui: bool,
}

impl Default for Cli {
    fn default() -> Self {
        Self {
            addr: "auto".into(),
            device: "/dev/video10".into(),
            width: 1920,
            height: 1080,
            fps: 30,
            bitrate_kbps: 8000,
            phone_settings: false,
            reconnect: true,
            sink: SinkKind::default_for_os(),
            output: "-".into(),
            list: false,
            discover: false,
            udid: None,
            adb: None,
            gui: false,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum CliError {
    Help,
    Invalid(String),
}

impl Cli {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, CliError> {
        let mut cli = Cli::default();
        let mut args = args.into_iter();
        while let Some(a) = args.next() {
            let mut value = |name: &str| {
                args.next()
                    .ok_or_else(|| CliError::Invalid(format!("falta el valor de {name}")))
            };
            match a.as_str() {
                "-h" | "--help" => return Err(CliError::Help),
                "--no-reconnect" => cli.reconnect = false,
                "--phone-settings" => cli.phone_settings = true,
                "--gui" => cli.gui = true,
                "--list" => cli.list = true,
                "--discover" => cli.discover = true,
                "--udid" | "--serial" => cli.udid = Some(value(&a)?),
                "--adb" => cli.adb = Some(value("--adb")?),
                "--output" => cli.output = value("--output")?,
                "--sink" => {
                    cli.sink = match value("--sink")?.as_str() {
                        "v4l2" => SinkKind::V4l2,
                        "windows" => SinkKind::Windows,
                        "y4m" => SinkKind::Y4m,
                        "none" => SinkKind::None,
                        o => {
                            return Err(CliError::Invalid(format!(
                                "--sink desconocido: {o} (v4l2|windows|y4m|none)"
                            )))
                        }
                    }
                }
                "--addr" => cli.addr = value("--addr")?,
                "--device" => cli.device = value("--device")?,
                "--width" | "--height" | "--fps" | "--bitrate" => {
                    let n: u32 = value(&a)?.parse().ok().filter(|n| *n > 0).ok_or_else(|| {
                        CliError::Invalid(format!("{a} necesita un entero positivo"))
                    })?;
                    match a.as_str() {
                        "--width" => cli.width = n,
                        "--height" => cli.height = n,
                        "--fps" => cli.fps = n,
                        _ => cli.bitrate_kbps = n,
                    }
                }
                s if s.starts_with('-') => {
                    return Err(CliError::Invalid(format!("opción desconocida: {s}")))
                }
                _ => cli.addr = a, // `iphone` o host:puerto
            }
        }
        Ok(cli)
    }

    /// JSON del mensaje CONFIG (ver docs/PROTOCOL.md). Con `phone_settings` solo fija el códec
    /// y el móvil conserva sus ajustes locales para el resto de campos.
    pub fn config_json(&self) -> String {
        if self.phone_settings {
            return r#"{"codec":"h264"}"#.into();
        }
        format!(
            r#"{{"codec":"h264","width":{},"height":{},"fps":{},"bitrate_kbps":{},"lens":"wide"}}"#,
            self.width, self.height, self.fps, self.bitrate_kbps
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(a: &[&str]) -> Result<Cli, CliError> {
        Cli::parse(a.iter().map(|s| s.to_string()))
    }

    #[test]
    fn defaults() {
        let c = parse(&[]).unwrap();
        assert_eq!(c, Cli::default());
        assert_eq!(
            c.config_json(),
            r#"{"codec":"h264","width":1920,"height":1080,"fps":30,"bitrate_kbps":8000,"lens":"wide"}"#
        );
    }

    #[test]
    fn all_options() {
        let c = parse(&[
            "iphone",
            "--device",
            "/dev/video2",
            "--width",
            "1280",
            "--height",
            "720",
            "--fps",
            "60",
            "--bitrate",
            "4000",
            "--no-reconnect",
        ])
        .unwrap();
        assert_eq!(c.addr, "iphone");
        assert_eq!(c.device, "/dev/video2");
        assert_eq!(
            (c.width, c.height, c.fps, c.bitrate_kbps),
            (1280, 720, 60, 4000)
        );
        assert!(!c.reconnect);
        assert!(c
            .config_json()
            .contains(r#""width":1280,"height":720,"fps":60,"bitrate_kbps":4000"#));
        assert_eq!(
            parse(&["--addr", "10.0.0.5:4747"]).unwrap().addr,
            "10.0.0.5:4747"
        );
    }

    #[test]
    fn errors() {
        assert_eq!(parse(&["--help"]), Err(CliError::Help));
        assert!(matches!(parse(&["--fps"]), Err(CliError::Invalid(_))));
        assert!(matches!(parse(&["--fps", "x"]), Err(CliError::Invalid(_))));
        assert!(matches!(
            parse(&["--width", "0"]),
            Err(CliError::Invalid(_))
        ));
        assert!(matches!(parse(&["--nope"]), Err(CliError::Invalid(_))));
        assert!(matches!(parse(&["--sink", "x"]), Err(CliError::Invalid(_))));
    }

    #[test]
    fn sink_and_discovery_options() {
        let c = parse(&[
            "--sink",
            "y4m",
            "--output",
            "a.y4m",
            "--discover",
            "--list",
            "--udid",
            "ABC",
        ])
        .unwrap();
        assert_eq!(
            (c.sink, c.output.as_str(), c.discover, c.list),
            (SinkKind::Y4m, "a.y4m", true, true)
        );
        assert_eq!(c.udid.as_deref(), Some("ABC"));
        let c = parse(&["usb", "--serial", "R58M", "--adb", "/opt/adb"]).unwrap();
        assert_eq!(
            (c.addr.as_str(), c.udid.as_deref(), c.adb.as_deref()),
            ("usb", Some("R58M"), Some("/opt/adb"))
        );
    }

    #[test]
    fn phone_settings_only_sends_codec() {
        let c = parse(&["--phone-settings", "--sink", "none", "--gui"]).unwrap();
        assert!(c.phone_settings && c.gui);
        assert_eq!(c.sink, SinkKind::None);
        assert_eq!(c.config_json(), r#"{"codec":"h264"}"#);
    }
}
