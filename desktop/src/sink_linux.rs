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

/// Mismo layout que `struct v4l2_capability` (104 bytes).
#[repr(C)]
#[derive(Default)]
struct Capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}

const V4L2_CAP_VIDEO_OUTPUT: u32 = 0x0000_0002;
const V4L2_CAP_DEVICE_CAPS: u32 = 0x8000_0000;
// VIDIOC_QUERYCAP = _IOR('V', 0, struct v4l2_capability)
const VIDIOC_QUERYCAP: libc::c_ulong = 0x8068_5600;

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

        let mut cap = Capability::default();
        // SAFETY: `cap` vive durante la llamada y su layout coincide con v4l2_capability.
        if unsafe {
            libc::ioctl(
                file.as_raw_fd(),
                VIDIOC_QUERYCAP,
                &mut cap as *mut Capability,
            )
        } < 0
        {
            return Err(io::Error::other(format!(
                "{device} no es un dispositivo V4L2: {}",
                io::Error::last_os_error()
            )));
        }
        let caps = if cap.capabilities & V4L2_CAP_DEVICE_CAPS != 0 {
            cap.device_caps
        } else {
            cap.capabilities
        };
        if caps & V4L2_CAP_VIDEO_OUTPUT == 0 {
            return Err(io::Error::other(format!(
                "{device} no admite salida de vídeo (V4L2_CAP_VIDEO_OUTPUT): no es un v4l2loopback; \
                 cárgalo con `modprobe v4l2loopback` y usa su /dev/videoN"
            )));
        }

        let mut pix = PixFormat {
            width,
            height,
            pixelformat: V4L2_PIX_FMT_YUYV,
            field: V4L2_FIELD_NONE,
            bytesperline: width * 2,
            sizeimage: width * height * 2,
            ..Default::default()
        };
        let mut fmt = Format {
            type_: V4L2_BUF_TYPE_VIDEO_OUTPUT,
            _pad: 0,
            fmt: [0u8; 200],
        };
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
        Ok(Self {
            file,
            width,
            height,
        })
    }

    /// Escribe un frame ya convertido a YUYV (2 bytes por píxel).
    pub fn write_yuyv(&mut self, yuyv: &[u8]) -> io::Result<()> {
        let expected = (self.width * self.height * 2) as usize;
        if yuyv.len() != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "tamaño de frame incorrecto",
            ));
        }
        self.file.write_all(yuyv)
    }
}

impl crate::sink::FrameSink for VirtualCamera {
    fn write_yuyv(&mut self, yuyv: &[u8]) -> io::Result<()> {
        VirtualCamera::write_yuyv(self, yuyv)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_struct_sizes() {
        assert_eq!(std::mem::size_of::<Format>(), 208);
        assert_eq!(std::mem::size_of::<Capability>(), 104);
        assert!(std::mem::size_of::<PixFormat>() <= 200);
    }
}
