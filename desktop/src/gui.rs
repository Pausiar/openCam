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

use crate::theme::{self, strong, ACCENT, BORDER, MUTED, TEXT};

use crate::cli::{Cli, SinkKind};
use crate::client::Sender;
use crate::discovery::{self, Found};
use crate::prefs;
use crate::session::{self, Event, Observer};
use crate::settings::{Command, Facing};

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
    /// El móvil elegido de la lista de la red, o el primero que se anuncie.
    Wifi,
    /// Android por adb, o iPhone por usbmuxd (`App::iphone`).
    Usb,
    /// Dirección escrita a mano.
    Manual,
}

/// Secciones de la barra lateral.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Devices,
    Settings,
    About,
}

/// Versión mostrada: la del tag en las releases (`OPENCAM_VERSION`), si no la de Cargo.
fn version() -> String {
    let v = option_env!("OPENCAM_VERSION")
        .filter(|v| !v.is_empty())
        .unwrap_or(env!("CARGO_PKG_VERSION"));
    format!("v{}", v.trim_start_matches('v'))
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
    /// El último intento falló por el emparejamiento: hay que escribir el código del móvil.
    need_code: bool,
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
    /// En modo USB: iPhone (usbmuxd) en vez de Android (adb).
    iphone: bool,
    /// Dirección escrita a mano (modo «IP manual»).
    wifi_addr: String,
    /// Móvil elegido de la lista de la red (modo «Wi-Fi»); `None`: el primero que aparezca.
    wifi_pick: Option<String>,
    /// Código de emparejamiento que muestra el móvil (solo hace falta la primera vez).
    code: String,
    page: Page,
    advanced_open: bool,
    /// Alto de la tarjeta de conexión en el último fotograma: la vista previa lo iguala.
    connection_h: f32,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    tx: Arc<Mutex<Option<Sender>>>,
    worker: Option<JoinHandle<()>>,
    discovery: Option<Receiver<io::Result<Vec<Found>>>>,
    found: Vec<Found>,
    discovery_msg: Option<String>,
    texture: Option<egui::TextureHandle>,
    /// Órdenes en vivo (cámara, zoom, espejo...) hacia la sesión en marcha.
    commands: Option<mpsc::Sender<Command>>,
}

/// Lo que la ventana necesita de `Shared` en cada fotograma.
struct View {
    running: bool,
    status: Status,
    stats: Option<(u32, u32, u32, u32)>,
    caps: String,
    log: Vec<String>,
    need_code: bool,
}

/// Bordes + márgenes de `theme::card()` (16 por lado y 1 de borde).
const CARD_CHROME: f32 = 34.0;
const GAP: f32 = 16.0;

