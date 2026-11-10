//! Filtro DirectShow de la cámara virtual de OpenCam (Windows).
//!
//! Es un filtro fuente con un pin de salida de vídeo YUY2 @ 30 fps (1080p, 720p o 480p;
//! escala con letterbox si la memoria compartida trae otra resolución).
//! Lee frames de la memoria compartida `Local\OpenCamVirtualCamera`, que escribe
//! el cliente `opencam-desktop` (ver `opencam_core::shared_frame`).
//!
//! Registro: `regsvr32 opencam_vcam.dll` (en una consola elevada) lo añade con
//! `IFilterMapper2` a la categoría "Video Input Device", igual que OBS Virtual Camera,
//! para que Discord, Zoom, OBS, Chrome, Teams, etc. lo listen. Hay que registrar la DLL
//! de 64 bits para las apps de 64 bits y la de 32 bits para las de 32 bits.

mod filter;
mod pin;

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};

use windows::core::{implement, ComObject, IUnknown, Interface, Ref, BOOL, GUID, HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::{
    CLASS_E_NOAGGREGATION, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, E_FAIL, E_POINTER, HMODULE, S_FALSE, S_OK,
};
use windows::Win32::Media::DirectShow::{
    IBaseFilter, IFilterMapper2, MERIT_DO_NOT_USE, REGFILTER2, REGFILTER2_0, REGFILTER2_0_1, REGFILTERPINS2,
    REGPINTYPES, REG_PINFLAG_B_OUTPUT,
};
use windows::Win32::Media::MediaFoundation::{
    CLSID_FilterMapper2, CLSID_VideoInputDeviceCategory, MEDIASUBTYPE_YUY2, MEDIATYPE_Video, PIN_CATEGORY_CAPTURE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IClassFactory, IClassFactory_Impl, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_WRITE,
    REG_OPTION_NON_VOLATILE, REG_SZ,
};
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

/// Lo llama `regsvr32`: registra el servidor COM y el filtro en la categoría de cámaras.
#[no_mangle]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    match register() {
        Ok(()) => S_OK,
        Err(e) => {
            // Deja el registro limpio si se quedó a medias.
            let _ = unregister();
            e.code()
        }
    }
}

/// Lo llama `regsvr32 /u`: borra el filtro de la categoría y el servidor COM.
#[no_mangle]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    match unregister() {
        Ok(()) => S_OK,
        Err(e) => e.code(),
    }
}

/// Ruta del CLSID dentro de `HKLM`. Una DLL de 32 bits escribe sola en la vista de 32 bits
/// (`WOW6432Node`) y una de 64 en la de 64, así que las dos pueden registrarse a la vez con el
/// mismo CLSID: cada app carga la que corresponde a su arquitectura.
fn clsid_key() -> String {
    format!("SOFTWARE\\Classes\\CLSID\\{CLSID_STR}")
}

fn register() -> windows::core::Result<()> {
    let dll = module_path().ok_or(windows::core::Error::from(E_FAIL))?;
    let base = clsid_key();
    reg_set(&base, None, FRIENDLY_NAME)?;
    let server = format!("{base}\\InprocServer32");
    reg_set(&server, None, &dll)?;
    reg_set(&server, Some("ThreadingModel"), "Both")?;
    with_mapper(|mapper| {
        // FilterData con un pin de salida de vídeo YUY2 de categoría captura, como el de una
        // webcam. Algunas apps (y la caché del enumerador de dispositivos) lo leen al listar.
        let pin_types = [REGPINTYPES { clsMajorType: &MEDIATYPE_Video, clsMinorType: &MEDIASUBTYPE_YUY2 }];
        let pins = [REGFILTERPINS2 {
            dwFlags: REG_PINFLAG_B_OUTPUT.0 as u32,
            cInstances: 1,
            nMediaTypes: pin_types.len() as u32,
            lpMediaType: pin_types.as_ptr(),
            nMediums: 0,
            lpMedium: std::ptr::null(),
            clsPinCategory: &PIN_CATEGORY_CAPTURE,
        }];
        let info = REGFILTER2 {
            dwVersion: 2,
            dwMerit: MERIT_DO_NOT_USE.0 as u32,
            Anonymous: REGFILTER2_0 { Anonymous2: REGFILTER2_0_1 { cPins2: pins.len() as u32, rgPins2: pins.as_ptr() } },
        };
        let name = HSTRING::from(FRIENDLY_NAME);
        let instance = HSTRING::from(CLSID_STR);
        // SAFETY: todos los punteros apuntan a datos locales vivos durante la llamada.
        unsafe {
            mapper.RegisterFilter(
                &CLSID_OPENCAM_VCAM,
                &name,
                None,
                &CLSID_VideoInputDeviceCategory,
                &instance,
                &info,
            )
        }
    })
}

