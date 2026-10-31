//! Filtro DirectShow de la cámara virtual de OpenCam (Windows).
//!
//! Es un filtro fuente con un pin de salida de vídeo YUY2 1920x1080 @ 30 fps.
//! Lee frames de la memoria compartida `Local\OpenCamVirtualCamera`, que escribe
//! el cliente `opencam-desktop` (ver `opencam_core::shared_frame`).
//!
//! Registro: `regsvr32 opencam_vcam.dll` (en una consola elevada) lo añade a la
//! categoría "Video Input Device" para que OBS, Teams clásico, etc. lo listen.
//! SIN COMPILAR NI PROBAR EN WINDOWS TODAVÍA.

mod filter;
mod pin;

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};

use windows::core::{implement, IUnknown, Interface, Ref, BOOL, GUID, HRESULT};
use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_POINTER, HMODULE, S_FALSE, S_OK};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;

/// CLSID del filtro de OpenCam (generado para este proyecto).
pub const CLSID_OPENCAM_VCAM: GUID = GUID::from_u128(0x6f1c2a4e_8b3d_4e2a_9c51_0d7a3b6e9f21);
/// Categoría CLSID_VideoInputDeviceCategory: donde las aplicaciones buscan cámaras.
const VIDEO_INPUT_CATEGORY: &str = "{860BB310-5D01-11D0-BD3B-00A0C911CE86}";
const CLSID_STR: &str = "{6F1C2A4E-8B3D-4E2A-9C51-0D7A3B6E9F21}";
const FRIENDLY_NAME: &str = "OpenCam Virtual Camera";

/// Handle del módulo DLL, fijado en `DllMain` y usado para localizar la propia DLL al registrarla.
static MODULE: AtomicIsize = AtomicIsize::new(0);
/// Número de objetos vivos (y bloqueos de servidor); `DllCanUnloadNow` lo consulta.
static LOCKS: AtomicU32 = AtomicU32::new(0);

#[no_mangle]
pub extern "system" fn DllMain(hinst: HMODULE, reason: u32, _reserved: *mut c_void) -> BOOL {
    // DLL_PROCESS_ATTACH = 1
    if reason == 1 {
        MODULE.store(hinst.0 as isize, Ordering::SeqCst);
    }
    BOOL::from(true)
}

/// Llamada por el sistema de COM para crear la fábrica del filtro.
#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() || rclsid.is_null() || riid.is_null() {
        return E_POINTER;
    }
    *ppv = std::ptr::null_mut();
    if *rclsid != CLSID_OPENCAM_VCAM {
        return windows::Win32::Foundation::CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IClassFactory = Factory.into();
    factory.query(riid, ppv)
}

#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if LOCKS.load(Ordering::SeqCst) == 0 { S_OK } else { S_FALSE }
}

/// Lo llama `regsvr32`: escribe las claves de registro del filtro.
#[no_mangle]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    match register() {
        Ok(()) => S_OK,
        Err(_) => HRESULT(-2147467259), // E_FAIL
    }
}

/// Lo llama `regsvr32 /u`: borra las claves de registro del filtro.
#[no_mangle]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    let clsid = CLSID_STR;
    let _ = reg(&["delete", &format!("HKCR\\CLSID\\{clsid}"), "/f"]);
    let _ = reg(&["delete", &format!("HKCR\\CLSID\\{VIDEO_INPUT_CATEGORY}\\Instance\\{clsid}"), "/f"]);
    S_OK
}

fn register() -> Result<(), String> {
    let dll = module_path().ok_or("no se pudo leer la ruta de la DLL")?;
    let clsid = CLSID_STR;
    let base = format!("HKCR\\CLSID\\{clsid}");
    let instance = format!("HKCR\\CLSID\\{VIDEO_INPUT_CATEGORY}\\Instance\\{clsid}");
    reg(&["add", &base, "/ve", "/d", FRIENDLY_NAME, "/f"])?;
    reg(&["add", &format!("{base}\\InprocServer32"), "/ve", "/d", &dll, "/f"])?;
    reg(&["add", &format!("{base}\\InprocServer32"), "/v", "ThreadingModel", "/d", "Both", "/f"])?;
    reg(&["add", &instance, "/v", "FriendlyName", "/d", FRIENDLY_NAME, "/f"])?;
    reg(&["add", &instance, "/v", "CLSID", "/d", &clsid, "/f"])?;
    Ok(())
}

/// Ruta completa de esta DLL, necesaria para `InprocServer32`.
fn module_path() -> Option<String> {
    let module = HMODULE(MODULE.load(Ordering::SeqCst) as *mut c_void);
    let mut buf = [0u16; 1024];
    // SAFETY: `buf` es válido durante la llamada.
    let len = unsafe { GetModuleFileNameW(Some(module), &mut buf) } as usize;
    if len == 0 { None } else { Some(String::from_utf16_lossy(&buf[..len])) }
}

/// Ejecuta `reg.exe` con los argumentos dados; devuelve error si el comando falla.
fn reg(args: &[&str]) -> Result<(), String> {
    let status = std::process::Command::new("reg")
        .args(args)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("reg {:?} falló", args)) }
}

/// Fábrica de clases: crea instancias del filtro.
#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Ref<IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut c_void,
    ) -> windows::core::Result<()> {
        if !punkouter.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        if ppvobject.is_null() {
            return Err(E_POINTER.into());
        }
        let unk: IUnknown = filter::Filter::new().into();
        // SAFETY: `ppvobject` se comprobó no nulo arriba y COM exige que apunte a memoria válida.
        unsafe { unk.query(riid, ppvobject) }.ok()
    }

    fn LockServer(&self, flock: BOOL) -> windows::core::Result<()> {
        if flock.as_bool() {
            LOCKS.fetch_add(1, Ordering::SeqCst);
        } else {
            LOCKS.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

pub(crate) fn lock() {
    LOCKS.fetch_add(1, Ordering::SeqCst);
}

pub(crate) fn unlock() {
    LOCKS.fetch_sub(1, Ordering::SeqCst);
}