impl App {
    fn new(mut cfg: Cli) -> Self {
        // En la ventana no tiene sentido escribir Y4M a stdout: sin cámara virtual, solo vista previa.
        if cfg.sink == SinkKind::Y4m && cfg.output == "-" {
            cfg.sink = SinkKind::None;
        }
        let (mode, iphone, wifi_addr) = match cfg.addr.as_str() {
            "iphone" if cfg!(unix) => (Mode::Usb, true, String::new()),
            "usb" => (Mode::Usb, false, String::new()),
            "wifi" => (Mode::Wifi, false, String::new()),
            a if a.is_empty() || a == "auto" || a == "iphone" => (Mode::Auto, false, String::new()),
            a => (Mode::Manual, false, a.to_string()),
        };
        Self {
            cfg,
            mode,
            iphone,
            wifi_addr,
            wifi_pick: None,
            code: String::new(),
            page: Page::Home,
            advanced_open: false,
            connection_h: 0.0,
            shared: Arc::new(Mutex::new(Shared {
                running: false,
                status: Status::Idle,
                caps: String::new(),
                stats: None,
                log: VecDeque::new(),
                preview: None,
                need_code: false,
            })),
            stop: Arc::new(AtomicBool::new(false)),
            tx: Arc::new(Mutex::new(None)),
            worker: None,
            discovery: None,
            found: Vec::new(),
            discovery_msg: None,
            texture: None,
            commands: None,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn effective_cfg(&self) -> Cli {
        let mut c = self.cfg.clone();
        c.addr = match self.mode {
            Mode::Auto => "auto".into(),
            // Sin móvil elegido, el primero que se anuncie por mDNS.
            Mode::Wifi => self.wifi_pick.clone().unwrap_or_else(|| "wifi".into()),
            Mode::Manual if self.wifi_addr.trim().is_empty() => "wifi".into(),
            Mode::Manual => normalize_addr(&self.wifi_addr),
            Mode::Usb if self.iphone && cfg!(unix) => "iphone".into(),
            Mode::Usb => "usb".into(),
        };
        c
    }

    fn start(&mut self, ctx: &egui::Context) {
        let mut cfg = self.effective_cfg();
        let code = self.code.trim();
        if !code.is_empty() {
            if opencam_core::secure::normalize_code(code).is_err() {
                self.lock().status =
                    Status::Error("El código son los 6 dígitos que muestra el móvil".into());
                return;
            }
            cfg.code = Some(code.to_string());
        }
        if let Err(e) = prefs::save(&cfg) {
            self.lock()
                .log(format!("No se pudieron guardar los ajustes: {e}"));
        }
        self.stop.store(false, Ordering::SeqCst);
        {
            let mut s = self.lock();
            s.running = true;
            s.need_code = false;
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
        let (cmd_tx, cmd_rx) = mpsc::channel();
        self.commands = Some(cmd_tx);
        self.worker = Some(thread::spawn(move || {
            let live = Mutex::new(cfg.live());
            let result = crate::open_sink(&cfg).and_then(|mut sink| {
                session::run(&cfg, &live, &obs, |c| {
                    session::serve(c, &mut *sink, &cfg, &live, Some(&cmd_rx), &obs)
                })
            });
            let stopped = obs.should_stop();
            obs.with(|s| {
                s.running = false;
                s.stats = None;
                s.status = match result {
                    Err(e) if !stopped => match crate::client::PairingError::from_io(&e) {
                        Some(p) => {
                            s.need_code = true;
                            Status::Error(pairing_text(p))
                        }
                        None => Status::Error(sink_hint(&cfg, e)),
                    },
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

    fn start_discovery(&mut self, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let _ = tx.send(discovery::browse(Duration::from_secs(3)));
            ctx.request_repaint();
        });
        self.discovery = Some(rx);
        self.discovery_msg = None;
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

    /// Envía una orden a la sesión en marcha (si la hay). Sin sesión, solo queda en `cfg`.
    fn live(&self, cmd: Command) {
        if let Some(tx) = &self.commands {
            let _ = tx.send(cmd);
        }
    }

    fn view(&self) -> View {
        let s = self.lock();
        View {
            running: s.running,
            status: s.status.clone(),
            stats: s
                .stats
                .as_ref()
                .map(|st| (st.fps, st.rtt_ms, st.dropped, st.bitrate_kbps)),
            caps: s.caps.clone(),
            log: s.log.iter().cloned().collect(),
            need_code: s.need_code,
        }
    }

    fn update_texture(&mut self, ctx: &egui::Context) {
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
    }

    // ───────────────────────── Barra lateral ─────────────────────────

    fn sidebar_ui(&mut self, ui: &mut egui::Ui) {
        // Ondas decorativas al pie, como en el diseño.
        let full = ui.max_rect().expand(16.0);
        let painter = ui.painter().with_clip_rect(full);
        let bottom = full.bottom();
        painter.circle_filled(
            egui::pos2(full.left() + 40.0, bottom + 60.0),
            150.0,
            ACCENT.gamma_multiply(0.10),
        );
        painter.circle_filled(
            egui::pos2(full.right() - 10.0, bottom + 110.0),
            170.0,
            ACCENT.gamma_multiply(0.13),
        );

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            theme::logo(ui, 44.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(strong("OpenCam", 20.0));
                ui.label(theme::muted("Cámara de tu móvil en el PC").size(12.0));
            });
        });
        ui.add_space(24.0);
        for (page, glyph, label) in [
            (Page::Home, "🏠", "Inicio"),
            (Page::Devices, "📱", "Dispositivos"),
            (Page::Settings, "⚙", "Ajustes"),
            (Page::About, "ℹ", "Acerca de"),
        ] {
            if nav_item(ui, glyph, label, self.page == page).clicked() {
                self.page = page;
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(4.0);
            ui.label(theme::muted(format!("{} · código abierto (MIT)", version())).size(12.0));
        });
    }

    // ───────────────────────── Inicio ─────────────────────────

    fn header_ui(&mut self, ui: &mut egui::Ui, v: &View, title: &str, subtitle: &str) {
        ui.horizontal(|ui| {
            theme::logo(ui, 64.0);
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.horizontal(|ui| {
                    ui.label(strong(title, 30.0));
                    theme::badge(ui, &version());
                });
                ui.label(theme::muted(subtitle).size(15.5));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (dot, text) = match &v.status {
                    Status::Idle => (Color32::from_gray(170), "Desconectado"),
                    Status::Connecting(_) => (theme::AMBER, "Conectando…"),
                    Status::Connected(_) => (theme::GREEN, "Conectado"),
                    Status::Retrying(_) => (theme::AMBER, "Reconectando…"),
                    Status::Error(_) => (theme::RED, "Sin conexión"),
                };
                theme::status_pill(ui, dot, text);
            });
        });
    }

    fn home_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, v: &View) {
        self.header_ui(
            ui,
            v,
            "OpenCam",
            "Conecta la cámara de tu móvil y úsala en tu PC",
        );
        ui.add_space(GAP);

        let total = ui.available_width();
        let right_w = (total * 0.38).clamp(320.0, 430.0);
        let left_w = total - right_w - GAP;
        let image_w = left_w - 22.0;
        // 16:9, o más alta si la tarjeta de conexión lo es (la imagen se encaja dentro).
        let image_h = (image_w * 9.0 / 16.0)
            .max(self.connection_h - 22.0)
            .max(200.0);
        let row_h = image_w * 9.0 / 16.0 + 22.0;

        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            ui.allocate_ui(egui::vec2(left_w, row_h), |ui| {
                self.preview_ui(ui, v, egui::vec2(image_w, image_h));
            });
            let r = ui.allocate_ui(egui::vec2(right_w, row_h), |ui| {
                card_sized(ui, right_w, row_h - CARD_CHROME, |ui| {
                    self.connection_ui(ui, ctx, v)
                });
            });
            self.connection_h = r.response.rect.height();
        });
        ui.add_space(GAP);
        self.tiles_ui(ui, v);
        ui.add_space(GAP);
        self.advanced_ui(ui, v);
    }

    fn preview_ui(&mut self, ui: &mut egui::Ui, v: &View, size: egui::Vec2) {
        let frame = egui::Frame::new()
            .fill(theme::CARD)
            .stroke(egui::Stroke::new(1.0, BORDER))
            .corner_radius(18)
            .inner_margin(10);
        frame.show(ui, |ui| {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let painter = ui.painter().clone();
            painter.rect_filled(rect, 14.0, theme::PREVIEW_BG);
            let connected = matches!(v.status, Status::Connected(_));
            match (&self.texture, v.running) {
                (Some(t), true) => {
                    // Encaja la imagen (16:9 u otra) dentro del recuadro.
                    let tex = t.size_vec2();
                    let scale = (rect.width() / tex.x).min(rect.height() / tex.y);
                    let img = egui::Rect::from_center_size(rect.center(), tex * scale);
                    egui::Image::new(t).corner_radius(14).paint_at(ui, img);
                }
                _ => {
                    let (glyph, line1, line2) = match &v.status {
                        Status::Connecting(_) | Status::Retrying(_) => (
                            "📡",
                            "Buscando el móvil…",
                            "Abre OpenCam en el móvil y pulsa Iniciar",
                        ),
                        _ => (
                            "📷",
                            "Sin vídeo",
                            "Pulsa Conectar para ver aquí la cámara del móvil",
                        ),
                    };
                    let c = rect.center();
                    painter.text(
                        c - egui::vec2(0.0, 34.0),
                        egui::Align2::CENTER_CENTER,
                        glyph,
                        egui::FontId::proportional(40.0),
                        Color32::from_white_alpha(170),
                    );
                    painter.text(
                        c + egui::vec2(0.0, 14.0),
                        egui::Align2::CENTER_CENTER,
                        line1,
                        theme::bold_font(18.0),
                        Color32::from_white_alpha(230),
                    );
                    painter.text(
                        c + egui::vec2(0.0, 40.0),
                        egui::Align2::CENTER_CENTER,
                        line2,
                        egui::FontId::proportional(14.0),
                        Color32::from_white_alpha(150),
                    );
                }
            }

            // Etiquetas sobre la imagen.
            let pad = 14.0;
            if let Status::Connected(via) = &v.status {
                chip(
                    &painter,
                    rect.left_top() + egui::vec2(pad, pad),
                    egui::Align2::LEFT_TOP,
                    &format!("📱  Móvil ({via})"),
                );
            }
            if let (true, Some((_, rtt, _, kbps))) = (connected, v.stats) {
                chip(
                    &painter,
                    rect.right_top() + egui::vec2(-pad, pad),
                    egui::Align2::RIGHT_TOP,
                    &format!("{rtt} ms · {:.1} Mbps", kbps as f32 / 1000.0),
                );
            }
            let fps = match v.stats {
                Some((fps, ..)) if connected => fps,
                _ => self.cfg.fps,
            };
            chip(
                &painter,
                rect.left_bottom() + egui::vec2(pad, -pad),
                egui::Align2::LEFT_BOTTOM,
                &format!("{} × {} · {fps} FPS", self.cfg.width, self.cfg.height),
            );

            // Cambiar de cámara (trasera/frontal), en vivo.
            let b = egui::Rect::from_center_size(
                rect.right_bottom() + egui::vec2(-pad - 20.0, -pad - 20.0),
                egui::vec2(40.0, 40.0),
            );
            let r = ui
                .put(
                    b,
                    egui::Button::new(RichText::new("🔄").size(18.0).color(Color32::WHITE))
                        .fill(Color32::from_black_alpha(130))
                        .stroke(egui::Stroke::NONE)
                        .corner_radius(20),
                )
                .on_hover_text("Cambiar entre cámara trasera y frontal");
            if r.clicked() {
                let next = match self.cfg.advanced.facing {
                    Some(Facing::Front) => Facing::Back,
                    _ => Facing::Front,
                };
                self.cfg.advanced.facing = Some(next);
                self.live(Command::Facing(next));
            }
        });
    }

    fn connection_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, v: &View) {
        ui.label(strong("Conexión", 20.0));
        ui.add_space(2.0);
        let running = v.running;
        ui.add_enabled_ui(!running, |ui| {
            theme::segmented(
                ui,
                &mut self.mode,
                &[
                    (Mode::Auto, "Auto"),
                    (Mode::Wifi, "Wi-Fi"),
                    (Mode::Usb, "USB"),
                    (Mode::Manual, "IP manual"),
                ],
            );
            theme::inset().show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.mode_details_ui(ui, ctx);
            });

            ui.add_space(2.0);
            let label = strong("Código de emparejamiento", 14.0);
            ui.label(if v.need_code {
                label.color(theme::AMBER)
            } else {
                label
            });
            ui.add(
                egui::TextEdit::singleline(&mut self.code)
                    .hint_text("123 456")
                    .font(theme::bold_font(18.0))
                    .margin(egui::Margin::symmetric(10, 6))
                    .desired_width(140.0),
            );
            ui.label(
                theme::muted("Solo la primera vez: son los 6 dígitos que muestra el móvil al pulsar Iniciar.")
                    .size(12.5),
            );
        });

        ui.add_space(4.0);
        let text = if running {
            "■  Desconectar"
        } else {
            "▶  Conectar"
        };
        if theme::primary_button(ui, text, ui.available_width()).clicked() {
            if running {
                self.stop();
            } else {
                self.start(ctx);
            }
        }
        let detail = match &v.status {
            Status::Connecting(a) => Some((MUTED, connecting_text(a))),
            Status::Retrying(m) => Some((theme::AMBER, m.clone())),
            Status::Error(m) => Some((theme::RED, m.clone())),
            Status::Connected(via) => Some((theme::GREEN, format!("Conectado por {via}"))),
            Status::Idle => None,
        };
        if let Some((color, text)) = detail {
            ui.label(RichText::new(text).color(color).size(13.0));
        }
    }

    fn mode_details_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let small = |t: &str| theme::muted(t).size(13.0);
        match self.mode {
            Mode::Auto => {
                ui.label(strong("Conexión automática", 14.0));
                ui.label(small(
                    "Prueba Android por USB, iPhone por USB y por último el primer móvil \
                     que encuentre en la Wi-Fi.",
                ));
            }
            Mode::Wifi => {
                ui.horizontal(|ui| {
                    ui.label(strong("Móviles en la red", 14.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let searching = self.discovery.is_some();
                        if searching {
                            ui.spinner();
                        } else if ui.button("Buscar").clicked() {
                            self.start_discovery(ctx);
                        }
                    });
                });
                if self.found.is_empty() {
                    ui.label(small(
                        "Sin elegir, se conecta al primero que encuentre en la misma Wi-Fi.",
                    ));
                }
                if let Some(msg) = &self.discovery_msg {
                    ui.label(small(msg));
                }
                for f in self.found.iter().take(4) {
                    let selected = self.wifi_pick.as_deref() == Some(f.addr.as_str());
                    let label = format!("{}  ·  {}", f.name, f.addr);
                    if ui.selectable_label(selected, label).clicked() {
                        self.wifi_pick = (!selected).then(|| f.addr.clone());
                    }
                }
            }
            Mode::Usb => {
                if cfg!(unix) {
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.iphone, false, "Android");
                        ui.radio_value(&mut self.iphone, true, "iPhone");
                    });
                }
                if self.iphone && cfg!(unix) {
                    ui.label(small(
                        "Conecta el iPhone por cable. Necesita usbmuxd (incluido en macOS; \
                         en Linux instala usbmuxd).",
                    ));
                } else {
                    ui.label(small(
                        "Conecta el móvil por cable con la depuración USB activada. Hace falta \
                         adb (Android platform-tools); el reenvío del puerto es automático.",
                    ));
                }
            }
            Mode::Manual => {
                ui.label(strong("Dirección del móvil", 14.0));
                ui.add(
                    egui::TextEdit::singleline(&mut self.wifi_addr)
                        .hint_text("192.168.1.20:4747")
                        .margin(egui::Margin::symmetric(10, 6))
                        .desired_width(f32::INFINITY),
                );
                ui.label(small(
                    "La que muestra la app del móvil en la pantalla de inicio.",
                ));
            }
        }
    }

    fn tiles_ui(&mut self, ui: &mut egui::Ui, v: &View) {
        let total = ui.available_width();
        let tile_w = (total - 4.0 * GAP) / 5.0;
        // El ancho del ComboBox no incluye su relleno interior.
        let inner = tile_w - CARD_CHROME - 2.0 * ui.spacing().button_padding.x;
        let running = v.running;
        let mut sent = Vec::new();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;

            tile(ui, tile_w, "🎥", "Resolución", |ui| {
                ui.add_enabled_ui(!running, |ui| {
                    egui::ComboBox::from_id_salt("res")
                        .width(inner)
                        .selected_text(format!("{} × {}", self.cfg.width, self.cfg.height))
                        .show_ui(ui, |ui| {
                            for (w, h) in RESOLUTIONS {
                                let r = ui.selectable_label(
                                    (self.cfg.width, self.cfg.height) == (w, h),
                                    format!("{w} × {h}"),
                                );
                                if r.clicked() {
                                    (self.cfg.width, self.cfg.height) = (w, h);
                                }
                            }
                        });
                });
            })
            .on_hover_text("Resolución de la cámara virtual; el vídeo del móvil se escala a ella.");

            tile(ui, tile_w, "🎞", "FPS", |ui| {
                ui.add_enabled_ui(!running && !self.cfg.phone_settings, |ui| {
                    egui::ComboBox::from_id_salt("fps")
                        .width(inner)
                        .selected_text(self.cfg.fps.to_string())
                        .show_ui(ui, |ui| {
                            for f in FPS {
                                ui.selectable_value(&mut self.cfg.fps, f, f.to_string());
                            }
                        });
                });
            });

            let adv = &mut self.cfg.advanced;
            tile(ui, tile_w, "📷", "Cámara", |ui| {
                let facing_text = |f: Option<Facing>| match f {
                    None => "La del móvil",
                    Some(Facing::Back) => "Trasera",
                    Some(Facing::Front) => "Frontal",
                };
                egui::ComboBox::from_id_salt("facing")
                    .width(inner)
                    .selected_text(facing_text(adv.facing))
                    .show_ui(ui, |ui| {
                        for f in [Facing::Back, Facing::Front] {
                            if ui
                                .selectable_label(adv.facing == Some(f), facing_text(Some(f)))
                                .clicked()
                            {
                                adv.facing = Some(f);
                                sent.push(Command::Facing(f));
                            }
                        }
                    });
            });

            tile(ui, tile_w, "↔", "Voltear imagen", |ui| {
                if theme::toggle(ui, &mut adv.mirror).changed() {
                    sent.push(Command::Mirror(adv.mirror));
                }
            })
            .on_hover_text("Efecto espejo, como en las videollamadas.");

            tile(ui, tile_w, "✋", "Estabilización", |ui| {
                let mut stab = adv.stabilization.unwrap_or(true);
                if theme::toggle(ui, &mut stab).changed() {
                    adv.stabilization = Some(stab);
                    sent.push(Command::Stabilization(stab));
                }
            });
        });
        for cmd in sent {
            self.live(cmd);
        }
    }

    fn advanced_ui(&mut self, ui: &mut egui::Ui, v: &View) {
        let w = ui.available_width();
        card_sized(ui, w, 0.0, |ui| {
            let header = ui
                .horizontal(|ui| {
                    ui.label(theme::icon("🔧", 18.0));
                    ui.label(strong("Ajustes avanzados", 16.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(if self.advanced_open { "⏶" } else { "⏷" })
                                .size(16.0)
                                .color(MUTED),
                        );
                    });
                })
                .response;
            if ui
                .interact(header.rect, ui.scope_id().with("adv"), egui::Sense::click())
                .clicked()
            {
                self.advanced_open = !self.advanced_open;
            }
            if !self.advanced_open {
                return;
            }
            ui.add_space(8.0);
            ui.spacing_mut().slider_width = 280.0;
            let mut sent = Vec::new();
            egui::Grid::new("advanced")
                .num_columns(2)
                .spacing([24.0, 14.0])
                .min_col_width(220.0)
                .show(ui, |ui| {
                    ui.label("Zoom");
                    let adv = &mut self.cfg.advanced;
                    let mut zoom = adv.zoom.unwrap_or(1.0);
                    let r = ui.add(
                        egui::Slider::new(&mut zoom, 1.0..=8.0)
                            .step_by(0.1)
                            .suffix("x"),
                    );
                    if r.changed() {
                        adv.zoom = Some(zoom);
                        sent.push(Command::Zoom(zoom));
                    }
                    ui.end_row();

                    ui.add_enabled_ui(!v.running, |ui| {
                        ui.label("Usar la calidad elegida en el móvil")
                            .on_hover_text(
                                "El PC no pide resolución, fps ni bitrate: el móvil usa los suyos.",
                            );
                    });
                    ui.add_enabled_ui(!v.running, |ui| {
                        theme::toggle(ui, &mut self.cfg.phone_settings);
                    });
                    ui.end_row();

                    ui.label("Calidad (bitrate)");
                    ui.add_enabled(
                        !v.running && !self.cfg.phone_settings,
                        egui::Slider::new(&mut self.cfg.bitrate_kbps, 1000..=20000)
                            .step_by(500.0)
                            .suffix(" kbps"),
                    );
                    ui.end_row();

                    ui.label("Reconectar si se corta");
                    ui.add_enabled_ui(!v.running, |ui| {
                        theme::toggle(ui, &mut self.cfg.reconnect);
                    });
                    ui.end_row();
                });
            for cmd in sent {
                self.live(cmd);
            }
        });
    }

    // ───────────────────────── Dispositivos ─────────────────────────

    fn devices_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, v: &View) {
        self.header_ui(
            ui,
            v,
            "Dispositivos",
            "Móviles con OpenCam que el PC puede usar",
        );
        ui.add_space(GAP);
        let w = ui.available_width();
        card_sized(ui, w, 0.0, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::icon("📶", 18.0));
                ui.label(strong("En la Wi-Fi", 18.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.discovery.is_some() {
                        ui.spinner();
                    } else if ui.button("🔍  Buscar móviles").clicked() {
                        self.start_discovery(ctx);
                    }
                });
            });
            ui.label(theme::muted(
                "Abre OpenCam en el móvil, pulsa Iniciar y deja los dos en la misma red.",
            ));
            if let Some(msg) = &self.discovery_msg {
                ui.label(RichText::new(msg).color(theme::AMBER));
            }
            ui.add_space(4.0);
            let mut chosen = None;
            for f in &self.found {
                theme::inset().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let glyph = if f.platform == "ios" {
                            "📱 iPhone"
                        } else {
                            "📱 Android"
                        };
                        ui.label(strong(&f.name, 15.0));
                        ui.label(theme::muted(format!("{glyph} · {}", f.addr)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_enabled(!v.running, egui::Button::new("Usar este"))
                                .clicked()
                            {
                                chosen = Some(f.addr.clone());
                            }
                        });
                    });
                });
            }
            if let Some(addr) = chosen {
                self.mode = Mode::Wifi;
                self.wifi_pick = Some(addr);
                self.page = Page::Home;
            }
        });
        ui.add_space(GAP);
        card_sized(ui, w, 0.0, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::icon("🔌", 18.0));
                ui.label(strong("Por cable", 18.0));
            });
            ui.label(strong("Android", 14.5));
            ui.label(theme::muted(
                "Activa la depuración USB en el móvil y conecta el cable. Hace falta adb \
                 (Android platform-tools); el reenvío del puerto es automático.",
            ));
            ui.label(strong("iPhone", 14.5));
            ui.label(theme::muted(if cfg!(windows) {
                "En Windows, de momento, el iPhone se conecta por Wi-Fi."
            } else {
                "Conecta el cable. Necesita usbmuxd (incluido en macOS; en Linux instala usbmuxd)."
            }));
        });
    }

    // ───────────────────────── Ajustes ─────────────────────────

    fn settings_ui(&mut self, ui: &mut egui::Ui, v: &View) {
        self.header_ui(
            ui,
            v,
            "Ajustes",
            "Salida de vídeo y detalles de la conexión",
        );
        ui.add_space(GAP);
        let w = ui.available_width();
        card_sized(ui, w, 0.0, |ui| {
            ui.label(strong("Salida de vídeo", 18.0));
            ui.add_enabled_ui(!v.running, |ui| {
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
            });
            if cfg!(target_os = "macos") {
                ui.label(theme::muted(
                    "macOS aún no tiene cámara virtual de OpenCam.",
                ));
            }
        });
        ui.add_space(GAP);
        card_sized(ui, w, 0.0, |ui| {
            ui.label(strong("Emparejamiento", 18.0));
            ui.label(theme::muted(
                "La primera vez el PC pide el código de 6 dígitos que muestra el móvil. \
                 Después ambos guardan una clave y la conexión va cifrada (AES-256-GCM).",
            ));
        });
        ui.add_space(GAP);
        card_sized(ui, w, 0.0, |ui| {
            ui.label(strong("Detalles de la conexión", 18.0));
            if let Some((fps, rtt, dropped, kbps)) = v.stats {
                ui.label(format!(
                    "{fps} fps · latencia {rtt} ms · {dropped} descartados · {kbps} kbps"
                ));
            }
            if !v.caps.is_empty() {
                ui.label(
                    RichText::new(format!("CAPS: {}", v.caps))
                        .small()
                        .monospace(),
                );
            }
            if v.log.is_empty() {
                ui.label(theme::muted("Aún no hay mensajes."));
            }
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for l in &v.log {
                        ui.label(RichText::new(l).size(12.5));
                    }
                });
        });
    }

    // ───────────────────────── Acerca de ─────────────────────────

    fn about_ui(&mut self, ui: &mut egui::Ui, v: &View) {
        self.header_ui(ui, v, "Acerca de", "Tu móvil como webcam, libre y gratis");
        ui.add_space(GAP);
        let w = ui.available_width();
        card_sized(ui, w, 0.0, |ui| {
            ui.label(strong(format!("OpenCam {}", version()), 18.0));
            ui.label(
                "Aplicación de código abierto que convierte tu móvil Android o iPhone en la \
                 webcam del PC, por Wi-Fi o por cable, con la conexión cifrada.",
            );
            ui.add_space(4.0);
            ui.hyperlink_to(
                "Código fuente en GitHub",
                "https://github.com/Pausiar/openCam",
            );
            ui.hyperlink_to(
                "Descargas y novedades",
                "https://github.com/Pausiar/openCam/releases",
            );
            ui.add_space(4.0);
            ui.label(theme::muted(
                "Licencia MIT. Tipografía Inter (SIL Open Font License 1.1).",
            ));
        });
    }
}

