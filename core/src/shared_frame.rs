//! Formato de la memoria compartida entre el cliente de escritorio (escritor)
//! y el filtro DirectShow de Windows (lector).
//!
//! Disposición (little-endian):
//! - 0..4   magic `OCVC`
//! - 4..8   ancho en píxeles (u32)
//! - 8..12  alto en píxeles (u32)
//! - 12..16 contador de versión `seq` (u32): impar mientras se escribe, par al terminar
//! - 16..64 reservado
//! - 64..   píxeles YUYV (2 bytes por píxel)
//!
//! Es un seqlock: el lector repite la copia si `seq` cambió o es impar. `seq` se accede como
//! `AtomicU32` (Release al publicar, Acquire al leer), así que la región debe estar alineada a 4
//! bytes (una vista de memoria compartida de Windows lo está). Ancho y alto se escriben dentro
//! de la sección crítica, así que el escritor puede cambiar de resolución.

use std::sync::atomic::{fence, AtomicU32, Ordering};

pub const MAGIC: [u8; 4] = *b"OCVC";
pub const HEADER_LEN: usize = 64;
const SEQ_OFFSET: usize = 12;

/// Tamaño total de la región compartida para un frame YUYV de `width` x `height`.
pub fn region_len(width: u32, height: u32) -> usize {
    HEADER_LEN + (width as usize) * (height as usize) * 2
}

/// `seq` como atómico sobre la propia región; `None` si no hay cabecera o no está alineada.
fn seq_atomic(region: &[u8]) -> Option<&AtomicU32> {
    let p = region.get(SEQ_OFFSET..SEQ_OFFSET + 4)?.as_ptr();
    if !(p as usize).is_multiple_of(4) {
        return None;
    }
    // SAFETY: `p` está alineado, dentro de `region` y vivo mientras dure el préstamo; todo acceso
    // concurrente a esos 4 bytes pasa por este atómico.
    Some(unsafe { AtomicU32::from_ptr(p as *mut u32) })
}

/// Escribe cabecera y píxeles en `region` con el protocolo seqlock.
/// Devuelve `false` si el tamaño de `region` o de `yuyv` no coincide, o si `region` no está alineada a 4.
pub fn write_frame(region: &mut [u8], width: u32, height: u32, seq: u32, yuyv: &[u8]) -> bool {
    let pixels = (width as usize) * (height as usize) * 2;
    if region.len() != HEADER_LEN + pixels || yuyv.len() != pixels {
        return false;
    }
    region[0..4].copy_from_slice(&MAGIC);
    let base = region.as_mut_ptr();
    if !(base as usize).is_multiple_of(4) {
        return false;
    }
    // SAFETY: `base + 12` está alineado y dentro de `region`; los accesos a esos bytes son atómicos.
    let seq_a = unsafe { AtomicU32::from_ptr(base.add(SEQ_OFFSET) as *mut u32) };
    // Marca "escribiendo" (impar); el fence impide que los datos suban por encima de ella.
    seq_a.store(seq.wrapping_mul(2).wrapping_add(1), Ordering::Relaxed);
    fence(Ordering::Release);
    // shortcut: copia no atómica; un lector concurrente puede ver bytes a medias pero lo descarta
    // por `seq`. Cambiar a copias atómicas por palabra si se exige ausencia total de data race.
    // SAFETY: todos los rangos están dentro de `region` (longitud comprobada arriba).
    unsafe {
        std::ptr::copy_nonoverlapping(width.to_le_bytes().as_ptr(), base.add(4), 4);
        std::ptr::copy_nonoverlapping(height.to_le_bytes().as_ptr(), base.add(8), 4);
        std::ptr::copy_nonoverlapping(yuyv.as_ptr(), base.add(HEADER_LEN), pixels);
    }
    seq_a.store(seq.wrapping_mul(2).wrapping_add(2), Ordering::Release);
    true
}

