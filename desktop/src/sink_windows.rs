//! Cámara virtual en Windows: publica cada frame YUYV en una memoria compartida
//! con nombre. El filtro DirectShow (`windows-vcam/`, DLL registrada con regsvr32)
//! la lee desde las aplicaciones de vídeo (OBS, Teams clásico, etc.).
//!
//! Formato de la región: `opencam_core::shared_frame`.
//! Sin compilar ni probar en Windows todavía; ver desktop/README.md.

use std::io;

use opencam_core::shared_frame;
use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_ALL_ACCESS, MEMORY_MAPPED_VIEW_ADDRESS,
    PAGE_READWRITE,
};

/// Nombre de la región compartida; debe coincidir con `windows-vcam/src/lib.rs`.
pub const SHARED_NAME: windows::core::PCWSTR = w!("Local\\OpenCamVirtualCamera");

pub struct VirtualCamera {
    mapping: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    len: usize,
    width: u32,
    height: u32,
    seq: u32,
}

impl VirtualCamera {
    /// Crea (o reutiliza) la región compartida para frames de `width` x `height`.
    pub fn open(width: u32, height: u32) -> io::Result<Self> {
        let len = shared_frame::region_len(width, height);
        // SAFETY: nombre y tamaño válidos; el handle se libera en `Drop`.
        let mapping = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                len as u32,
                SHARED_NAME,
            )
        }
        .map_err(|e| io::Error::other(e.to_string()))?;
        // SAFETY: `mapping` es un handle válido de tamaño `len`.
        let view = unsafe { MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, len) };
        if view.Value.is_null() {
            // SAFETY: handle creado arriba, aún no está en uso.
            unsafe { let _ = CloseHandle(mapping); }
            return Err(io::Error::last_os_error());
        }
        Ok(Self { mapping, view, len, width, height, seq: 0 })
    }

    /// Publica un frame YUYV (2 bytes por píxel) para que lo lea el filtro.
    pub fn write_yuyv(&mut self, yuyv: &[u8]) -> io::Result<()> {
        // SAFETY: la vista mapea exactamente `len` bytes y sigue viva hasta `Drop`.
        let region = unsafe { std::slice::from_raw_parts_mut(self.view.Value as *mut u8, self.len) };
        self.seq = self.seq.wrapping_add(1);
        if shared_frame::write_frame(region, self.width, self.height, self.seq, yuyv) {
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::InvalidInput, "tamaño de frame incorrecto"))
        }
    }
}

impl Drop for VirtualCamera {
    fn drop(&mut self) {
        // SAFETY: vista y handle obtenidos en `open`; se liberan una sola vez.
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.mapping);
        }
    }
}

impl crate::sink::FrameSink for VirtualCamera {
    fn write_yuyv(&mut self, yuyv: &[u8]) -> io::Result<()> {
        VirtualCamera::write_yuyv(self, yuyv)
    }
}
