//! Decodificador H.264 (OpenH264, software) de las unidades Annex-B recibidas por OPCM.
//!
//! Los frames pasan antes por la `FrameQueue` del núcleo, que descarta P-frames
//! hasta el primer keyframe y limita la cola para mantener la latencia baja.

use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
use opencam_core::{Frame, FrameQueue};

use crate::convert::{self, ConvertError};

/// Error de `next_yuyv`: fallo del decodificador o tamaño de salida inválido.
#[derive(Debug)]
#[allow(dead_code)] // lo cablea main.rs en otra rama
pub enum DecodeError {
    Decode(openh264::Error),
    Convert(ConvertError),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(e) => write!(f, "decodificador H.264: {e}"),
            Self::Convert(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for DecodeError {}

pub struct VideoDecoder {
    decoder: Decoder,
    queue: FrameQueue,
    seq: u32,
}

/// Imagen decodificada en RGB8, lista para el sink de cámara virtual.
pub struct RgbFrame {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

impl VideoDecoder {
    pub fn new(queue_capacity: usize) -> Result<Self, openh264::Error> {
        let decoder = Decoder::new()?;
        Ok(Self { decoder, queue: FrameQueue::new(queue_capacity), seq: 0 })
    }

    /// Mete una unidad de acceso. Devuelve `true` si hace falta pedir un keyframe.
    pub fn push_access_unit(&mut self, data: Vec<u8>, timestamp_us: u64, keyframe: bool) -> bool {
        self.seq = self.seq.wrapping_add(1);
        self.queue.push(Frame { seq: self.seq, timestamp_us, keyframe, data })
    }

    /// Decodifica el frame más reciente disponible. Devuelve `Ok(None)` si no hay nada.
    pub fn next_rgb(&mut self) -> Result<Option<RgbFrame>, openh264::Error> {
        let Some(frame) = self.queue.pop() else {
            return Ok(None);
        };
        let Some(yuv) = self.decoder.decode(&frame.data)? else {
            return Ok(None);
        };
        let (width, height) = yuv.dimensions();
        let mut rgb = vec![0u8; width * height * 3];
        yuv.write_rgb8(&mut rgb);
        Ok(Some(RgbFrame { width, height, rgb }))
    }

    /// Como `next_rgb`, pero devuelve YUYV de `out_w` x `out_h` (escalado con letterbox).
    #[allow(dead_code)] // lo cablea main.rs en otra rama
    pub fn next_yuyv(&mut self, out_w: usize, out_h: usize) -> Result<Option<Vec<u8>>, DecodeError> {
        let Some(frame) = self.queue.pop() else {
            return Ok(None);
        };
        let Some(yuv) = self.decoder.decode(&frame.data).map_err(DecodeError::Decode)? else {
            return Ok(None);
        };
        convert::i420_to_yuyv(&yuv, out_w, out_h).map(Some).map_err(DecodeError::Convert)
    }

    pub fn dropped(&self) -> u64 {
        self.queue.dropped()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openh264::encoder::Encoder;
    use openh264::formats::YUVBuffer;

    fn encode_solid(w: usize, h: usize) -> Vec<u8> {
        let mut buf = vec![120u8; w * h];
        buf.extend(vec![90u8; w * h / 4]);
        buf.extend(vec![200u8; w * h / 4]);
        Encoder::new().unwrap().encode(&YUVBuffer::from_vec(buf, w, h)).unwrap().to_vec()
    }

    #[test]
    fn encode_decode_to_yuyv() {
        let mut dec = VideoDecoder::new(4).unwrap();
        dec.push_access_unit(encode_solid(64, 48), 0, true);
        let out = dec.next_yuyv(128, 72).unwrap().expect("frame");
        assert_eq!(out.len(), 128 * 72 * 2);
        let mid = (36 * 128 + 64) * 2; // centro: dentro de la imagen
        let near = |a: u8, b: u8| a.abs_diff(b) <= 4;
        assert!(near(out[mid], 120) && near(out[mid + 1], 90) && near(out[mid + 3], 200), "{:?}", &out[mid..mid + 4]);
        assert_eq!(&out[..4], &[16, 128, 16, 128]); // barra negra lateral
    }

    #[test]
    fn odd_size_is_typed_error() {
        let mut dec = VideoDecoder::new(4).unwrap();
        assert!(dec.next_yuyv(7, 7).unwrap().is_none()); // cola vacía
        dec.push_access_unit(encode_solid(64, 48), 0, true);
        assert!(matches!(dec.next_yuyv(7, 7), Err(DecodeError::Convert(ConvertError::InvalidSize))));
    }
}
