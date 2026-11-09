//! Interfaz gráfica del cliente de escritorio (egui). Misma lógica que la CLI: `session::run`
//! en un hilo aparte, con un observador que pasa el estado y la vista previa a la ventana.

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::cli::{Cli, SinkKind};
use crate::client::Sender;
use crate::discovery::{self, Found};
use crate::session::{self, Event, Observer};
use crate::settings;

const PORT: u16 = 4747;
const PREVIEW_WIDTH: usize = 480;
const PREVIEW_INTERVAL: Duration = Duration::from_millis(66); // ~15 fps de vista previa
const LOG_LINES: usize = 40;

/// Resoluciones de salida: las que anuncia la cámara virtual de Windows.
const RESOLUTIONS: [(u32, u32); 3] = [(1920, 1080), (1280, 720), (640, 480)];
const FPS: [u32; 4] = [15, 24, 30, 60];

/// Cómo buscar el móvil; se traduce a `cli.addr` (`auto`, `usb`, `iphone`, `wifi` o `host:puerto`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Auto,
    Wifi,
    UsbAndroid,
    Iphone,
}

fn connecting_text(addr: &str) -> String {
    match addr {
        "auto" => "Buscando el móvil (USB y Wi-Fi)…".into(),
        "usb" => "Buscando el móvil Android por USB…".into(),
        "iphone" => "Buscando el iPhone por USB…".into(),
        "wifi" => "Buscando el móvil en la Wi-Fi…".into(),
        a => format!("Conectando a {a}…"),
    }
}

#[derive(Clone, PartialEq)]
enum Status {
    Idle,
    Connecting(String),
    Connected(String),
    Retrying(String),
    Error(String),
}

#[derive(Default)]
struct Stats {
    fps: u32,
    rtt_ms: u32,
    dropped: u32,
    bitrate_kbps: u32,
}

struct Preview {
    size: [usize; 2],
    rgba: Vec<u8>,
}

/// Estado que escribe el hilo de sesión y lee la ventana.
struct Shared {
    running: bool,
    status: Status,
    caps: String,
    stats: Option<Stats>,
    log: VecDeque<String>,
    preview: Option<Preview>,
}

impl Shared {
    fn log(&mut self, line: String) {
        if self.log.len() == LOG_LINES {
            self.log.pop_front();
        }
        self.log.push_back(line);
    }
}

struct GuiObserver {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    tx: Arc<Mutex<Option<Sender>>>,
    ctx: egui::Context,
    last_preview: Mutex<Option<Instant>>,
}

impl GuiObserver {
    fn with<R>(&self, f: impl FnOnce(&mut Shared) -> R) -> R {
        let r = f(&mut self.shared.lock().unwrap_or_else(|e| e.into_inner()));
        self.ctx.request_repaint();
        r
    }
}

impl Observer for GuiObserver {
    fn event(&self, e: Event) {
        self.with(|s| match e {
            Event::Connecting(addr) => s.status = Status::Connecting(addr),
            Event::Connected { caps, via } => {
                s.status = Status::Connected(via);
                s.log(format!("Conectado. El móvil ofrece: {caps}"));
                s.caps = caps;
            }
            Event::Retrying { error, delay_ms } => {
                s.stats = None;
                s.log(format!("Conexión perdida: {error}"));
                s.status = Status::Retrying(format!(
                    "{error}. Reintentando en {:.1} s",
                    delay_ms as f32 / 1000.0
                ));
            }
            Event::Stats {
                fps,
                rtt_ms,
                dropped,
                bitrate_kbps,
            } => {
                s.stats = Some(Stats {
                    fps,
                    rtt_ms,
                    dropped,
                    bitrate_kbps,
                })
            }
            Event::Warning(w) => s.log(w),
        });
    }

