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
//! Es un seqlock: el lector repite la copia si `seq` cambió o es impar.

pub const MAGIC: [u8; 4] = *b"OCVC";
pub const HEADER_LEN: usize = 64;

/// Tamaño total de la región compartida para un frame YUYV de `width` x `height`.
pub fn region_len(width: u32, height: u32) -> usize {
    HEADER_LEN + (width as usize) * (height as usize) * 2
}

/// Escribe cabecera y píxeles en `region` con el protocolo seqlock.
/// Devuelve `false` si el tamaño de `region` o de `yuyv` no coincide.
pub fn write_frame(region: &mut [u8], width: u32, height: u32, seq: u32, yuyv: &[u8]) -> bool {
    let pixels = (width as usize) * (height as usize) * 2;
    if region.len() != HEADER_LEN + pixels || yuyv.len() != pixels {
        return false;
    }
    region[0..4].copy_from_slice(&MAGIC);
    region[4..8].copy_from_slice(&width.to_le_bytes());
    region[8..12].copy_from_slice(&height.to_le_bytes());
    // Marca "escribiendo" (impar) antes de tocar los píxeles.
    region[12..16].copy_from_slice(&seq.wrapping_mul(2).wrapping_add(1).to_le_bytes());
    region[HEADER_LEN..].copy_from_slice(yuyv);
    region[12..16].copy_from_slice(&seq.wrapping_mul(2).wrapping_add(2).to_le_bytes());
    true
}

/// Lee la versión actual de la cabecera (`seq`), o `None` si la región no es válida.
pub fn read_seq(region: &[u8]) -> Option<u32> {
    if region.len() < HEADER_LEN || region[0..4] != MAGIC {
        return None;
    }
    Some(u32::from_le_bytes(region[12..16].try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_seq() {
        let (w, h) = (4u32, 2u32);
        let mut region = vec![0u8; region_len(w, h)];
        let yuyv = vec![7u8; (w * h * 2) as usize];
        assert!(write_frame(&mut region, w, h, 1, &yuyv));
        let seq = read_seq(&region).unwrap();
        assert_eq!(seq % 2, 0, "al terminar la escritura la versión debe ser par");
        assert_eq!(&region[HEADER_LEN..], &yuyv[..]);
    }

    #[test]
    fn rejects_wrong_sizes() {
        let mut region = vec![0u8; region_len(4, 2)];
        assert!(!write_frame(&mut region, 4, 2, 0, &[0u8; 3]));
        assert!(read_seq(&[0u8; 8]).is_none());
    }
}
