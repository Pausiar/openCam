//! Pin de salida del filtro: tipos de medio YUY2 1080p/720p/480p a 30 fps (`IAMStreamConfig`),
//! categoría de pin (`IKsPropertySet`) y entrega de muestras. Los frames llegan de la memoria
//! compartida (`opencam_core::shared_frame`) y se escalan con letterbox al formato negociado;
//! si no hay datos se entrega negro para que la cámara siga activa.

use std::ffi::c_void;
use std::mem::{size_of, ManuallyDrop};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use opencam_core::shared_frame;
use windows::core::{implement, w, Interface, OutRef, Ref, GUID, HRESULT, PCWSTR, PWSTR};
// `IUnknownImpl` (de `windows-core`) aporta `to_interface` a los tipos generados por `#[implement]`.
use windows_core::IUnknownImpl;
use windows::Win32::Foundation::{
    CloseHandle, E_INVALIDARG, E_NOTIMPL, E_OUTOFMEMORY, E_POINTER, E_UNEXPECTED, HANDLE, S_FALSE, S_OK, SIZE,
};
use windows::Win32::Media::DirectShow::{
    IAMStreamConfig, IAMStreamConfig_Impl, IBaseFilter, IEnumMediaTypes, IEnumMediaTypes_Impl, IMediaSample,
    IMemAllocator, IMemInputPin, IPin, IPin_Impl, PIN_DIRECTION, ALLOCATOR_PROPERTIES, PIN_INFO, PINDIR_OUTPUT,
    VIDEO_STREAM_CONFIG_CAPS,
};
use windows::Win32::Media::KernelStreaming::{IKsPropertySet, IKsPropertySet_Impl};
use windows::Win32::Media::MediaFoundation::AM_MEDIA_TYPE;
use windows::Win32::System::Com::{CoTaskMemAlloc, CoTaskMemFree};
use windows::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, VirtualQuery, FILE_MAP_READ, MEMORY_BASIC_INFORMATION,
    MEMORY_MAPPED_VIEW_ADDRESS,
};

/// Resoluciones que ofrece el pin (la primera es la predeterminada).
const SIZES: [(u32, u32); 3] = [(1920, 1080), (1280, 720), (640, 480)];
pub const FPS: i64 = 30;

fn frame_bytes(idx: usize) -> usize {
    (SIZES[idx].0 * SIZES[idx].1 * 2) as usize
}
/// Duración de un frame en unidades de 100 ns (DirectShow).
const FRAME_100NS: i64 = 10_000_000 / FPS;
/// Nombre de la región compartida; debe coincidir con `desktop/src/sink_windows.rs`.
pub const SHARED_NAME: PCWSTR = w!("Local\\OpenCamVirtualCamera");

const MEDIATYPE_VIDEO: GUID = GUID::from_u128(0x73646976_0000_0010_8000_00AA00389B71);
const MEDIASUBTYPE_YUY2: GUID = GUID::from_u128(0x32595559_0000_0010_8000_00AA00389B71);
const FORMAT_VIDEOINFO: GUID = GUID::from_u128(0x05589F80_C356_11CE_BF01_00AA0055595A);
const VFW_E_NOT_CONNECTED: HRESULT = HRESULT(0x8004_0209u32 as i32);
const VFW_E_WRONG_STATE: HRESULT = HRESULT(0x8004_0227u32 as i32);
const VFW_E_INVALIDMEDIATYPE: HRESULT = HRESULT(0x8004_0200u32 as i32);
/// HRESULT_FROM_WIN32(ERROR_SET_NOT_FOUND) / (ERROR_NOT_FOUND): lo que devuelve `IKsPropertySet`.
const E_PROP_SET_UNSUPPORTED: HRESULT = HRESULT(0x8007_0492u32 as i32);
const E_PROP_ID_UNSUPPORTED: HRESULT = HRESULT(0x8007_0490u32 as i32);
const AMPROPSETID_PIN: GUID = GUID::from_u128(0x9b00f101_1567_11d1_b3f1_00aa003761c5);
const PIN_CATEGORY_CAPTURE: GUID = GUID::from_u128(0xfb6c4281_0353_11d1_905f_0000c0cc16ba);
const AMPROPERTY_PIN_CATEGORY: u32 = 0;
const KSPROPERTY_SUPPORT_GET: u32 = 1;

