//! Argumentos de línea de órdenes, sin dependencias externas.

use crate::settings::{parse_bool, parse_zoom, Facing, Focus, LiveSettings};

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
    --device <ruta>    dispositivo v4l2loopback (Linux) [/dev/video10]
    --width <px>       ancho [1920]
    --height <px>      alto [1080]
    --fps <n>          fotogramas por segundo [30]
    --bitrate <kbps>   tasa de bits inicial [8000]
    --codec <c>        h264 (h265 aún no se decodifica en el escritorio) [h264]
    --lens <l>         lente trasera: wide, ultrawide o tele [wide]
    --camera <c>       frontal o trasera [lo elegido en el móvil]
    --zoom <x>         zoom, 1 = sin zoom (p. ej. 2 o 1.5x) [lo del móvil]
    --focus <f>        auto, o manual 0..1 (0 = infinito, 1 = lo más cerca) [lo del móvil]
    --stabilization <on|off>  estabilización de vídeo [lo del móvil]
    --mirror           voltea la imagen en horizontal (modo espejo)
    --no-controls      no leer órdenes en vivo de la consola (zoom, enfoque...)
    --sink <tipo>      v4l2 (Linux), windows o y4m [v4l2 en Linux, windows en Windows, y4m en macOS]
    --output <ruta>    archivo de salida de --sink y4m; `-` es stdout [-]
    --list             lista teléfonos (mDNS) e iPhones USB y sale
    --discover         conecta al primer teléfono hallado por mDNS
    --udid <udid>      elige el iPhone USB por UDID (con `iphone`)
    --no-reconnect     salir al perder la conexión en vez de reconectar
    -h, --help         muestra esta ayuda

Mientras transmite, escribe `ayuda` en la consola para ver las órdenes en vivo.
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkKind {
    V4l2,
    Windows,
    Y4m,
}

impl SinkKind {
    fn default_for_os() -> Self {
        if cfg!(target_os = "linux") {
            Self::V4l2
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Y4m
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cli {
    pub addr: String,
    pub device: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub codec: String,
    pub lens: String,
    /// Ajustes avanzados (cámara, zoom, enfoque, estabilización, espejo).
    /// Su `bitrate_kbps` no se usa: manda el de arriba (ver `live()`).
    pub advanced: LiveSettings,
    pub controls: bool,
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
            device: "/dev/video10".into(),
            width: 1920,
            height: 1080,
            fps: 30,
            bitrate_kbps: 8000,
            codec: "h264".into(),
            lens: "wide".into(),
            advanced: LiveSettings::default(),
            controls: true,
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
                "--no-controls" => cli.controls = false,
                "--mirror" => cli.advanced.mirror = true,
                "--codec" => {
                    cli.codec = match value("--codec")?.as_str() {
                        "h264" => "h264".into(),
                        "h265" => {
                            return Err(CliError::Invalid(
                                "--codec h265: el escritorio aún no decodifica H.265; usa h264"
                                    .into(),
                            ))
                        }
                        o => return Err(CliError::Invalid(format!("--codec desconocido: {o}"))),
                    }
                }
                "--lens" => {
                    let l = value("--lens")?;
                    if !matches!(l.as_str(), "wide" | "ultrawide" | "tele") {
                        return Err(CliError::Invalid(format!(
                            "--lens desconocida: {l} (wide|ultrawide|tele)"
                        )));
                    }
                    cli.lens = l;
                }
                "--camera" => {
                    let c = value("--camera")?;
                    cli.advanced.facing = Some(Facing::parse(&c).ok_or_else(|| {
                        CliError::Invalid(format!("--camera desconocida: {c} (frontal|trasera)"))
                    })?);
                }
                "--zoom" => {
                    let z = value("--zoom")?;
                    cli.advanced.zoom = Some(parse_zoom(&z).ok_or_else(|| {
                        CliError::Invalid(format!("--zoom necesita un número ≥ 1: {z}"))
                    })?);
                }
                "--focus" => {
                    let f = value("--focus")?;
                    cli.advanced.focus = Some(Focus::parse(&f).ok_or_else(|| {
                        CliError::Invalid(format!("--focus necesita auto o un número 0..1: {f}"))
                    })?);
                }
                "--stabilization" => {
                    let b = value("--stabilization")?;
                    cli.advanced.stabilization = Some(parse_bool(&b).ok_or_else(|| {
                        CliError::Invalid(format!("--stabilization necesita on u off: {b}"))
                    })?);
                }
                "--list" => cli.list = true,
                "--discover" => cli.discover = true,
                "--udid" => cli.udid = Some(value("--udid")?),
                "--output" => cli.output = value("--output")?,
                "--sink" => {
                    cli.sink = match value("--sink")?.as_str() {
                        "v4l2" => SinkKind::V4l2,
                        "windows" => SinkKind::Windows,
                        "y4m" => SinkKind::Y4m,
                        o => {
                            return Err(CliError::Invalid(format!(
                                "--sink desconocido: {o} (v4l2|windows|y4m)"
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

    /// Ajustes en vivo iniciales, con el bitrate de `--bitrate`.
    pub fn live(&self) -> LiveSettings {
        LiveSettings {
            bitrate_kbps: self.bitrate_kbps,
            ..self.advanced.clone()
        }
    }

    /// JSON del mensaje CONFIG (ver docs/PROTOCOL.md).
    pub fn config_json(&self, live: &LiveSettings) -> String {
        format!(
            r#"{{"codec":"{}","width":{},"height":{},"fps":{},"bitrate_kbps":{},"lens":"{}"{}}}"#,
            self.codec,
            self.width,
            self.height,
            self.fps,
            live.bitrate_kbps,
            self.lens,
            live.config_fields()
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
            c.config_json(&c.live()),
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
            .config_json(&c.live())
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
    fn advanced_options() {
        let c = parse(&[
            "--camera",
            "frontal",
            "--zoom",
            "2",
            "--focus",
            "0.5",
            "--stabilization",
            "on",
            "--mirror",
            "--lens",
            "tele",
            "--no-controls",
        ])
        .unwrap();
        assert!(c.advanced.mirror && !c.controls);
        assert_eq!(
            c.config_json(&c.live()),
            r#"{"codec":"h264","width":1920,"height":1080,"fps":30,"bitrate_kbps":8000,"lens":"tele","facing":"front","stabilization":true,"zoom":2,"focus":"manual","focus_distance":0.5}"#
        );
        assert!(matches!(
            parse(&["--codec", "h265"]),
            Err(CliError::Invalid(_))
        ));
        assert!(matches!(parse(&["--zoom", "0"]), Err(CliError::Invalid(_))));
        assert!(matches!(
            parse(&["--camera", "x"]),
            Err(CliError::Invalid(_))
        ));
        assert!(matches!(parse(&["--lens", "x"]), Err(CliError::Invalid(_))));
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