fn unregister() -> windows::core::Result<()> {
    let instance = HSTRING::from(CLSID_STR);
    // Si no estaba en la categoría, UnregisterFilter falla; no importa.
    let _ = with_mapper(|mapper| {
        // SAFETY: GUIDs y cadena válidos durante la llamada.
        unsafe { mapper.UnregisterFilter(&CLSID_VideoInputDeviceCategory, &instance, &CLSID_OPENCAM_VCAM) }
    });
    // Versiones anteriores escribían la instancia a mano con reg.exe; se borra también por si acaso.
    let old_instance = format!(
        "SOFTWARE\\Classes\\CLSID\\{VIDEO_INPUT_CATEGORY}\\Instance\\{CLSID_STR}"
    );
    reg_delete(&old_instance)?;
    reg_delete(&clsid_key())
}

/// Ejecuta `f` con un `IFilterMapper2`, iniciando COM si el hilo no lo tenía.
fn with_mapper<T>(f: impl FnOnce(&IFilterMapper2) -> windows::core::Result<T>) -> windows::core::Result<T> {
    // SAFETY: inicialización de COM del hilo actual; sólo se deshace si la hizo esta llamada.
    let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let result = (|| {
        // SAFETY: CLSID_FilterMapper2 es un servidor COM del sistema (quartz.dll).
        let mapper: IFilterMapper2 = unsafe { CoCreateInstance(&CLSID_FilterMapper2, None, CLSCTX_INPROC_SERVER) }?;
        f(&mapper)
    })();
    if init.is_ok() {
        // SAFETY: equilibra el CoInitializeEx que tuvo éxito arriba.
        unsafe { CoUninitialize() };
    }
    result
}

/// Escribe un valor de cadena en `HKLM\{path}` (el valor predeterminado si `name` es `None`).
fn reg_set(path: &str, name: Option<&str>, value: &str) -> windows::core::Result<()> {
    let mut key = HKEY::default();
    let path = HSTRING::from(path);
    // SAFETY: `key` recibe el handle y se cierra abajo.
    unsafe {
        RegCreateKeyExW(HKEY_LOCAL_MACHINE, &path, None, None, REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None)
    }
    .ok()?;
    let data: Vec<u8> = value.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect();
    let name = name.map(HSTRING::from);
    let name_ptr = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
    // SAFETY: `key` es válido; `data` es una cadena UTF-16 terminada en nulo.
    let result = unsafe { RegSetValueExW(key, name_ptr, None, REG_SZ, Some(&data)) }.ok();
    // SAFETY: handle abierto arriba.
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

/// Borra `HKLM\{path}` con todo su contenido; que no exista no es un error.
fn reg_delete(path: &str) -> windows::core::Result<()> {
    let path = HSTRING::from(path);
    // SAFETY: cadena válida durante la llamada.
    let err = unsafe { RegDeleteTreeW(HKEY_LOCAL_MACHINE, &path) };
    if err == ERROR_SUCCESS || err == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        err.ok()
    }
}

/// Ruta completa de esta DLL, necesaria para `InprocServer32`.
fn module_path() -> Option<String> {
    let module = HMODULE(MODULE.load(Ordering::SeqCst) as *mut c_void);
    let mut buf = [0u16; 1024];
    // SAFETY: `buf` es válido durante la llamada.
    let len = unsafe { GetModuleFileNameW(Some(module), &mut buf) } as usize;
    if len == 0 { None } else { Some(String::from_utf16_lossy(&buf[..len])) }
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
        let obj = ComObject::new(filter::Filter::new());
        obj.bind_pin(&obj.to_interface::<IBaseFilter>());
        let unk: IUnknown = obj.to_interface();
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
