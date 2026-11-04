//! Pin de salida del filtro: tipo de medio YUY2 1920x1080 a 30 fps y entrega de muestras.
//! Los frames llegan de la memoria compartida (`opencam_core::shared_frame`); si no hay
//! datos se entrega negro para que la cámara siga activa.

use std::mem::{size_of, ManuallyDrop};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use opencam_core::shared_frame::{self, HEADER_LEN};
use windows::core::{implement, w, Interface, OutRef, Ref, GUID, HRESULT, PCWSTR, PWSTR};
// `IUnknownImpl` (de `windows-core`) aporta `to_interface` a los tipos generados por `#[implement]`.
use windows_core::IUnknownImpl;
use windows::Win32::Foundation::{CloseHandle, E_NOTIMPL, E_OUTOFMEMORY, E_POINTER, E_UNEXPECTED, HANDLE, S_FALSE, S_OK};
use windows::Win32::Media::DirectShow::{
    IEnumMediaTypes, IEnumMediaTypes_Impl, IMediaSample, IMemAllocator, IMemInputPin, IPin, IPin_Impl, PIN_DIRECTION,
    ALLOCATOR_PROPERTIES, PIN_INFO, PINDIR_OUTPUT,
};
use windows::Win32::Media::MediaFoundation::AM_MEDIA_TYPE;
use windows::Win32::System::Com::{CoTaskMemAlloc, CoTaskMemFree};
use windows::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS,
};

pub const WIDTH: u32 = 1920;
pub const HEIGHT: u32 = 1080;
pub const FPS: i64 = 30;
pub const PIXEL_BYTES: usize = (WIDTH * HEIGHT * 2) as usize;
/// Duración de un frame en unidades de 100 ns (DirectShow).
const FRAME_100NS: i64 = 10_000_000 / FPS;
/// Nombre de la región compartida; debe coincidir con `desktop/src/sink_windows.rs`.
pub const SHARED_NAME: PCWSTR = w!("Local\\OpenCamVirtualCamera");

const MEDIATYPE_VIDEO: GUID = GUID::from_u128(0x73646976_0000_0010_8000_00AA00389B71);
const MEDIASUBTYPE_YUY2: GUID = GUID::from_u128(0x32595559_0000_0010_8000_00AA00389B71);
const FORMAT_VIDEOINFO: GUID = GUID::from_u128(0x05589F80_C356_11CE_BF01_00AA0055595A);
const VFW_E_NOT_CONNECTED: HRESULT = HRESULT(0x8004_0209u32 as i32);

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
            peer: Mutex::new(None),
            alloc: Mutex::new(None),
            thread: Mutex::new(None),
        })
    }

    fn is_connected(&self) -> bool {
        self.peer.lock().unwrap().is_some()
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

/// Construye el tipo de medio YUY2 1920x1080 @ 30 fps. `pbFormat` se reserva con CoTaskMemAlloc.
pub fn build_media_type() -> Option<AM_MEDIA_TYPE> {
    // SAFETY: CoTaskMemAlloc devuelve memoria del tamaño pedido o null.
    let format = unsafe { CoTaskMemAlloc(size_of::<VideoInfoHeader>()) } as *mut VideoInfoHeader;
    if format.is_null() {
        return None;
    }
    let vih = VideoInfoHeader {
        avg_time_per_frame: FRAME_100NS,
        header: BitmapInfoHeader {
            size: size_of::<BitmapInfoHeader>() as u32,
            width: WIDTH as i32,
            height: HEIGHT as i32,
            planes: 1,
            bit_count: 16,
            compression: u32::from_le_bytes(*b"YUY2"),
            size_image: PIXEL_BYTES as u32,
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
        lSampleSize: PIXEL_BYTES as u32,
        formattype: FORMAT_VIDEOINFO,
        pUnk: ManuallyDrop::new(None),
        cbFormat: size_of::<VideoInfoHeader>() as u32,
        pbFormat: format as *mut u8,
    })
}

/// Pin de salida único del filtro.
#[implement(IPin)]
pub struct OutPin {
    shared: Arc<Shared>,
}

impl OutPin {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }
}

impl IPin_Impl for OutPin_Impl {
    fn Connect(&self, preceivepin: Ref<IPin>, _pmt: *const AM_MEDIA_TYPE) -> windows::core::Result<()> {
        let peer: IPin = preceivepin.ok()?.clone();
        let me: IPin = self.to_interface();
        let mt = build_media_type().ok_or(E_OUTOFMEMORY)?;
        // SAFETY: llamadas COM sobre interfaces válidas del grafo; `mt.pbFormat` se libera abajo.
        let result = unsafe { peer.ReceiveConnection(&me, &mt) };
        // SAFETY: `pbFormat` vino de CoTaskMemAlloc y ya no lo usa nadie.
        unsafe { CoTaskMemFree(Some(mt.pbFormat as *const _)) };
        result?;
        // SAFETY: interfaz recibida del mismo grafo; llamadas COM válidas.
        unsafe {
            let input: IMemInputPin = peer.cast()?;
            let alloc = input.GetAllocator()?;
            input.NotifyAllocator(&alloc, false)?;
            // Sin SetProperties y Commit el asignador no entrega buffers (GetBuffer falla).
            let req = ALLOCATOR_PROPERTIES { cBuffers: 4, cbBuffer: PIXEL_BYTES as i32, cbAlign: 1, cbPrefix: 0 };
            let actual = alloc.SetProperties(&req)?;
            if (actual.cbBuffer as usize) < PIXEL_BYTES {
                return Err(E_OUTOFMEMORY.into());
            }
            alloc.Commit()?;
            *self.shared.alloc.lock().unwrap() = Some(alloc);
        }
        *self.shared.peer.lock().unwrap() = Some(peer);
        Ok(())
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
        let mt = build_media_type().ok_or(E_OUTOFMEMORY)?;
        // SAFETY: `pmt` lo proporciona el llamador y tiene espacio para un AM_MEDIA_TYPE.
        unsafe { ptr::write(pmt, mt) };
        Ok(())
    }

