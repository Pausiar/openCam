//! Argumentos de línea de órdenes, sin dependencias externas.

pub const HELP: &str = "\
opencam-desktop: recibe vídeo OPCM del móvil y lo expone como cámara virtual

USO:
    opencam-desktop [DIRECCION] [OPCIONES]

DIRECCION (o --addr):
    host:puerto   móvil por Wi-Fi, o 127.0.0.1:4747 tras `adb forward tcp:4747 tcp:4747`
    iphone        iPhone por USB (usbmuxd, macOS/Linux)
    Por defecto: 127.0.0.1:4747

OPCIONES:
    --addr <dir>       igual que el argumento posicional
    --device <ruta>    dispositivo v4l2loopback (Linux); `auto` busca el llamado OpenCam [auto]
    --width <px>       ancho [1920]
    --height <px>      alto [1080]
    --fps <n>          fotogramas por segundo [30]
    --bitrate <kbps>   tasa de bits [8000]
    --sink <tipo>      v4l2 (Linux), windows, macos o y4m [el del sistema operativo]
    --output <ruta>    archivo de salida de --sink y4m; `-` es stdout [-]
    --list             lista teléfonos (mDNS) e iPhones USB y sale
    --discover         conecta al primer teléfono hallado por mDNS
    --udid <udid>      elige el iPhone USB por UDID (con `iphone`)
    --no-reconnect     salir al perder la conexión en vez de reconectar
    -h, --help         muestra esta ayuda
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkKind {
    V4l2,
    Windows,
    Macos,
    Y4m,
}

impl SinkKind {
    fn default_for_os() -> Self {
        if cfg!(target_os = "linux") {
            Self::V4l2
        } else if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Y4m
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Cli {
    pub addr: String,
    pub device: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub reconnect: bool,
    pub sink: SinkKind,
    pub output: String,
    pub list: bool,
    pub discover: bool,
    pub udid: Option<String>,
}

impl Default for Cli {
    fn default() -> Self {
        Self {
            addr: "127.0.0.1:4747".into(),
            device: "auto".into(),
            width: 1920,
            height: 1080,
            fps: 30,
            bitrate_kbps: 8000,
            reconnect: true,
            sink: SinkKind::default_for_os(),
            output: "-".into(),
            list: false,
            discover: false,
            udid: None,
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
                "--list" => cli.list = true,
                "--discover" => cli.discover = true,
                "--udid" => cli.udid = Some(value("--udid")?),
                "--output" => cli.output = value("--output")?,
                "--sink" => {
                    cli.sink = match value("--sink")?.as_str() {
                        "v4l2" => SinkKind::V4l2,
                        "windows" => SinkKind::Windows,
                        "macos" => SinkKind::Macos,
                        "y4m" => SinkKind::Y4m,
                        o => {
                            return Err(CliError::Invalid(format!(
                                "--sink desconocido: {o} (v4l2|windows|macos|y4m)"
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

    /// JSON del mensaje CONFIG (ver docs/PROTOCOL.md).
    pub fn config_json(&self) -> String {
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
    }
}