    fn frame(&self, yuyv: &[u8], width: usize, height: usize) {
        let mut last = self.last_preview.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|t| t.elapsed() < PREVIEW_INTERVAL) {
            return;
        }
        *last = Some(Instant::now());
        let preview = downscale_yuyv(yuyv, width, height, PREVIEW_WIDTH);
        self.with(|s| s.preview = Some(preview));
    }

    fn connected(&self, tx: &Sender) {
        *self.tx.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx.clone());
    }

    fn should_stop(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

/// YUYV (BT.601, rango limitado) -> RGBA reducido a `out_w` de ancho, por vecino más cercano.
fn downscale_yuyv(yuyv: &[u8], w: usize, h: usize, out_w: usize) -> Preview {
    let out_w = out_w.min(w).max(1);
    let out_h = (h * out_w / w.max(1)).max(1);
    let mut rgba = Vec::with_capacity(out_w * out_h * 4);
    for oy in 0..out_h {
        let row = (oy * h / out_h) * w * 2;
        for ox in 0..out_w {
            let x = ox * w / out_w;
            let pair = row + (x & !1) * 2;
            let (y, u, v) = match yuyv.get(pair..pair + 4) {
                Some(p) => (if x & 1 == 0 { p[0] } else { p[2] }, p[1], p[3]),
                None => (16, 128, 128),
            };
            let c = (y as f32 - 16.0) * 1.164;
            let d = u as f32 - 128.0;
            let e = v as f32 - 128.0;
            let clamp = |f: f32| f.clamp(0.0, 255.0) as u8;
            rgba.extend_from_slice(&[
                clamp(c + 1.596 * e),
                clamp(c - 0.392 * d - 0.813 * e),
                clamp(c + 2.017 * d),
                255,
            ]);
        }
    }
    Preview {
        size: [out_w, out_h],
        rgba,
    }
}

/// `192.168.1.20` -> `192.168.1.20:4747`; deja intacto lo que ya trae puerto.
fn normalize_addr(input: &str) -> String {
    let a = input.trim();
    if a.is_empty() {
        return String::new();
    }
    let has_port = a.rsplit_once(':').is_some_and(|(host, port)| {
        !host.is_empty() && !host.ends_with(':') && port.parse::<u16>().is_ok()
    });
    if has_port || a.starts_with('[') && a.contains("]:") {
        a.to_string()
    } else {
        format!("{a}:{PORT}")
    }
}

pub struct App {
    cfg: Cli,
    mode: Mode,
    wifi_addr: String,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    tx: Arc<Mutex<Option<Sender>>>,
    worker: Option<JoinHandle<()>>,
    discovery: Option<Receiver<io::Result<Vec<Found>>>>,
    found: Vec<Found>,
    discovery_msg: Option<String>,
    texture: Option<egui::TextureHandle>,
}

impl App {
    fn new(mut cfg: Cli) -> Self {
        // En la ventana no tiene sentido escribir Y4M a stdout: sin cámara virtual, solo vista previa.
        if cfg.sink == SinkKind::Y4m && cfg.output == "-" {
            cfg.sink = SinkKind::None;
        }
        let (mode, wifi_addr) = match cfg.addr.as_str() {
            "iphone" if cfg!(unix) => (Mode::Iphone, String::new()),
            "usb" => (Mode::UsbAndroid, String::new()),
            "wifi" => (Mode::Wifi, String::new()),
            a if a.is_empty() || a == "auto" || a == "iphone" => (Mode::Auto, String::new()),
            a => (Mode::Wifi, a.to_string()),
        };
        Self {
            cfg,
            mode,
            wifi_addr,
            shared: Arc::new(Mutex::new(Shared {
                running: false,
                status: Status::Idle,
                caps: String::new(),
                stats: None,
                log: VecDeque::new(),
                preview: None,
            })),
            stop: Arc::new(AtomicBool::new(false)),
            tx: Arc::new(Mutex::new(None)),
            worker: None,
            discovery: None,
            found: Vec::new(),
            discovery_msg: None,
            texture: None,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn effective_cfg(&self) -> Cli {
        let mut c = self.cfg.clone();
        c.addr = match self.mode {
            Mode::Auto => "auto".into(),
            // Sin dirección escrita, el primer móvil que se anuncie por mDNS.
            Mode::Wifi if self.wifi_addr.trim().is_empty() => "wifi".into(),
            Mode::Wifi => normalize_addr(&self.wifi_addr),
            Mode::UsbAndroid => "usb".into(),
            Mode::Iphone => "iphone".into(),
        };
        c
    }

    fn start(&mut self, ctx: &egui::Context) {
        let cfg = self.effective_cfg();
        if let Err(e) = settings::save(&cfg) {
            self.lock()
                .log(format!("No se pudieron guardar los ajustes: {e}"));
        }
        self.stop.store(false, Ordering::SeqCst);
        {
            let mut s = self.lock();
            s.running = true;
            s.status = Status::Connecting(cfg.addr.clone());
            s.stats = None;
            s.preview = None;
        }
        let obs = GuiObserver {
            shared: self.shared.clone(),
            stop: self.stop.clone(),
            tx: self.tx.clone(),
            ctx: ctx.clone(),
            last_preview: Mutex::new(None),
        };
        self.worker = Some(thread::spawn(move || {
            let result = crate::open_sink(&cfg).and_then(|mut sink| {
                session::run(&cfg, &obs, |c| session::serve(c, &mut *sink, &cfg, &obs))
            });
            let stopped = obs.should_stop();
            obs.with(|s| {
                s.running = false;
                s.stats = None;
                s.status = match result {
                    Err(e) if !stopped => Status::Error(sink_hint(&cfg, e)),
                    _ => Status::Idle,
                };
            });
        }));
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(tx) = self.tx.lock().unwrap_or_else(|e| e.into_inner()).take() {
            tx.close();
        }
    }

    fn poll_discovery(&mut self) {
        let Some(rx) = &self.discovery else { return };
        let Ok(result) = rx.try_recv() else { return };
        self.discovery = None;
        match result {
            Ok(found) => {
                self.discovery_msg = found.is_empty().then(|| {
                    "No se encontró ningún móvil. ¿Está la app iniciada y en la misma Wi-Fi?".into()
                });
                self.found = found;
            }
            Err(e) => self.discovery_msg = Some(format!("Error al buscar: {e}")),
        }
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Conexión");
        ui.add_space(4.0);
        ui.radio_value(&mut self.mode, Mode::Auto, "Automática (USB y luego Wi-Fi)");
        ui.radio_value(&mut self.mode, Mode::Wifi, "Wi-Fi (misma red que el móvil)");
        ui.radio_value(&mut self.mode, Mode::UsbAndroid, "Android por USB");
        if cfg!(unix) {
            ui.radio_value(&mut self.mode, Mode::Iphone, "iPhone por USB");
        }
        ui.add_space(6.0);
        match self.mode {
            Mode::Auto => {
                ui.label(
                    RichText::new(
                        "Prueba Android por USB, iPhone por USB y por último el primer móvil \
                         que encuentre en la Wi-Fi.",
                    )
                    .small(),
                );
            }
            Mode::Wifi => {
                ui.label(
                    "Dirección que muestra la app del móvil (vacío: la primera que encuentre):",
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.wifi_addr)
                        .hint_text("192.168.1.20:4747")
                        .desired_width(f32::INFINITY),
                );
                ui.horizontal(|ui| {
                    let searching = self.discovery.is_some();
                    if ui
                        .add_enabled(!searching, egui::Button::new("Buscar móviles en la red"))
                        .clicked()
                    {
                        let (tx, rx) = mpsc::channel();
                        let ctx = ctx.clone();
                        thread::spawn(move || {
                            let _ = tx.send(discovery::browse(Duration::from_secs(3)));
                            ctx.request_repaint();
                        });
                        self.discovery = Some(rx);
                        self.discovery_msg = None;
                    }
                    if searching {
                        ui.spinner();
                    }
                });
                if let Some(msg) = &self.discovery_msg {
                    ui.label(RichText::new(msg).small());
                }
                for f in &self.found {
                    let label = format!("{} ({}) - {}", f.name, f.platform, f.addr);
                    if ui
                        .selectable_label(self.wifi_addr == f.addr, label)
                        .clicked()
                    {
                        self.wifi_addr = f.addr.clone();
                    }
                }
            }
            Mode::UsbAndroid => {
                ui.label(
                    RichText::new(
                        "Conecta el móvil por cable con la depuración USB activada. \
                         Hace falta adb (Android platform-tools); el reenvío del puerto es automático.",
                    )
                    .small(),
                );
            }
            Mode::Iphone => {
                ui.label(
                    RichText::new(
                        "Necesita usbmuxd (incluido en macOS; en Linux instala usbmuxd).",
                    )
                    .small(),
                );
            }
        }

        ui.add_space(12.0);
        ui.heading("Vídeo");
        ui.add_space(4.0);
        ui.checkbox(
            &mut self.cfg.phone_settings,
            "Usar la calidad elegida en el móvil",
        )
        .on_hover_text("El PC no pide resolución, fps ni bitrate: el móvil usa los suyos.");
        let custom = !self.cfg.phone_settings;
        egui::Grid::new("video")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Resolución");
                let current = format!("{}x{}", self.cfg.width, self.cfg.height);
                egui::ComboBox::from_id_salt("res")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for (w, h) in RESOLUTIONS {
                            let r = ui.selectable_label(
                                (self.cfg.width, self.cfg.height) == (w, h),
                                format!("{w}x{h}"),
                            );
                            if r.clicked() {
                                (self.cfg.width, self.cfg.height) = (w, h);
                            }
                        }
                    });
                ui.end_row();

                ui.label("Fotogramas/s");
                ui.add_enabled_ui(custom, |ui| {
                    egui::ComboBox::from_id_salt("fps")
                        .selected_text(format!("{} fps", self.cfg.fps))
                        .show_ui(ui, |ui| {
                            for f in FPS {
                                ui.selectable_value(&mut self.cfg.fps, f, format!("{f} fps"));
                            }
                        });
                });
                ui.end_row();

                ui.label("Calidad (bitrate)");
                ui.add_enabled(
                    custom,
                    egui::Slider::new(&mut self.cfg.bitrate_kbps, 1000..=20000)
                        .step_by(500.0)
                        .suffix(" kbps"),
                );
                ui.end_row();
            });
        ui.label(
            RichText::new(
                "La resolución es la de la cámara virtual; el vídeo del móvil se escala a ella.",
            )
            .small()
            .weak(),
        );

        ui.add_space(12.0);
        ui.heading("Salida");
        ui.add_space(4.0);
        if cfg!(target_os = "linux") {
            ui.radio_value(
                &mut self.cfg.sink,
                SinkKind::V4l2,
                "Cámara virtual (v4l2loopback)",
            );
        }
        if cfg!(windows) {
            ui.radio_value(
                &mut self.cfg.sink,
                SinkKind::Windows,
                "Cámara virtual OpenCam",
            );
        }
        ui.radio_value(&mut self.cfg.sink, SinkKind::Y4m, "Grabar a archivo Y4M");
        ui.radio_value(&mut self.cfg.sink, SinkKind::None, "Solo vista previa");
        match self.cfg.sink {
            SinkKind::V4l2 => {
                ui.horizontal(|ui| {
                    ui.label("Dispositivo");
                    ui.text_edit_singleline(&mut self.cfg.device);
                });
            }
            SinkKind::Y4m => {
                if self.cfg.output == "-" {
                    self.cfg.output.clear();
                }
                ui.horizontal(|ui| {
                    ui.label("Archivo");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.cfg.output)
                            .hint_text("opencam.y4m")
                            .desired_width(f32::INFINITY),
                    );
                });
            }
            _ => {}
        }
        if cfg!(target_os = "macos") {
            ui.label(
                RichText::new("macOS aún no tiene cámara virtual de OpenCam.")
                    .small()
                    .weak(),
            );
        }

        ui.add_space(12.0);
        ui.checkbox(&mut self.cfg.reconnect, "Reconectar si se corta");
    }

    fn status_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let (running, status, stats, caps, log) = {
            let s = self.lock();
            (
                s.running,
                s.status.clone(),
                s.stats
                    .as_ref()
                    .map(|st| (st.fps, st.rtt_ms, st.dropped, st.bitrate_kbps)),
                s.caps.clone(),
                s.log.iter().cloned().collect::<Vec<_>>(),
            )
        };
        let preview = self.lock().preview.take();
        if let Some(p) = preview {
            let image = egui::ColorImage::from_rgba_unmultiplied(p.size, &p.rgba);
            match &mut self.texture {
                Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                None => {
                    self.texture =
                        Some(ctx.load_texture("preview", image, egui::TextureOptions::LINEAR))
                }
            }
        }

        ui.horizontal(|ui| {
            let (color, text) = match &status {
                Status::Idle => (Color32::GRAY, "Desconectado".to_string()),
                Status::Connecting(a) => (Color32::from_rgb(230, 170, 30), connecting_text(a)),
                Status::Connected(via) => (
                    Color32::from_rgb(40, 180, 90),
                    format!("Conectado por {via}"),
                ),
                Status::Retrying(m) => (Color32::from_rgb(230, 170, 30), m.clone()),
                Status::Error(m) => (Color32::from_rgb(220, 60, 60), m.clone()),
            };
            let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 6.0, color);
            ui.label(RichText::new(text).strong().size(15.0));
        });
        ui.add_space(8.0);

        let button = if running { "Desconectar" } else { "Conectar" };
        let clicked = ui
            .add_sized(
                [180.0, 36.0],
                egui::Button::new(RichText::new(button).size(16.0)),
            )
            .clicked();
        if clicked {
            if running {
                self.stop();
            } else {
                self.start(ctx);
            }
        }

        if let Some((fps, rtt, dropped, kbps)) = stats {
            ui.add_space(6.0);
            ui.label(format!(
                "{fps} fps · latencia {rtt} ms · {dropped} descartados · {kbps} kbps"
            ));
        }
        ui.add_space(8.0);

        let avail = ui.available_size();
        let preview_h = (avail.y - 120.0).max(120.0);
        let preview_w = avail.x.min(preview_h * 16.0 / 9.0);
        let size = egui::vec2(preview_w, preview_w * 9.0 / 16.0);
        match (&self.texture, running) {
            (Some(t), true) => {
                ui.add(egui::Image::new(t).fit_to_exact_size(size));
            }
            _ => {
                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                ui.painter().rect_filled(rect, 6.0, Color32::from_gray(25));
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Sin vídeo",
                    egui::FontId::proportional(16.0),
                    Color32::from_gray(140),
                );
            }
        }

        ui.add_space(6.0);
        egui::CollapsingHeader::new("Detalles").show(ui, |ui| {
            if !caps.is_empty() {
                ui.label(RichText::new(format!("CAPS: {caps}")).small().monospace());
            }
            egui::ScrollArea::vertical()
                .max_height(120.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for l in &log {
                        ui.label(RichText::new(l).small());
                    }
                });
        });
    }
}

