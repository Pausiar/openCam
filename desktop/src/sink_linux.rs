//! Cámara virtual en Linux: escribe frames en un dispositivo `v4l2loopback` (/dev/videoN).
//!
//! Requiere el módulo del kernel `v4l2loopback` cargado (ver README de desktop).
//! El formato de salida es YUYV 4:2:2, que todas las aplicaciones de vídeo aceptan.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::io::AsRawFd;

const V4L2_PIX_FMT_YUYV: u32 = u32::from_le_bytes(*b"YUYV");
const V4L2_BUF_TYPE_VIDEO_OUTPUT: u32 = 2;
const V4L2_FIELD_NONE: u32 = 1;

/// Mismo layout que `struct v4l2_pix_format` (linux/videodev2.h).
#[repr(C)]
#[derive(Default)]
struct PixFormat {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    bytesperline: u32,
    sizeimage: u32,
    colorspace: u32,
    priv_: u32,
    flags: u32,
    ycbcr_enc: u32,
    quantization: u32,
    xfer_func: u32,
}

/// Mismo layout que `struct v4l2_format` en 64 bits (union de 200 bytes, alineada a 8).
#[repr(C)]
struct Format {
    type_: u32,
    _pad: u32,
    fmt: [u8; 200],
}

// VIDIOC_S_FMT = _IOWR('V', 5, struct v4l2_format)
const VIDIOC_S_FMT: libc::c_ulong = 0xC0D0_5605;

pub struct VirtualCamera {
    file: File,
    width: u32,
    height: u32,
}

impl VirtualCamera {
    /// Abre `/dev/videoN` y configura el formato de salida YUYV de `width` x `height`.
    pub fn open(device: &str, width: u32, height: u32) -> io::Result<Self> {
        let file = OpenOptions::new().write(true).open(device)?;

        let mut pix = PixFormat {
            width,
            height,
            pixelformat: V4L2_PIX_FMT_YUYV,
            field: V4L2_FIELD_NONE,
            bytesperline: width * 2,
            sizeimage: width * height * 2,
            ..Default::default()
        };
        let mut fmt = Format { type_: V4L2_BUF_TYPE_VIDEO_OUTPUT, _pad: 0, fmt: [0u8; 200] };
        // SAFETY: `PixFormat` es `repr(C)` y cabe en el campo `fmt` de `Format`.
        unsafe {
            std::ptr::copy_nonoverlapping(
                &mut pix as *mut PixFormat as *const u8,
                fmt.fmt.as_mut_ptr(),
                std::mem::size_of::<PixFormat>(),
            );
        }
        // SAFETY: `fmt` vive durante la llamada y su layout coincide con v4l2_format.
        let rc = unsafe { libc::ioctl(file.as_raw_fd(), VIDIOC_S_FMT, &mut fmt as *mut Format) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { file, width, height })
    }

    /// Escribe un frame ya convertido a YUYV (2 bytes por píxel).
    pub fn write_yuyv(&mut self, yuyv: &[u8]) -> io::Result<()> {
        let expected = (self.width * self.height * 2) as usize;
        if yuyv.len() != expected {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "tamaño de frame incorrecto"));
        }
        self.file.write_all(yuyv)
    }
}