#[repr(C)]
#[derive(Default)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// `BITMAPINFOHEADER` (40 bytes).
#[repr(C)]
#[derive(Default)]
struct BitmapInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_ppm: i32,
    y_ppm: i32,
    clr_used: u32,
    clr_important: u32,
}

/// `VIDEOINFOHEADER` (88 bytes), formato de `FORMAT_VideoInfo`.
#[repr(C)]
#[derive(Default)]
struct VideoInfoHeader {
    src: Rect,
    dst: Rect,
    bit_rate: u32,
    bit_error_rate: u32,
    avg_time_per_frame: i64,
    header: BitmapInfoHeader,
}

/// Estado compartido entre el filtro, el pin y el hilo de entrega.
pub struct Shared {
    running: AtomicBool,
    /// Índice en `SIZES` del formato actual (`IAMStreamConfig`/negociado).
    fmt: AtomicUsize,
    /// `IBaseFilter` propietario, SIN contar referencia: el filtro posee el pin y un strong
    /// aquí sería un ciclo. Lo pone la fábrica y lo borra `Filter::drop`.
    filter: AtomicPtr<c_void>,
    peer: Mutex<Option<IPin>>,
    alloc: Mutex<Option<IMemAllocator>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

// Los interfaces COM del crate `windows` no son Send/Sync. El hilo de entrega
// sólo llama a Receive/GetBuffer sobre los punteros que el grafo nos pasó en
// Connect/NotifyAllocator, y el filtro se registra con ThreadingModel=Both.
// SAFETY: los punteros se usan únicamente desde los métodos de Shared y el hilo
// de entrega, protegidos por Mutex.
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Shared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            running: AtomicBool::new(false),
            fmt: AtomicUsize::new(0),
            filter: AtomicPtr::new(ptr::null_mut()),
            peer: Mutex::new(None),
            alloc: Mutex::new(None),
            thread: Mutex::new(None),
        })
    }

    fn is_connected(&self) -> bool {
        self.peer.lock().unwrap().is_some()
    }

    pub fn set_filter(&self, raw: *mut c_void) {
        self.filter.store(raw, Ordering::Release);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn disconnect(&self) {
        *self.peer.lock().unwrap() = None;
        if let Some(alloc) = self.alloc.lock().unwrap().take() {
            // SAFETY: allocator obtenido en Connect; se desactiva al desconectar.
            unsafe { let _ = alloc.Decommit(); }
        }
    }

    /// Arranca el hilo de entrega (una sola vez).
    pub fn start(self: &Arc<Self>) {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.clone();
        *self.thread.lock().unwrap() = Some(thread::spawn(move || stream(&me)));
    }

    /// Detiene el hilo de entrega y espera a que termine.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.thread.lock().unwrap().take() {
            let _ = handle.join();
        }
    }
}