/// Añade una pista al error más común de cada salida.
fn sink_hint(cfg: &Cli, e: io::Error) -> String {
    match cfg.sink {
        SinkKind::V4l2 if e.kind() == io::ErrorKind::NotFound => format!(
            "{e}. ¿Cargaste v4l2loopback? sudo modprobe v4l2loopback video_nr=10 card_label=OpenCam exclusive_caps=1"
        ),
        _ => e.to_string(),
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_discovery();
        let running = self.lock().running;
        if !running {
            if let Some(w) = self.worker.take() {
                let _ = w.join();
            }
        }
        egui::Panel::left("ajustes")
            .resizable(false)
            .exact_size(330.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(8.0);
                    ui.add_enabled_ui(!running, |ui| self.settings_ui(ui, &ctx));
                    ui.add_space(8.0);
                });
            });
        egui::CentralPanel::default_margins().show(ui, |ui| self.status_ui(ui, &ctx));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = settings::save(&self.effective_cfg());
        self.stop();
    }
}

pub fn run() -> io::Result<()> {
    let base = Cli {
        sink: if cfg!(target_os = "macos") {
            SinkKind::None
        } else {
            SinkKind::default_for_os()
        },
        ..Cli::default()
    };
    let cfg = settings::load(base);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("OpenCam")
            .with_inner_size([960.0, 600.0])
            .with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "OpenCam",
        options,
        Box::new(|_cc| Ok(Box::new(App::new(cfg)))),
    )
    .map_err(|e| io::Error::other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_addresses() {
        assert_eq!(normalize_addr(" 192.168.1.20 "), "192.168.1.20:4747");
        assert_eq!(normalize_addr("192.168.1.20:5000"), "192.168.1.20:5000");
        assert_eq!(normalize_addr("[fe80::1]:4747"), "[fe80::1]:4747");
        assert_eq!(normalize_addr(""), "");
    }

    #[test]
    fn downscales_gray_frame() {
        // 4x2 gris medio (Y=126, U=V=128) a 2 de ancho.
        let yuyv = [126u8, 128, 126, 128].repeat(4);
        let p = downscale_yuyv(&yuyv, 4, 2, 2);
        assert_eq!(p.size, [2, 1]);
        assert_eq!(p.rgba.len(), 8);
        assert!(
            p.rgba[..3].iter().all(|c| c.abs_diff(128) <= 2),
            "{:?}",
            p.rgba
        );
    }
}
