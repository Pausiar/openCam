//! Filtro DirectShow de OpenCam: IBaseFilter con un único pin de salida de vídeo.
//! Run arranca la entrega de frames y Stop la detiene.

use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex};

use windows::core::{implement, Interface, PCWSTR, PWSTR, GUID, HRESULT, Ref};
use windows::Win32::Foundation::{E_NOTIMPL, S_FALSE, S_OK};
use windows::Win32::Media::DirectShow::{
    IBaseFilter, IBaseFilter_Impl, IEnumPins, IEnumPins_Impl, IFilterGraph, IMediaFilter_Impl, IPin, FILTER_INFO,
    FILTER_STATE, State_Running, State_Stopped,
};
use windows::Win32::Media::IReferenceClock;
use windows::Win32::System::Com::IPersist_Impl;

use crate::pin::{name128, OutPin, Shared};

const VFW_E_NOT_FOUND: HRESULT = HRESULT(0x8004_0216u32 as i32);

/// Filtro de OpenCam. Se crea desde la clase factory (`IClassFactory::CreateInstance`).
#[implement(IBaseFilter)]
pub struct Filter {
    shared: Arc<Shared>,
    pin: IPin,
    /// `IFilterGraph` SIN contar referencia (convención de DirectShow: el grafo posee al filtro,
    /// no al revés). `JoinFilterGraph(NULL)` lo borra antes de que el grafo se libere.
    graph: AtomicPtr<c_void>,
}

impl Filter {
    pub fn new() -> Self {
        crate::lock();
        let shared = Shared::new();
        let pin: IPin = OutPin::new(shared.clone()).into();
        Self { shared, pin, graph: AtomicPtr::new(ptr::null_mut()) }
    }
}

impl Filter {
    /// Fija la referencia débil (sin AddRef) del pin a este filtro; `iface` es su `IBaseFilter`.
    pub fn bind_pin(&self, iface: &IBaseFilter) {
        self.shared.set_filter(iface.as_raw());
    }
}

impl Drop for Filter {
    fn drop(&mut self) {
        self.shared.set_filter(ptr::null_mut());
        self.shared.stop();
        crate::unlock();
    }
}

impl IPersist_Impl for Filter_Impl {
    fn GetClassID(&self) -> windows::core::Result<GUID> {
        Ok(crate::CLSID_OPENCAM_VCAM)
    }
}

impl IMediaFilter_Impl for Filter_Impl {
    fn Stop(&self) -> windows::core::Result<()> {
        self.shared.stop();
        Ok(())
    }

    fn Pause(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn Run(&self, _tstart: i64) -> windows::core::Result<()> {
        self.shared.start();
        Ok(())
    }

    fn GetState(&self, _dwmilliseconds: u32) -> windows::core::Result<FILTER_STATE> {
        Ok(if self.shared.is_running() { State_Running } else { State_Stopped })
    }

    fn SetSyncSource(&self, _pclock: Ref<IReferenceClock>) -> windows::core::Result<()> {
        Ok(())
    }

    fn GetSyncSource(&self) -> windows::core::Result<IReferenceClock> {
        Err(E_NOTIMPL.into())
    }
}

impl IBaseFilter_Impl for Filter_Impl {
    fn EnumPins(&self) -> windows::core::Result<IEnumPins> {
        Ok(EnumPins { pin: self.pin.clone(), index: Mutex::new(0) }.into())
    }

    fn FindPin(&self, id: &PCWSTR) -> windows::core::Result<IPin> {
        // SAFETY: el llamador pasa una cadena terminada en nulo válida.
        let name = unsafe { id.to_string() }.ok();
        if name.as_deref() == Some("Video") {
            Ok(self.pin.clone())
        } else {
            Err(VFW_E_NOT_FOUND.into())
        }
    }

    fn QueryFilterInfo(&self, pinfo: *mut FILTER_INFO) -> windows::core::Result<()> {
        let raw = self.graph.load(Ordering::Acquire);
        // SAFETY: `raw` es nulo o el grafo que nos unió y aún no hizo JoinFilterGraph(NULL); el clon cuenta referencia para el llamador.
        let graph = unsafe { IFilterGraph::from_raw_borrowed(&raw) }.cloned();
        let info = FILTER_INFO {
            achName: name128("OpenCam Virtual Camera"),
            pGraph: ManuallyDrop::new(graph),
        };
        // SAFETY: `pinfo` lo proporciona el llamador.
        unsafe { ptr::write(pinfo, info) };
        Ok(())
    }

    fn JoinFilterGraph(&self, pgraph: Ref<IFilterGraph>, _pname: &PCWSTR) -> windows::core::Result<()> {
        // Un grafo nulo significa que el filtro se retira del grafo.
        let raw = pgraph.ok().ok().map_or(ptr::null_mut(), |g| g.as_raw());
        self.graph.store(raw, Ordering::Release);
        Ok(())
    }

    fn QueryVendorInfo(&self) -> windows::core::Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }
}

/// Enumerador con el único pin del filtro.
#[implement(IEnumPins)]
pub struct EnumPins {
    pin: IPin,
    index: Mutex<u32>,
}

impl IEnumPins_Impl for EnumPins_Impl {
    fn Next(&self, cpins: u32, ppins: *mut Option<IPin>, pcfetched: *mut u32) -> HRESULT {
        let mut idx = self.index.lock().unwrap();
        let mut fetched = 0u32;
        if cpins > 0 && *idx == 0 && !ppins.is_null() {
            // SAFETY: el llamador reserva `cpins` posiciones; `ptr::write` evita liberar basura.
            unsafe { ptr::write(ppins, Some(self.pin.clone())) };
            *idx = 1;
            fetched = 1;
        }
        if !pcfetched.is_null() {
            // SAFETY: `pcfetched` lo proporciona el llamador.
            unsafe { *pcfetched = fetched };
        }
        if fetched == cpins { S_OK } else { S_FALSE }
    }

    fn Skip(&self, cpins: u32) -> windows::core::Result<()> {
        *self.index.lock().unwrap() += cpins;
        Ok(())
    }

    fn Reset(&self) -> windows::core::Result<()> {
        *self.index.lock().unwrap() = 0;
        Ok(())
    }

    fn Clone(&self) -> windows::core::Result<IEnumPins> {
        Ok(EnumPins { pin: self.pin.clone(), index: Mutex::new(*self.index.lock().unwrap()) }.into())
    }
}
