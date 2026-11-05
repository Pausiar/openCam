//! Conversión directa I420 -> YUYV con escalado (vecino más cercano) y letterbox negro.

use openh264::formats::YUVSource;

#[derive(Debug, PartialEq, Eq)]
pub enum ConvertError {
    /// El tamaño de salida debe ser par y no nulo (YUYV comparte croma entre 2 píxeles).
    InvalidSize,
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("tamaño de salida inválido: ancho y alto deben ser pares y > 0")
    }
}
impl std::error::Error for ConvertError {}

/// Convierte `src` a YUYV de `out_w` x `out_h`, manteniendo la relación de aspecto
/// y rellenando con negro (Y=16, U=V=128) los bordes.
pub fn i420_to_yuyv(
    src: &impl YUVSource,
    out_w: usize,
    out_h: usize,
) -> Result<Vec<u8>, ConvertError> {
    if out_w == 0 || out_h == 0 || !out_w.is_multiple_of(2) || !out_h.is_multiple_of(2) {
        return Err(ConvertError::InvalidSize);
    }
    let (sw, sh) = src.dimensions();
    let (ys, us, vs) = src.strides();
    let (y, u, v) = (src.y(), src.u(), src.v());

    let mut out = Vec::with_capacity(out_w * out_h * 2);
    for _ in 0..out_w * out_h / 2 {
        out.extend_from_slice(&[16, 128, 16, 128]);
    }
    if sw == 0 || sh == 0 {
        return Ok(out);
    }

    // Tamaño escalado que cabe en la salida; ancho par y origen par para alinear pares de píxeles.
    let (mut dw, dh) = if sw * out_h > sh * out_w {
        (out_w, sh * out_w / sw)
    } else {
        (sw * out_h / sh, out_h)
    };
    dw = (dw & !1).max(2);
    let dh = dh.max(1);
    let x0 = ((out_w - dw) / 2) & !1;
    let y0 = (out_h - dh) / 2;

    for dy in 0..dh {
        let sy = dy * sh / dh;
        let row = &mut out[(y0 + dy) * out_w * 2..][..out_w * 2];
        for dx in (0..dw).step_by(2) {
            let (sx0, sx1) = (dx * sw / dw, (dx + 1) * sw / dw);
            let o = (x0 + dx) * 2;
            row[o] = y[sy * ys + sx0];
            row[o + 1] = u[(sy / 2) * us + sx0 / 2];
            row[o + 2] = y[sy * ys + sx1];
            row[o + 3] = v[(sy / 2) * vs + sx0 / 2];
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openh264::formats::YUVBuffer;

    /// I420 plano de un solo color.
    fn solid(w: usize, h: usize, (y, u, v): (u8, u8, u8)) -> YUVBuffer {
        let mut buf = vec![y; w * h];
        buf.extend(vec![u; w * h / 4]);
        buf.extend(vec![v; w * h / 4]);
        YUVBuffer::from_vec(buf, w, h)
    }

    #[test]
    fn same_size_keeps_color() {
        let out = i420_to_yuyv(&solid(8, 4, (100, 90, 200)), 8, 4).unwrap();
        assert_eq!(out.len(), 8 * 4 * 2);
        assert!(out.chunks(4).all(|p| p == [100, 90, 100, 200]));
    }

    #[test]
    fn upscale_16_9_has_exact_size_and_no_bars() {
        let out = i420_to_yuyv(&solid(1280, 720, (100, 90, 200)), 1920, 1080).unwrap();
        assert_eq!(out.len(), 1920 * 1080 * 2);
        assert!(out.chunks(4).all(|p| p == [100, 90, 100, 200]));
    }

    #[test]
    fn letterbox_adds_black_bars() {
        // 4:3 en 16:9 -> barras laterales.
        let out = i420_to_yuyv(&solid(64, 48, (100, 90, 200)), 128, 72).unwrap();
        assert_eq!(&out[..4], &[16, 128, 16, 128]);
        let mid = (36 * 128 + 64) * 2;
        assert_eq!(&out[mid..mid + 4], &[100, 90, 100, 200]);
        // 16:9 en 4:3 -> barras arriba/abajo.
        let out = i420_to_yuyv(&solid(128, 72, (100, 90, 200)), 64, 48).unwrap();
        assert_eq!(&out[..4], &[16, 128, 16, 128]);
        let mid = (24 * 64 + 32) * 2;
        assert_eq!(&out[mid..mid + 4], &[100, 90, 100, 200]);
    }

    #[test]
    fn odd_output_rejected() {
        assert_eq!(
            i420_to_yuyv(&solid(8, 4, (0, 0, 0)), 7, 4),
            Err(ConvertError::InvalidSize)
        );
    }
}