/// Lee la versión actual de la cabecera (`seq`), o `None` si la región no es válida.
pub fn read_seq(region: &[u8]) -> Option<u32> {
    if region.len() < HEADER_LEN || region[0..4] != MAGIC {
        return None;
    }
    Some(seq_atomic(region)?.load(Ordering::Acquire))
}

/// Copia el último frame completo a `out` (se redimensiona) y devuelve `(ancho, alto)`.
/// `None` si no hay frame válido o el escritor no deja de escribir tras unos reintentos.
pub fn read_frame(region: &[u8], out: &mut Vec<u8>) -> Option<(u32, u32)> {
    if region.len() < HEADER_LEN || region[0..4] != MAGIC {
        return None;
    }
    let seq_a = seq_atomic(region)?;
    for _ in 0..4 {
        let before = seq_a.load(Ordering::Acquire);
        if before % 2 == 1 {
            std::hint::spin_loop();
            continue;
        }
        let w = u32::from_le_bytes(region[4..8].try_into().ok()?);
        let h = u32::from_le_bytes(region[8..12].try_into().ok()?);
        // Con valores rasgados por un escritor concurrente, `seq` cambiará y se reintenta.
        let pixels = (w as usize)
            .checked_mul(h as usize)
            .and_then(|n| n.checked_mul(2));
        let Some(pixels) = pixels.filter(|&n| n > 0 && HEADER_LEN + n <= region.len()) else {
            continue;
        };
        out.resize(pixels, 0);
        out.copy_from_slice(&region[HEADER_LEN..HEADER_LEN + pixels]);
        fence(Ordering::Acquire);
        if seq_a.load(Ordering::Relaxed) == before {
            return Some((w, h));
        }
    }
    None
}