/// Construye el tipo de medio YUY2 `SIZES[idx]` @ 30 fps. `pbFormat` se reserva con CoTaskMemAlloc.
fn build_media_type(idx: usize) -> Option<AM_MEDIA_TYPE> {
    let (w, h) = SIZES[idx];
    let bytes = frame_bytes(idx);
    // SAFETY: CoTaskMemAlloc devuelve memoria del tamaño pedido o null.
    let format = unsafe { CoTaskMemAlloc(size_of::<VideoInfoHeader>()) } as *mut VideoInfoHeader;
    if format.is_null() {
        return None;
    }
    let vih = VideoInfoHeader {
        avg_time_per_frame: FRAME_100NS,
        header: BitmapInfoHeader {
            size: size_of::<BitmapInfoHeader>() as u32,
            width: w as i32,
            height: h as i32,
            planes: 1,
            bit_count: 16,
            compression: u32::from_le_bytes(*b"YUY2"),
            size_image: bytes as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: `format` apunta a memoria reservada del tamaño de VideoInfoHeader.
    unsafe { ptr::write(format, vih) };
    Some(AM_MEDIA_TYPE {
        majortype: MEDIATYPE_VIDEO,
        subtype: MEDIASUBTYPE_YUY2,
        bFixedSizeSamples: true.into(),
        bTemporalCompression: false.into(),
        lSampleSize: bytes as u32,
        formattype: FORMAT_VIDEOINFO,
        pUnk: ManuallyDrop::new(None),
        cbFormat: size_of::<VideoInfoHeader>() as u32,
        pbFormat: format as *mut u8,
    })
}

/// Índice en `SIZES` de un tipo de medio completo YUY2/VIDEOINFO con una resolución que ofrecemos.
fn media_type_index(mt: &AM_MEDIA_TYPE) -> Option<usize> {
    if mt.majortype != MEDIATYPE_VIDEO
        || mt.subtype != MEDIASUBTYPE_YUY2
        || mt.formattype != FORMAT_VIDEOINFO
        || mt.pbFormat.is_null()
        || (mt.cbFormat as usize) < size_of::<VideoInfoHeader>()
    {
        return None;
    }
    // SAFETY: `pbFormat` no es nulo y `cbFormat` cubre un VIDEOINFOHEADER (comprobado arriba).
    let vih = unsafe { ptr::read_unaligned(mt.pbFormat as *const VideoInfoHeader) };
    // Una altura negativa sólo indica orden de filas; en YUY2 se ignora.
    SIZES.iter().position(|&(w, h)| vih.header.width == w as i32 && vih.header.height.unsigned_abs() == h)
}

/// Copia de `build_media_type` en memoria de COM, para devolverla al llamador (que la libera).
fn alloc_media_type(idx: usize) -> Option<*mut AM_MEDIA_TYPE> {
    let value = build_media_type(idx)?;
    // SAFETY: memoria del tamaño de AM_MEDIA_TYPE, liberada por quien la recibe.
    let mt = unsafe { CoTaskMemAlloc(size_of::<AM_MEDIA_TYPE>()) } as *mut AM_MEDIA_TYPE;
    if mt.is_null() {
        // SAFETY: `pbFormat` lo reservó build_media_type y nadie más lo usa.
        unsafe { CoTaskMemFree(Some(value.pbFormat as *const _)) };
        return None;
    }
    // SAFETY: `mt` tiene espacio para un AM_MEDIA_TYPE.
    unsafe { ptr::write(mt, value) };
    Some(mt)
}

/// Pin de salida único del filtro.
#[implement(IPin, IAMStreamConfig, IKsPropertySet)]
pub struct OutPin {
    shared: Arc<Shared>,
}

impl OutPin {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }
}

impl OutPin_Impl {
    /// Negocia el formato `idx` con `peer` y deja listo el asignador de memoria.
    fn connect_with(&self, peer: &IPin, idx: usize) -> windows::core::Result<()> {
        let me: IPin = self.to_interface();
        let mt = build_media_type(idx).ok_or(E_OUTOFMEMORY)?;
        // SAFETY: llamadas COM sobre interfaces válidas del grafo; `mt.pbFormat` se libera abajo.
        let result = unsafe { peer.ReceiveConnection(&me, &mt) };
        // SAFETY: `pbFormat` vino de CoTaskMemAlloc y ya no lo usa nadie.
        unsafe { CoTaskMemFree(Some(mt.pbFormat as *const _)) };
        result?;
        let setup = || -> windows::core::Result<IMemAllocator> {
            // SAFETY: interfaz recibida del mismo grafo; llamadas COM válidas.
            unsafe {
                let input: IMemInputPin = peer.cast()?;
                let alloc = input.GetAllocator()?;
                input.NotifyAllocator(&alloc, false)?;
                // Sin SetProperties y Commit el asignador no entrega buffers (GetBuffer falla).
                let req = ALLOCATOR_PROPERTIES { cBuffers: 4, cbBuffer: frame_bytes(idx) as i32, cbAlign: 1, cbPrefix: 0 };
                let actual = alloc.SetProperties(&req)?;
                if (actual.cbBuffer as usize) < frame_bytes(idx) {
                    return Err(E_OUTOFMEMORY.into());
                }
                alloc.Commit()?;
                Ok(alloc)
            }
        };
        match setup() {
            Ok(alloc) => {
                self.shared.fmt.store(idx, Ordering::SeqCst);
                *self.shared.alloc.lock().unwrap() = Some(alloc);
                *self.shared.peer.lock().unwrap() = Some(peer.clone());
                Ok(())
            }
            Err(e) => {
                // SAFETY: deshace la conexión que acaba de aceptar el peer.
                unsafe { let _ = peer.Disconnect(); }
                Err(e)
            }
        }
    }
}

impl IPin_Impl for OutPin_Impl {
    fn Connect(&self, preceivepin: Ref<IPin>, pmt: *const AM_MEDIA_TYPE) -> windows::core::Result<()> {
        let peer: IPin = preceivepin.ok()?.clone();
        // SAFETY: `pmt` es nulo o apunta a un AM_MEDIA_TYPE válido del llamador.
        let wanted = unsafe { pmt.as_ref() }.and_then(media_type_index);
        // Con un tipo pedido se intenta sólo ése; si no, el actual y luego los demás.
        let order: Vec<usize> = match wanted {
            Some(i) => vec![i],
            None => self.type_order(),
        };
        let mut last = Err(VFW_E_NOT_CONNECTED.into());
        for idx in order {
            last = self.connect_with(&peer, idx);
            if last.is_ok() {
                break;
            }
        }
        last
    }

    fn ReceiveConnection(&self, _pconnector: Ref<IPin>, _pmt: *const AM_MEDIA_TYPE) -> windows::core::Result<()> {
        // Es un pin de salida: no acepta conexiones entrantes.
        Err(E_UNEXPECTED.into())
    }

    fn Disconnect(&self) -> windows::core::Result<()> {
        self.shared.disconnect();
        Ok(())
    }

    fn ConnectedTo(&self) -> windows::core::Result<IPin> {
        self.shared.peer.lock().unwrap().clone().ok_or(VFW_E_NOT_CONNECTED.into())
    }

    fn ConnectionMediaType(&self, pmt: *mut AM_MEDIA_TYPE) -> windows::core::Result<()> {
        if !self.shared.is_connected() {
            return Err(VFW_E_NOT_CONNECTED.into());
        }
        let mt = build_media_type(self.shared.fmt.load(Ordering::SeqCst)).ok_or(E_OUTOFMEMORY)?;
        // SAFETY: `pmt` lo proporciona el llamador y tiene espacio para un AM_MEDIA_TYPE.
        unsafe { ptr::write(pmt, mt) };
        Ok(())
    }

    fn QueryPinInfo(&self, pinfo: *mut PIN_INFO) -> windows::core::Result<()> {
        // Referencia fuerte para el llamador (la libera él); la del pin al filtro es débil.
        let raw = self.shared.filter.load(Ordering::Acquire);
        // SAFETY: `raw` es el IBaseFilter que posee este pin; `Filter::drop` lo pone a nulo antes de liberarse.
        let filter = unsafe { IBaseFilter::from_raw_borrowed(&raw) }.cloned();
        let info = PIN_INFO {
            pFilter: ManuallyDrop::new(filter),
            dir: PINDIR_OUTPUT,
            achName: name128("Video"),
        };
        // SAFETY: `pinfo` lo proporciona el llamador.
        unsafe { ptr::write(pinfo, info) };
        Ok(())
    }

    fn QueryDirection(&self) -> windows::core::Result<PIN_DIRECTION> {
        Ok(PINDIR_OUTPUT)
    }

    fn QueryId(&self) -> windows::core::Result<PWSTR> {
        co_string("Video")
    }

    fn QueryAccept(&self, pmt: *const AM_MEDIA_TYPE) -> HRESULT {
        if pmt.is_null() {
            return E_POINTER;
        }
        // SAFETY: comprobado no nulo arriba; el llamador pasa un AM_MEDIA_TYPE válido.
        let mt = unsafe { &*pmt };
        let size_ok = mt.pbFormat.is_null() || media_type_index(mt).is_some();
        if mt.majortype == MEDIATYPE_VIDEO && mt.subtype == MEDIASUBTYPE_YUY2 && size_ok {
            S_OK
        } else {
            S_FALSE
        }
    }

    fn EnumMediaTypes(&self) -> windows::core::Result<IEnumMediaTypes> {
        Ok(EnumMediaTypes { order: self.type_order(), index: Mutex::new(0) }.into())
    }

    fn QueryInternalConnections(&self, _appin: OutRef<IPin>, _npin: *mut u32) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn EndOfStream(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn BeginFlush(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn EndFlush(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn NewSegment(&self, _tstart: i64, _tstop: i64, _drate: f64) -> windows::core::Result<()> {
        Ok(())
    }
}

impl OutPin_Impl {
    /// Índices de `SIZES` con el formato actual primero (es el preferido).
    fn type_order(&self) -> Vec<usize> {
        let cur = self.shared.fmt.load(Ordering::SeqCst);
        std::iter::once(cur).chain((0..SIZES.len()).filter(|&i| i != cur)).collect()
    }
}

impl IAMStreamConfig_Impl for OutPin_Impl {
    fn SetFormat(&self, pmt: *const AM_MEDIA_TYPE) -> windows::core::Result<()> {
        // SAFETY: `pmt` es nulo o apunta a un AM_MEDIA_TYPE válido del llamador.
        let idx = match unsafe { pmt.as_ref() } {
            None => 0, // nulo = formato predeterminado
            Some(mt) => media_type_index(mt).ok_or(VFW_E_INVALIDMEDIATYPE)?,
        };
        // shortcut: no reconecta en caliente; con el pin conectado sólo se acepta el formato ya activo.
        if self.shared.is_connected() && idx != self.shared.fmt.load(Ordering::SeqCst) {
            return Err(VFW_E_WRONG_STATE.into());
        }
        self.shared.fmt.store(idx, Ordering::SeqCst);
        Ok(())
    }

    fn GetFormat(&self) -> windows::core::Result<*mut AM_MEDIA_TYPE> {
        alloc_media_type(self.shared.fmt.load(Ordering::SeqCst)).ok_or(E_OUTOFMEMORY.into())
    }

    fn GetNumberOfCapabilities(&self, picount: *mut i32, pisize: *mut i32) -> windows::core::Result<()> {
        if picount.is_null() || pisize.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: punteros no nulos del llamador.
        unsafe {
            *picount = SIZES.len() as i32;
            *pisize = size_of::<VIDEO_STREAM_CONFIG_CAPS>() as i32;
        }
        Ok(())
    }

    fn GetStreamCaps(&self, iindex: i32, ppmt: *mut *mut AM_MEDIA_TYPE, pscc: *mut u8) -> windows::core::Result<()> {
        if ppmt.is_null() || pscc.is_null() {
            return Err(E_POINTER.into());
        }
        let idx = usize::try_from(iindex).ok().filter(|&i| i < SIZES.len()).ok_or(E_INVALIDARG)?;
        let (w, h) = (SIZES[idx].0 as i32, SIZES[idx].1 as i32);
        let size = SIZE { cx: w, cy: h };
        // Resolución fija por índice: mínimo = máximo y sin granularidad (el filtro no recorta ni estira).
        let caps = VIDEO_STREAM_CONFIG_CAPS {
            guid: FORMAT_VIDEOINFO,
            VideoStandard: 0,
            InputSize: size,
            MinCroppingSize: size,
            MaxCroppingSize: size,
            CropGranularityX: 0,
            CropGranularityY: 0,
            CropAlignX: 0,
            CropAlignY: 0,
            MinOutputSize: size,
            MaxOutputSize: size,
            OutputGranularityX: 0,
            OutputGranularityY: 0,
            StretchTapsX: 0,
            StretchTapsY: 0,
            ShrinkTapsX: 0,
            ShrinkTapsY: 0,
            MinFrameInterval: FRAME_100NS,
            MaxFrameInterval: FRAME_100NS,
            MinBitsPerSecond: w * h * 16 * FPS as i32,
            MaxBitsPerSecond: w * h * 16 * FPS as i32,
        };
        let mt = alloc_media_type(idx).ok_or(E_OUTOFMEMORY)?;
        // SAFETY: `pscc` tiene `size_of::<VIDEO_STREAM_CONFIG_CAPS>()` bytes (GetNumberOfCapabilities); `ppmt` no es nulo.
        unsafe {
            ptr::write_unaligned(pscc as *mut VIDEO_STREAM_CONFIG_CAPS, caps);
            *ppmt = mt;
        }
        Ok(())
    }
}

impl IKsPropertySet_Impl for OutPin_Impl {
    fn Set(&self, _: *const GUID, _: u32, _: *const c_void, _: u32, _: *const c_void, _: u32) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn Get(
        &self,
        guidpropset: *const GUID,
        dwpropid: u32,
        _pinstancedata: *const c_void,
        _cbinstancedata: u32,
        ppropdata: *mut c_void,
        cbpropdata: u32,
        pcbreturned: *mut u32,
    ) -> windows::core::Result<()> {
        self.QuerySupported(guidpropset, dwpropid)?;
        if pcbreturned.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: `pcbreturned` no es nulo.
        unsafe { *pcbreturned = size_of::<GUID>() as u32 };
        if ppropdata.is_null() {
            return Ok(()); // consulta de tamaño
        }
        if (cbpropdata as usize) < size_of::<GUID>() {
            return Err(E_UNEXPECTED.into());
        }
        // SAFETY: el llamador reservó al menos `size_of::<GUID>()` bytes.
        unsafe { ptr::write_unaligned(ppropdata as *mut GUID, PIN_CATEGORY_CAPTURE) };
        Ok(())
    }

    fn QuerySupported(&self, guidpropset: *const GUID, dwpropid: u32) -> windows::core::Result<u32> {
        // SAFETY: `guidpropset` es nulo o apunta a un GUID válido.
        match unsafe { guidpropset.as_ref() } {
            Some(g) if *g == AMPROPSETID_PIN => {}
            Some(_) => return Err(E_PROP_SET_UNSUPPORTED.into()),
            None => return Err(E_POINTER.into()),
        }
        if dwpropid == AMPROPERTY_PIN_CATEGORY {
            Ok(KSPROPERTY_SUPPORT_GET)
        } else {
            Err(E_PROP_ID_UNSUPPORTED.into())
        }
    }
}

/// Enumerador de tipos de medio (formato actual primero).
#[implement(IEnumMediaTypes)]
pub struct EnumMediaTypes {
    order: Vec<usize>,
    index: Mutex<u32>,
}

impl IEnumMediaTypes_Impl for EnumMediaTypes_Impl {
    fn Next(&self, cmediatypes: u32, ppmediatypes: *mut *mut AM_MEDIA_TYPE, pcfetched: *mut u32) -> HRESULT {
        if ppmediatypes.is_null() || (pcfetched.is_null() && cmediatypes > 1) {
            return E_POINTER;
        }
        let mut idx = self.index.lock().unwrap();
        let mut fetched = 0u32;
        while fetched < cmediatypes && (*idx as usize) < self.order.len() {
            let Some(mt) = alloc_media_type(self.order[*idx as usize]) else {
                break;
            };
            // SAFETY: el llamador reserva `cmediatypes` posiciones; las recibidas las libera él.
            unsafe { *ppmediatypes.add(fetched as usize) = mt };
            *idx += 1;
            fetched += 1;
        }
        if !pcfetched.is_null() {
            // SAFETY: `pcfetched` lo proporciona el llamador.
            unsafe { *pcfetched = fetched };
        }
        if fetched == cmediatypes { S_OK } else { S_FALSE }
    }

    fn Skip(&self, cmediatypes: u32) -> windows::core::Result<()> {
        let mut idx = self.index.lock().unwrap();
        *idx += cmediatypes;
        if *idx as usize > self.order.len() { Err(S_FALSE.into()) } else { Ok(()) }
    }

    fn Reset(&self) -> windows::core::Result<()> {
        *self.index.lock().unwrap() = 0;
        Ok(())
    }

    fn Clone(&self) -> windows::core::Result<IEnumMediaTypes> {
        Ok(EnumMediaTypes { order: self.order.clone(), index: Mutex::new(*self.index.lock().unwrap()) }.into())
    }
}

/// Hilo de entrega: a 30 fps copia el último frame compartido (o negro), lo escala al formato
/// negociado y lo envía al pin conectado.
fn stream(shared: &Shared) {
    let mut reader: Option<SharedReader> = None;
    let mut src: Vec<u8> = Vec::new();
    let mut frame: Vec<u8> = Vec::new();
    let mut fmt = usize::MAX;
    let mut have_frame = false;
    let period = Duration::from_micros(1_000_000 / FPS as u64);
    let start = Instant::now();
    let mut n: u32 = 0;
    while shared.running.load(Ordering::SeqCst) {
        let cur = shared.fmt.load(Ordering::SeqCst);
        if cur != fmt {
            fmt = cur;
            frame = vec![0u8; frame_bytes(fmt)];
            have_frame = false;
        }
        // Reabre la región cada segundo: recoge un escritor que arranca tarde o que se reinicia.
        if n % (FPS as u32) == 0 {
            reader = SharedReader::open();
        }
        let (dw, dh) = SIZES[fmt];
        let fresh = reader
            .as_ref()
            .and_then(|r| r.read(&mut src))
            .is_some_and(|(sw, sh)| shared_frame::letterbox_yuyv(&src, sw, sh, &mut frame, dw, dh));
        // Si una lectura falla (escritor a medias) se repite el frame anterior; negro sólo si no hubo ninguno.
        have_frame |= fresh;
        if !have_frame {
            fill_black(&mut frame);
        }
        // Marca de tiempo = tiempo de flujo real desde Run (100 ns), monótona aunque el hilo se retrase.
        // shortcut: no usa el reloj de referencia del grafo; basta para una fuente en vivo.
        let t0 = (start.elapsed().as_nanos() / 100) as i64;
        let _ = deliver(shared, &frame, t0, n == 0);
        n = n.wrapping_add(1);
        let next = start + period * n;
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        }
    }
}

fn deliver(shared: &Shared, frame: &[u8], t0: i64, first: bool) -> windows::core::Result<()> {
    let peer = shared.peer.lock().unwrap().clone();
    let alloc = shared.alloc.lock().unwrap().clone();
    let (Some(peer), Some(alloc)) = (peer, alloc) else {
        return Ok(());
    };
    let input: IMemInputPin = peer.cast()?;
    let mut sample: Option<IMediaSample> = None;
    // SAFETY: llamadas COM con punteros válidos a variables locales.
    unsafe {
        alloc.GetBuffer(&mut sample, None, None, 0)?;
        let s = sample.ok_or(E_POINTER)?;
        if (s.GetSize() as usize) < frame.len() {
            return Err(E_OUTOFMEMORY.into());
        }
        let dst = s.GetPointer()?;
        ptr::copy_nonoverlapping(frame.as_ptr(), dst, frame.len());
        s.SetActualDataLength(frame.len() as i32)?;
        let t1 = t0 + FRAME_100NS;
        s.SetTime(Some(&t0 as *const i64), Some(&t1 as *const i64))?;
        s.SetSyncPoint(true)?;
        s.SetDiscontinuity(first)?;
        input.Receive(&s)?;
    }
    Ok(())
}

fn fill_black(frame: &mut [u8]) {
    for px in frame.chunks_exact_mut(4) {
        px.copy_from_slice(&[16, 128, 16, 128]);
    }
}

/// Lector de la región compartida escrita por el cliente de escritorio.
struct SharedReader {
    handle: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    len: usize,
}

impl SharedReader {
    fn open() -> Option<Self> {
        // SAFETY: nombre válido; el handle se cierra en `Drop`.
        let handle = unsafe { OpenFileMappingW(FILE_MAP_READ.0, false, SHARED_NAME) }.ok()?;
        // Longitud 0 = toda la región, sea cual sea la resolución con la que la creó el cliente.
        // SAFETY: handle abierto arriba con permiso de lectura.
        let view = unsafe { MapViewOfFile(handle, FILE_MAP_READ, 0, 0, 0) };
        // SAFETY: `info` es válido y `view` es una vista recién mapeada (o nula, y entonces falla).
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let queried = !view.Value.is_null()
            && unsafe { VirtualQuery(Some(view.Value), &mut info, size_of::<MEMORY_BASIC_INFORMATION>()) } != 0;
        if !queried {
            // SAFETY: vista y handle abiertos arriba y todavía no usados.
            unsafe {
                if !view.Value.is_null() {
                    let _ = UnmapViewOfFile(view);
                }
                let _ = CloseHandle(handle);
            }
            return None;
        }
        Some(Self { handle, view, len: info.RegionSize })
    }

    /// Copia el último frame completo en `out` (seqlock) y devuelve su tamaño; `None` si no hay frame válido.
    fn read(&self, out: &mut Vec<u8>) -> Option<(u32, u32)> {
        // SAFETY: la vista mapea `len` bytes y sigue viva hasta `Drop`. El escritor usa un contador
        // de versión atómico; `read_frame` descarta copias rasgadas.
        let region = unsafe { std::slice::from_raw_parts(self.view.Value as *const u8, self.len) };
        shared_frame::read_frame(region, out)
    }
}

impl Drop for SharedReader {
    fn drop(&mut self) {
        // SAFETY: vista y handle obtenidos en `open`; se liberan una sola vez.
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.handle);
        }
    }
}

pub fn name128(s: &str) -> [u16; 128] {
    let mut out = [0u16; 128];
    for (dst, src) in out.iter_mut().zip(s.encode_utf16()) {
        *dst = src;
    }
    out
}

/// Cadena UTF-16 en memoria de COM (la libera quien la recibe).
fn co_string(s: &str) -> windows::core::Result<PWSTR> {
    let wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: reserva del tamaño exacto de la cadena.
    let p = unsafe { CoTaskMemAlloc(wide.len() * 2) } as *mut u16;
    if p.is_null() {
        return Err(E_OUTOFMEMORY.into());
    }
    // SAFETY: `p` tiene `wide.len()` elementos reservados.
    unsafe { ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len()) };
    Ok(PWSTR(p))
}
