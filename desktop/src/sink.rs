//! Interfaz común de los sinks de vídeo y un sink Y4M (archivo/stdout) para macOS y tests.

use std::fs::File;
use std::io::{self, Write};

pub trait FrameSink {
    /// Escribe un frame YUYV (2 bytes por píxel) del tamaño con el que se abrió el sink.
    fn write_yuyv(&mut self, yuyv: &[u8]) -> io::Result<()>;
}

/// Descarta los frames: la interfaz gráfica sin cámara virtual (p. ej. macOS) solo muestra la vista previa.
pub struct NullSink;

impl FrameSink for NullSink {
    fn write_yuyv(&mut self, _yuyv: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

/// Escribe YUV4MPEG2 (4:2:2 planar) a cualquier `Write`.
pub struct Y4mSink<W: Write> {
    out: W,
    width: usize,
    height: usize,
}

impl<W: Write> Y4mSink<W> {
    pub fn new(mut out: W, width: usize, height: usize, fps: u32) -> io::Result<Self> {
        if width == 0 || height == 0 || !width.is_multiple_of(2) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "tamaño Y4M inválido (ancho par)",
            ));
        }
        writeln!(out, "YUV4MPEG2 W{width} H{height} F{fps}:1 Ip A1:1 C422")?;
        Ok(Self { out, width, height })
    }
}

impl Y4mSink<Box<dyn Write>> {
    /// `-` es stdout; cualquier otra ruta es un archivo nuevo.
    pub fn open(path: &str, width: usize, height: usize, fps: u32) -> io::Result<Self> {
        let out: Box<dyn Write> = if path == "-" {
            Box::new(io::stdout().lock())
        } else {
            Box::new(io::BufWriter::new(File::create(path)?))
        };
        Self::new(out, width, height, fps)
    }
}

impl<W: Write> FrameSink for Y4mSink<W> {
    fn write_yuyv(&mut self, yuyv: &[u8]) -> io::Result<()> {
        if yuyv.len() != self.width * self.height * 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "tamaño de frame incorrecto",
            ));
        }
        // YUYV -> planos Y, U, V (4:2:2).
        let (mut y, mut u, mut v) = (Vec::new(), Vec::new(), Vec::new());
        for p in yuyv.chunks_exact(4) {
            y.extend_from_slice(&[p[0], p[2]]);
            u.push(p[1]);
            v.push(p[3]);
        }
        self.out.write_all(b"FRAME\n")?;
        self.out.write_all(&y)?;
        self.out.write_all(&u)?;
        self.out.write_all(&v)?;
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_frame_layout() {
        let mut s = Y4mSink::new(Vec::new(), 4, 2, 30).unwrap();
        s.write_yuyv(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16])
            .unwrap();
        let hdr = b"YUV4MPEG2 W4 H2 F30:1 Ip A1:1 C422\nFRAME\n";
        assert!(s.out.starts_with(hdr));
        assert_eq!(
            &s.out[hdr.len()..],
            &[1, 3, 5, 7, 9, 11, 13, 15, 2, 6, 10, 14, 4, 8, 12, 16]
        );
    }

    #[test]
    fn rejects_wrong_size() {
        let mut s = Y4mSink::new(Vec::new(), 4, 2, 30).unwrap();
        assert!(s.write_yuyv(&[0; 3]).is_err());
    }
}
