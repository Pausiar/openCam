//! Ajustes de la interfaz gráfica guardados entre sesiones, en un archivo `clave=valor`:
//! Windows `%APPDATA%\OpenCam\settings.txt`, macOS `~/Library/Application Support/OpenCam/settings.txt`,
//! Linux `$XDG_CONFIG_HOME/opencam/settings.txt` (o `~/.config/opencam/settings.txt`).

use std::fs;
use std::path::PathBuf;

use crate::cli::{Cli, SinkKind};

pub fn path() -> Option<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let dir = if cfg!(windows) {
        env("APPDATA")?.join("OpenCam")
    } else if cfg!(target_os = "macos") {
        env("HOME")?.join("Library/Application Support/OpenCam")
    } else {
        env("XDG_CONFIG_HOME")
            .or_else(|| env("HOME").map(|h| h.join(".config")))?
            .join("opencam")
    };
    Some(dir.join("settings.txt"))
}

fn sink_name(s: SinkKind) -> &'static str {
    match s {
        SinkKind::V4l2 => "v4l2",
        SinkKind::Windows => "windows",
        SinkKind::Y4m => "y4m",
        SinkKind::None => "none",
    }
}

pub fn to_text(c: &Cli) -> String {
    format!(
        "addr={}\nwidth={}\nheight={}\nfps={}\nbitrate_kbps={}\nphone_settings={}\nreconnect={}\nsink={}\ndevice={}\noutput={}\n",
        c.addr,
        c.width,
        c.height,
        c.fps,
        c.bitrate_kbps,
        c.phone_settings,
        c.reconnect,
        sink_name(c.sink),
        c.device,
        c.output
    )
}

/// Aplica sobre `base` las claves válidas de `text`; ignora las desconocidas o mal formadas.
pub fn from_text(text: &str, base: Cli) -> Cli {
    let mut c = base;
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        let num = || v.parse::<u32>().ok().filter(|n| *n > 0);
        let flag = || v.parse::<bool>().ok();
        match k {
            "addr" if !v.is_empty() => c.addr = v.into(),
            "width" => c.width = num().unwrap_or(c.width),
            "height" => c.height = num().unwrap_or(c.height),
            "fps" => c.fps = num().unwrap_or(c.fps),
            "bitrate_kbps" => c.bitrate_kbps = num().unwrap_or(c.bitrate_kbps),
            "phone_settings" => c.phone_settings = flag().unwrap_or(c.phone_settings),
            "reconnect" => c.reconnect = flag().unwrap_or(c.reconnect),
            "sink" => {
                c.sink = match v {
                    "v4l2" if cfg!(target_os = "linux") => SinkKind::V4l2,
                    "windows" if cfg!(windows) => SinkKind::Windows,
                    "y4m" => SinkKind::Y4m,
                    "none" => SinkKind::None,
                    _ => c.sink,
                }
            }
            "device" if !v.is_empty() => c.device = v.into(),
            "output" if !v.is_empty() => c.output = v.into(),
            _ => {}
        }
    }
    c
}

pub fn load(base: Cli) -> Cli {
    match path().and_then(|p| fs::read_to_string(p).ok()) {
        Some(text) => from_text(&text, base),
        None => base,
    }
}

pub fn save(c: &Cli) -> std::io::Result<()> {
    let p = path().ok_or_else(|| std::io::Error::other("sin carpeta de configuración"))?;
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(p, to_text(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let c = Cli {
            addr: "192.168.1.20:4747".into(),
            width: 1280,
            height: 720,
            fps: 60,
            bitrate_kbps: 4000,
            phone_settings: true,
            reconnect: false,
            sink: SinkKind::None,
            output: "/tmp/a.y4m".into(),
            ..Cli::default()
        };
        assert_eq!(from_text(&to_text(&c), Cli::default()), c);
    }

    #[test]
    fn ignores_garbage() {
        let c = from_text("fps=0\nwidth=abc\nnope=1\nsin_igual\n", Cli::default());
        assert_eq!(c, Cli::default());
    }
}