/// Tarjeta de ancho fijo (`width` incluye borde y márgenes).
fn card_sized<R>(
    ui: &mut egui::Ui,
    width: f32,
    min_h: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    theme::card()
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
            ui.set_width(width - CARD_CHROME);
            ui.set_min_height(min_h);
            ui.vertical(add).inner
        })
        .inner
}

/// Ajuste rápido: icono, título y un control debajo.
fn tile(
    ui: &mut egui::Ui,
    width: f32,
    glyph: &str,
    title: &str,
    add: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    theme::card()
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
            ui.set_width(width - CARD_CHROME);
            ui.set_min_height(96.0);
            ui.vertical(|ui| {
                ui.label(theme::icon(glyph, 22.0));
                ui.label(strong(title, 15.0));
                add(ui);
            });
        })
        .response
}

/// Entrada de la barra lateral.
fn nav_item(ui: &mut egui::Ui, glyph: &str, label: &str, selected: bool) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 46.0), egui::Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 12.0, Color32::from_rgb(226, 220, 252));
    } else if resp.hovered() {
        painter.rect_filled(rect, 12.0, Color32::from_rgb(231, 227, 250));
    }
    let color = if selected { theme::ACCENT_DARK } else { TEXT };
    painter.text(
        rect.left_center() + egui::vec2(16.0, 0.0),
        egui::Align2::LEFT_CENTER,
        glyph,
        egui::FontId::proportional(18.0),
        if selected { ACCENT } else { MUTED },
    );
    painter.text(
        rect.left_center() + egui::vec2(48.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        if selected {
            theme::bold_font(16.0)
        } else {
            egui::FontId::proportional(16.0)
        },
        color,
    );
    ui.add_space(2.0);
    resp
}