/// Escala `src` (YUYV `sw`x`sh`) a `dst` (YUYV `dw`x`dh`) conservando la proporción: vecino más
/// cercano y bandas negras (letterbox). Con el mismo tamaño es una copia. Los tamaños deben ser
/// pares en ancho (YUYV) y `src`/`dst` del tamaño exacto; si no, no hace nada y devuelve `false`.
pub fn letterbox_yuyv(src: &[u8], sw: u32, sh: u32, dst: &mut [u8], dw: u32, dh: u32) -> bool {
    let (sw, sh, dw, dh) = (sw as usize, sh as usize, dw as usize, dh as usize);
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || sw % 2 != 0 || dw % 2 != 0 {
        return false;
    }
    if src.len() != sw * sh * 2 || dst.len() != dw * dh * 2 {
        return false;
    }
    if (sw, sh) == (dw, dh) {
        dst.copy_from_slice(src);
        return true;
    }
    // Rectángulo destino con la proporción de origen, ancho par para no partir macropíxeles.
    let (mut fw, mut fh) = if dw * sh <= dh * sw {
        (dw, sh * dw / sw)
    } else {
        (sw * dh / sh, dh)
    };
    fw = (fw & !1).max(2);
    fh = fh.max(1);
    let (ox, oy) = (((dw - fw) / 2) & !1, (dh - fh) / 2);
    for px in dst.chunks_exact_mut(4) {
        px.copy_from_slice(&[16, 128, 16, 128]); // negro YUYV
    }
    for y in 0..fh {
        let srow = &src[(y * sh / fh) * sw * 2..][..sw * 2];
        let drow = &mut dst[((oy + y) * dw + ox) * 2..][..fw * 2];
        for (x, d) in drow.chunks_exact_mut(4).enumerate() {
            let sx = (x * 2 * sw / fw) & !1;
            d.copy_from_slice(&srow[sx * 2..sx * 2 + 4]);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Región alineada a 4 bytes (un `Vec<u8>` no lo garantiza).
    fn aligned(len: usize) -> Vec<u32> {
        vec![0u32; len.div_ceil(4)]
    }
    fn bytes(v: &mut [u32], len: usize) -> &mut [u8] {
        // SAFETY: `v` cubre al menos `len` bytes.
        unsafe { std::slice::from_raw_parts_mut(v.as_mut_ptr() as *mut u8, len) }
    }

    #[test]
    fn write_then_read_seq() {
        let (w, h) = (4u32, 2u32);
        let len = region_len(w, h);
        let mut buf = aligned(len);
        let region = bytes(&mut buf, len);
        let yuyv = vec![7u8; (w * h * 2) as usize];
        assert!(write_frame(region, w, h, 1, &yuyv));
        let seq = read_seq(region).unwrap();
        assert_eq!(
            seq % 2,
            0,
            "al terminar la escritura la versión debe ser par"
        );
        assert_eq!(&region[HEADER_LEN..], &yuyv[..]);
        let mut out = Vec::new();
        assert_eq!(read_frame(region, &mut out), Some((w, h)));
        assert_eq!(out, yuyv);
    }

    #[test]
    fn rejects_wrong_sizes() {
        let mut buf = aligned(region_len(4, 2));
        let region = bytes(&mut buf, region_len(4, 2));
        assert!(!write_frame(region, 4, 2, 0, &[0u8; 3]));
        assert!(read_seq(&[0u8; 8]).is_none());
        assert!(read_frame(&[0u8; 8], &mut Vec::new()).is_none());
    }

    #[test]
    fn read_frame_skips_write_in_progress() {
        let len = region_len(2, 2);
        let mut buf = aligned(len);
        let region = bytes(&mut buf, len);
        assert!(write_frame(region, 2, 2, 0, &[1u8; 8]));
        region[12..16].copy_from_slice(&3u32.to_le_bytes()); // impar: escritura a medias
        assert!(read_frame(region, &mut Vec::new()).is_none());
    }

    #[test]
    fn concurrent_reader_never_sees_torn_frame() {
        let (w, h) = (64u32, 32u32);
        let len = region_len(w, h);
        let mut buf = aligned(len);
        let addr = buf.as_mut_ptr() as usize;
        let writer = std::thread::spawn(move || {
            // SAFETY: `buf` vive hasta el `join` de ambos hilos.
            let region = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, len) };
            for i in 0..2000u32 {
                assert!(write_frame(
                    region,
                    w,
                    h,
                    i,
                    &vec![i as u8; (w * h * 2) as usize]
                ));
            }
        });
        let reader = std::thread::spawn(move || {
            // SAFETY: igual que arriba.
            let region = unsafe { std::slice::from_raw_parts(addr as *const u8, len) };
            let mut out = Vec::new();
            for _ in 0..2000 {
                if read_frame(region, &mut out).is_some() {
                    assert!(out.iter().all(|&b| b == out[0]), "frame rasgado");
                }
            }
        });
        writer.join().unwrap();
        reader.join().unwrap();
        drop(buf);
    }

    #[test]
    fn letterbox_keeps_aspect_and_fills_black() {
        // 4x2 blanco (Y=200) a 8x8: debe ocupar 8x4 centrado, con negro arriba y abajo.
        let src: Vec<u8> = [200u8, 128].repeat(4 * 2);
        let mut dst = vec![0u8; 8 * 8 * 2];
        assert!(letterbox_yuyv(&src, 4, 2, &mut dst, 8, 8));
        let row = |y: usize| &dst[y * 16..(y + 1) * 16];
        assert!(row(0).chunks(2).all(|p| p == [16, 128]));
        assert!(row(3).chunks(2).all(|p| p == [200, 128]));
        assert!(row(7).chunks(2).all(|p| p == [16, 128]));
        // mismo tamaño = copia; tamaños incoherentes = rechazo
        let mut same = vec![0u8; src.len()];
        assert!(letterbox_yuyv(&src, 4, 2, &mut same, 4, 2));
        assert_eq!(same, src);
        assert!(!letterbox_yuyv(&src, 4, 2, &mut dst, 8, 4));
    }
}
