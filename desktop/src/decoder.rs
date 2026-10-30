//! Decodificador H.264 (OpenH264, software) de las unidades Annex-B recibidas por OPCM.
//!
//! Los frames pasan antes por la `FrameQueue` del núcleo, que descarta P-frames
//! hasta el primer keyframe y limita la cola para mantener la latencia baja.

use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
use opencam_core::{Frame, FrameQueue};

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

    pub fn dropped(&self) -> u64 {
        self.queue.dropped()
    }
}