/// Etiqueta translúcida sobre la vista previa.
fn chip(painter: &egui::Painter, pos: egui::Pos2, align: egui::Align2, text: &str) {
    let galley = painter.layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(13.5),
        Color32::WHITE,
    );
    let size = galley.size() + egui::vec2(20.0, 12.0);
    let rect = align.anchor_size(pos, size);
    painter.rect_filled(rect, 10.0, Color32::from_black_alpha(140));
    painter.galley(rect.min + egui::vec2(10.0, 6.0), galley, Color32::WHITE);
}

/// Mensaje para la ventana (los de `PairingError` hablan de `--code`).
fn pairing_text(e: &crate::client::PairingError) -> String {
    use crate::client::PairingError as P;
    match e {
        P::NeedCode => "El móvil pide emparejar: escribe el código que muestra y pulsa Conectar".into(),
        P::BadCode => "Código incorrecto: revisa el que muestra el móvil".into(),
        P::KeyMismatch => {
            "El emparejamiento guardado no coincide: escribe el código del móvil para emparejar de nuevo"
                .into()
        }
        other => other.to_string(),
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
        if matches!(self.lock().status, Status::Connected(_)) {
            self.code.clear(); // ya emparejado: la clave queda guardada
        }
        let running = self.lock().running;
        if !running {
            if let Some(w) = self.worker.take() {
                let _ = w.join();
            }
            self.commands = None;
        }
        self.update_texture(&ctx);
        let v = self.view();

        egui::Panel::left("navegacion")
            .resizable(false)
            .exact_size(232.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::SIDEBAR)
                    .inner_margin(egui::Margin::symmetric(16, 18)),
            )
            .show(ui, |ui| self.sidebar_ui(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(26, 22))
                            .show(ui, |ui| match self.page {
                                Page::Home => self.home_ui(ui, &ctx, &v),
                                Page::Devices => self.devices_ui(ui, &ctx, &v),
                                Page::Settings => self.settings_ui(ui, &v),
                                Page::About => self.about_ui(ui, &v),
                            });
                    });
            });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = prefs::save(&self.effective_cfg());
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
    let cfg = prefs::load(base);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("OpenCam")
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([980.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "OpenCam",
        options,
        Box::new(|cc| {
            theme::install(&cc.egui_ctx);
            Ok(Box::new(App::new(cfg)))
        }),
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