    fn QueryPinInfo(&self, pinfo: *mut PIN_INFO) -> windows::core::Result<()> {
        let info = PIN_INFO {
            pFilter: ManuallyDrop::new(None),
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
        if mt.majortype == MEDIATYPE_VIDEO && mt.subtype == MEDIASUBTYPE_YUY2 {
            S_OK
        } else {
            S_FALSE
        }
    }

    fn EnumMediaTypes(&self) -> windows::core::Result<IEnumMediaTypes> {
        Ok(EnumMediaTypes { index: Mutex::new(0) }.into())
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

/// Enumerador con un único tipo de medio.
#[implement(IEnumMediaTypes)]
pub struct EnumMediaTypes {
    index: Mutex<u32>,
}

impl IEnumMediaTypes_Impl for EnumMediaTypes_Impl {
    fn Next(&self, cmediatypes: u32, ppmediatypes: *mut *mut AM_MEDIA_TYPE, pcfetched: *mut u32) -> HRESULT {
        let mut idx = self.index.lock().unwrap();
        let mut fetched = 0u32;
        if cmediatypes > 0 && *idx == 0 && !ppmediatypes.is_null() {
            if let Some(value) = build_media_type() {
                // SAFETY: memoria del tamaño de AM_MEDIA_TYPE, liberada por quien la recibe.
                let mt = unsafe { CoTaskMemAlloc(size_of::<AM_MEDIA_TYPE>()) } as *mut AM_MEDIA_TYPE;
                if mt.is_null() {
                    // SAFETY: `pbFormat` lo reservó build_media_type y nadie más lo usa.
                    unsafe { CoTaskMemFree(Some(value.pbFormat as *const _)) };
                } else {
                    unsafe {
                        ptr::write(mt, value);
                        *ppmediatypes = mt;
                    }
                    *idx = 1;
                    fetched = 1;
                }
            }
        }
        if !pcfetched.is_null() {
            // SAFETY: `pcfetched` lo proporciona el llamador.
            unsafe { *pcfetched = fetched };
        }
        if fetched == cmediatypes { S_OK } else { S_FALSE }
    }

    fn Skip(&self, cmediatypes: u32) -> windows::core::Result<()> {
        *self.index.lock().unwrap() += cmediatypes;
        Ok(())
    }

    fn Reset(&self) -> windows::core::Result<()> {
        *self.index.lock().unwrap() = 0;
        Ok(())
    }

    fn Clone(&self) -> windows::core::Result<IEnumMediaTypes> {
        Ok(EnumMediaTypes { index: Mutex::new(*self.index.lock().unwrap()) }.into())
    }
}

/// Hilo de entrega: a 30 fps copia el último frame compartido (o negro) y lo envía al pin conectado.
fn stream(shared: &Shared) {
    let mut reader: Option<SharedReader> = None;
    let mut frame = vec![0u8; PIXEL_BYTES];
    let period = Duration::from_micros(1_000_000 / FPS as u64);
    let start = Instant::now();
    let mut n: u32 = 0;
    while shared.running.load(Ordering::SeqCst) {
        // Reabre la región cada segundo: recoge un escritor que arranca tarde o que se reinicia.
        if n % (FPS as u32) == 0 {
            reader = SharedReader::open();
        }
        let fresh = reader.as_ref().is_some_and(|r| r.read(&mut frame));
        if !fresh {
            fill_black(&mut frame);
        }
        let _ = deliver(shared, &frame, n);
        n = n.wrapping_add(1);
        let next = start + period * n;
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        }
    }
}

fn deliver(shared: &Shared, frame: &[u8], n: u32) -> windows::core::Result<()> {
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
        let dst = s.GetPointer()?;
        ptr::copy_nonoverlapping(frame.as_ptr(), dst, frame.len());
        s.SetActualDataLength(frame.len() as i32)?;
        let t0 = n as i64 * FRAME_100NS;
        let t1 = t0 + FRAME_100NS;
        s.SetTime(Some(&t0 as *const i64), Some(&t1 as *const i64))?;
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
        let len = shared_frame::region_len(WIDTH, HEIGHT);
        // SAFETY: nombre válido; el handle se cierra en `Drop`.
        let handle = unsafe { OpenFileMappingW(FILE_MAP_READ.0, false, SHARED_NAME) }.ok()?;
        // SAFETY: handle abierto arriba con tamaño de lectura.
        let view = unsafe { MapViewOfFile(handle, FILE_MAP_READ, 0, 0, len) };
        if view.Value.is_null() {
            // SAFETY: handle abierto arriba y todavía no usado.
            unsafe { let _ = CloseHandle(handle); }
            return None;
        }
        Some(Self { handle, view, len })
    }

    /// Copia el último frame completo en `out` (seqlock). Devuelve `false` si no hay frame válido.
    fn read(&self, out: &mut [u8]) -> bool {
        // SAFETY: la vista mapea `len` bytes y sigue viva hasta `Drop`. El escritor usa
        // un contador de versión; si cambia durante la copia se reintenta.
        let region = unsafe { std::slice::from_raw_parts(self.view.Value as *const u8, self.len) };
        for _ in 0..3 {
            let Some(before) = shared_frame::read_seq(region) else {
                return false;
            };
            if before % 2 == 1 {
                continue;
            }
            out.copy_from_slice(&region[HEADER_LEN..]);
            if shared_frame::read_seq(region) == Some(before) {
                return true;
            }
        }
        false
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
