//! Cámara virtual en Linux: escribe frames en un dispositivo `v4l2loopback` (/dev/videoN).
//!
//! Requiere el módulo del kernel `v4l2loopback` cargado (ver README de desktop o
//! `desktop/linux/install.sh`). El formato de salida es YUYV 4:2:2, que todas las
//! aplicaciones de vídeo aceptan.
//!
//! Con `--device auto` (por defecto) se busca en `/sys/class/video4linux` el dispositivo
//! cuyo nombre contiene `OpenCam` (el `card_label` del instalador); si no hay ninguno, el
//! primer v4l2loopback que admita salida de vídeo.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::io::AsRawFd;
use std::path::Path;

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

/// Mismo layout que `struct v4l2_streamparm` con `struct v4l2_outputparm` (204 bytes).
#[repr(C)]
struct StreamParm {
    type_: u32,
    capability: u32,
    outputmode: u32,
    /// `timeperframe` (v4l2_fract): segundos por frame = numerator / denominator.
    numerator: u32,
    denominator: u32,
    extendedmode: u32,
    writebuffers: u32,
    reserved: [u32; 4],
    _rest: [u8; 160],
}

// VIDIOC_S_PARM = _IOWR('V', 22, struct v4l2_streamparm)
const VIDIOC_S_PARM: libc::c_ulong = 0xC0CC_5616;

/// Nombre que pone el instalador (`card_label=OpenCam`).
const CARD_LABEL: &str = "OpenCam";

/// Resuelve `auto` a un `/dev/videoN`; cualquier otra ruta se devuelve tal cual.
pub fn resolve_device(device: &str) -> io::Result<String> {
    if device != "auto" {
        return Ok(device.to_string());
    }
    let mut nodes: Vec<(u32, String)> = fs::read_dir("/sys/class/video4linux")
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().into_string().ok()?;
                    let n = name.strip_prefix("video")?.parse().ok()?;
                    let label = fs::read_to_string(e.path().join("name")).unwrap_or_default();
                    Some((n, label.trim().to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    nodes.sort();
    if let Some(dev) = pick_device(&nodes, is_output_device) {
        return Ok(dev);
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "no hay ninguna cámara virtual v4l2loopback. Instálala con \
         `sudo desktop/linux/install.sh` (o `sudo modprobe v4l2loopback video_nr=10 \
         card_label=OpenCam exclusive_caps=1`) o indica una con --device /dev/videoN",
    ))
}

/// Prefiere el nodo llamado OpenCam; si no, el primero que admita salida de vídeo.
fn pick_device(nodes: &[(u32, String)], is_output: impl Fn(&str) -> bool) -> Option<String> {
    let path = |n: u32| format!("/dev/video{n}");
    nodes
        .iter()
        .find(|(_, label)| label.contains(CARD_LABEL))
        .map(|(n, _)| path(*n))
        .or_else(|| nodes.iter().map(|(n, _)| path(*n)).find(|p| is_output(p)))
}

fn is_output_device(path: &str) -> bool {
    if !Path::new(path).exists() {
        return false;
    }
    let Ok(file) = OpenOptions::new().write(true).open(path) else {
        return false;
    };
    query_caps(&file).is_ok_and(|c| c & V4L2_CAP_VIDEO_OUTPUT != 0)
}

fn query_caps(file: &File) -> io::Result<u32> {
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
        return Err(io::Error::last_os_error());
    }
    Ok(if cap.capabilities & V4L2_CAP_DEVICE_CAPS != 0 {
        cap.device_caps
    } else {
        cap.capabilities
    })
}

pub struct VirtualCamera {
    file: File,
    width: u32,
    height: u32,
}

impl VirtualCamera {
    /// Abre `/dev/videoN` (o `auto`) y configura salida YUYV de `width` x `height` a `fps`.
    pub fn open(device: &str, width: u32, height: u32, fps: u32) -> io::Result<Self> {
        let device = resolve_device(device)?;
        let device = device.as_str();
        let file = OpenOptions::new().write(true).open(device).map_err(|e| {
            let hint = if e.kind() == io::ErrorKind::PermissionDenied {
                " (añade tu usuario al grupo `video` y vuelve a iniciar sesión)"
            } else {
                ""
            };
            io::Error::new(e.kind(), format!("no se pudo abrir {device}: {e}{hint}"))
        })?;

        let caps = query_caps(&file)
            .map_err(|e| io::Error::other(format!("{device} no es un dispositivo V4L2: {e}")))?;
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

        // Frecuencia anunciada a las apps. No es fatal: módulos antiguos no la admiten.
        let mut parm = StreamParm {
            type_: V4L2_BUF_TYPE_VIDEO_OUTPUT,
            capability: 0,
            outputmode: 0,
            numerator: 1,
            denominator: fps.max(1),
            extendedmode: 0,
            writebuffers: 0,
            reserved: [0; 4],
            _rest: [0; 160],
        };
        // SAFETY: `parm` vive durante la llamada y su layout coincide con v4l2_streamparm.
        if unsafe {
            libc::ioctl(
                file.as_raw_fd(),
                VIDIOC_S_PARM,
                &mut parm as *mut StreamParm,
            )
        } < 0
        {
            eprintln!(
                "Aviso: {device} no aceptó {fps} fps: {}",
                io::Error::last_os_error()
            );
        }
        eprintln!("Cámara virtual: {device} ({width}x{height} YUYV a {fps} fps)");
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
        assert_eq!(std::mem::size_of::<StreamParm>(), 204);
    }

    #[test]
    fn picks_opencam_label_first() {
        let nodes = vec![
            (0, "Integrated Camera".to_string()),
            (2, "Dummy video device".to_string()),
            (10, "OpenCam".to_string()),
        ];
        assert_eq!(
            pick_device(&nodes, |_| true).as_deref(),
            Some("/dev/video10")
        );
        let nodes = vec![(0, "Integrated Camera".into()), (3, "Loop".into())];
        assert_eq!(
            pick_device(&nodes, |p| p == "/dev/video3").as_deref(),
            Some("/dev/video3")
        );
        assert_eq!(pick_device(&nodes, |_| false), None);
    }

    #[test]
    fn explicit_device_is_kept() {
        assert_eq!(resolve_device("/dev/video7").unwrap(), "/dev/video7");
    }
}
