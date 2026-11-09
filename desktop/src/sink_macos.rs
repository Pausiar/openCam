//! Cámara virtual en macOS: los frames van a la extensión de cámara CoreMediaIO de
//! `OpenCam.app` (carpeta `macos/` del repo).
//!
//! La extensión corre aislada como extensión de sistema, así que no puede leer memoria
//! del cliente. Su entrada es un stream *sink* de CoreMediaIO, y quien le mete frames es
//! la propia app en modo `--feed`: este sink la lanza como proceso hijo y le escribe por
//! stdin frames YUYV crudos de `WIDTH` x `HEIGHT`, uno tras otro, sin cabeceras.
//!
//! Sin probar en un Mac real todavía; ver desktop/README.md.

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};

/// Formato fijo de la extensión (`macos/CameraExtension/Provider.swift`).
pub const WIDTH: usize = 1920;
pub const HEIGHT: usize = 1080;

/// Variable de entorno para usar otra copia de la app (p. ej. la de DerivedData).
const FEEDER_ENV: &str = "OPENCAM_FEEDER";
const APP_EXECUTABLE: &str = "OpenCam.app/Contents/MacOS/OpenCam";

pub struct VirtualCamera {
    child: Child,
    stdin: ChildStdin,
}

fn find_feeder() -> io::Result<PathBuf> {
    if let Some(p) = std::env::var_os(FEEDER_ENV) {
        return Ok(PathBuf::from(p));
    }
    let mut candidates = vec![PathBuf::from("/Applications").join(APP_EXECUTABLE)];
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(
            PathBuf::from(home)
                .join("Applications")
                .join(APP_EXECUTABLE),
        );
    }
    candidates.into_iter().find(|p| p.exists()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no se encontró OpenCam.app en /Applications. Instálala y pulsa «Instalar cámara» \
             (ver desktop/README.md), define OPENCAM_FEEDER con la ruta de su ejecutable, \
             o usa --sink y4m",
        )
    })
}

impl VirtualCamera {
    pub fn open(fps: u32) -> io::Result<Self> {
        let feeder = find_feeder()?;
        let mut child = Command::new(&feeder)
            .args(["--feed", "--fps", &fps.to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", feeder.display())))?;
        let stdin = child.stdin.take().expect("stdin del proceso hijo");
        eprintln!("Cámara virtual: OpenCam ({WIDTH}x{HEIGHT} a {fps} fps)");
        Ok(Self { child, stdin })
    }
}

impl crate::sink::FrameSink for VirtualCamera {
    fn write_yuyv(&mut self, yuyv: &[u8]) -> io::Result<()> {
        if yuyv.len() != WIDTH * HEIGHT * 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "tamaño de frame incorrecto",
            ));
        }
        self.stdin.write_all(yuyv).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "la app OpenCam dejó de aceptar frames ({e}); mira su mensaje de error arriba"
                ),
            )
        })
    }

    fn fixed_size(&self) -> Option<(usize, usize)> {
        Some((WIDTH, HEIGHT))
    }
}

impl Drop for VirtualCamera {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
