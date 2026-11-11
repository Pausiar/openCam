//! Aspecto de la ventana: tema claro lila, tipografía Inter y piezas reutilizables
//! (tarjetas, interruptores, píldoras, el logotipo).

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame, Rect,
    Response, RichText, Sense, Shadow, Stroke, StrokeKind, TextStyle, Ui, Vec2,
};

pub const ACCENT: Color32 = Color32::from_rgb(108, 92, 231);
pub const ACCENT_DARK: Color32 = Color32::from_rgb(84, 68, 205);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(236, 232, 253);
pub const BG: Color32 = Color32::from_rgb(246, 244, 253);
pub const SIDEBAR: Color32 = Color32::from_rgb(239, 236, 251);
pub const CARD: Color32 = Color32::WHITE;
pub const BORDER: Color32 = Color32::from_rgb(229, 225, 245);
pub const TEXT: Color32 = Color32::from_rgb(31, 27, 58);
pub const MUTED: Color32 = Color32::from_rgb(107, 103, 136);
pub const GREEN: Color32 = Color32::from_rgb(34, 197, 94);
pub const AMBER: Color32 = Color32::from_rgb(234, 160, 20);
pub const RED: Color32 = Color32::from_rgb(220, 60, 60);
pub const PREVIEW_BG: Color32 = Color32::from_rgb(30, 27, 46);

/// Familia de la fuente seminegrita (títulos y botones).
pub fn bold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn bold_font(size: f32) -> FontId {
    FontId::new(size, bold())
}

/// Texto en seminegrita del tamaño dado.
pub fn strong(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).font(bold_font(size)).color(TEXT)
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).color(MUTED)
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        (
            "inter",
            &include_bytes!("../assets/fonts/Inter-Regular.otf")[..],
        ),
        (
            "inter-semibold",
            &include_bytes!("../assets/fonts/Inter-SemiBold.otf")[..],
        ),
        (
            "icons-noto",
            &include_bytes!("../assets/fonts/icons-noto.ttf")[..],
        ),
        (
            "icons-extra",
            &include_bytes!("../assets/fonts/icons-extra.ttf")[..],
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    // Iconos (subconjuntos de fuentes de emojis monocromas) como respaldo detrás de Inter.
    let mut fallback = vec!["icons-noto".to_string(), "icons-extra".to_string()];
    fallback.extend(
        fonts
            .families
            .get(&FontFamily::Proportional)
            .into_iter()
            .flatten()
            .filter(|n| !n.contains("Ubuntu"))
            .cloned(),
    );
    let with = |first: &str| {
        let mut v = vec![first.to_string()];
        v.extend(fallback.iter().cloned());
        v
    };
    fonts
        .families
        .insert(FontFamily::Proportional, with("inter"));
    fonts.families.insert(bold(), with("inter-semibold"));
    ctx.set_fonts(fonts);

    ctx.set_theme(egui::Theme::Light);
    ctx.style_mut_of(egui::Theme::Light, |style| {
        use FontFamily::Proportional as P;
        style.text_styles = [
            (TextStyle::Small, FontId::new(12.0, P)),
            (TextStyle::Body, FontId::new(14.5, P)),
            (TextStyle::Button, FontId::new(14.5, P)),
            (TextStyle::Heading, FontId::new(20.0, bold())),
            (
                TextStyle::Monospace,
                FontId::new(13.5, FontFamily::Monospace),
            ),
        ]
        .into();
        let s = &mut style.spacing;
        s.item_spacing = egui::vec2(8.0, 8.0);
        s.button_padding = egui::vec2(12.0, 6.0);
        s.interact_size.y = 28.0;
        s.combo_height = 300.0;
        // Barra de desplazamiento lila (bg_fill), no del color del texto.
        s.scroll.foreground_color = false;

        let v = &mut style.visuals;
        v.panel_fill = BG;
        v.window_fill = CARD;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.window_corner_radius = CornerRadius::same(12);
        v.menu_corner_radius = CornerRadius::same(10);
        v.extreme_bg_color = Color32::WHITE;
        v.faint_bg_color = SIDEBAR;
        v.override_text_color = Some(TEXT);
        v.hyperlink_color = ACCENT;
        v.selection.bg_fill = Color32::from_rgb(201, 190, 251);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.slider_trailing_fill = true;
        v.popup_shadow = soft_shadow();
        let radius = CornerRadius::same(10);
        let w = &mut v.widgets;
        w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        // bg_fill: carril de los deslizadores, casillas y barra de desplazamiento.
        w.inactive.bg_fill = Color32::from_rgb(222, 217, 243);
        w.inactive.weak_bg_fill = Color32::WHITE;
        w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        w.hovered.bg_fill = ACCENT_SOFT;
        w.hovered.weak_bg_fill = ACCENT_SOFT;
        w.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.6));
        w.hovered.fg_stroke = Stroke::new(1.5, TEXT);
        w.active.bg_fill = ACCENT_SOFT;
        w.active.weak_bg_fill = ACCENT_SOFT;
        w.active.bg_stroke = Stroke::new(1.0, ACCENT);
        w.active.fg_stroke = Stroke::new(1.5, ACCENT_DARK);
        w.open.weak_bg_fill = ACCENT_SOFT;
        w.open.bg_stroke = Stroke::new(1.0, ACCENT);
        for wv in [
            &mut w.noninteractive,
            &mut w.inactive,
            &mut w.hovered,
            &mut w.active,
            &mut w.open,
        ] {
            wv.corner_radius = radius;
            wv.expansion = 0.0;
        }
    });
}

fn soft_shadow() -> Shadow {
    Shadow {
        offset: [0, 4],
        blur: 18,
        spread: 0,
        color: Color32::from_rgba_unmultiplied(80, 60, 180, 22),
    }
}

/// Tarjeta blanca con borde suave y sombra.
pub fn card() -> Frame {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(16)
        .shadow(soft_shadow())
        .inner_margin(16)
}

/// Recuadro lila claro dentro de una tarjeta.
pub fn inset() -> Frame {
    Frame::new()
        .fill(BG)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(12)
        .inner_margin(12)
}

/// Píldora con un punto de color delante (estado de la conexión).
pub fn status_pill(ui: &mut Ui, dot: Color32, text: &str) {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(255)
        .shadow(soft_shadow())
        .inner_margin(egui::Margin::symmetric(14, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // `horizontal` hereda la dirección: en una fila de derecha a izquierda,
                // el punto se añade después para que siga quedando delante del texto.
                let rtl = ui.layout().prefer_right_to_left();
                if rtl {
                    ui.label(strong(text, 14.5));
                }
                let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 6.0, dot);
                if !rtl {
                    ui.label(strong(text, 14.5));
                }
            });
        });
}

/// Etiqueta pequeña con fondo (p. ej. la versión).
pub fn badge(ui: &mut Ui, text: &str) {
    Frame::new()
        .fill(ACCENT_SOFT)
        .corner_radius(255)
        .inner_margin(egui::Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).font(bold_font(14.0)).color(ACCENT_DARK));
        });
}

/// Botón principal (relleno morado, ancho completo).
pub fn primary_button(ui: &mut Ui, text: &str, width: f32) -> Response {
    let button = egui::Button::new(
        RichText::new(text)
            .font(bold_font(16.0))
            .color(Color32::WHITE),
    )
    .fill(ACCENT)
    .stroke(Stroke::NONE)
    .corner_radius(12)
    .min_size(egui::vec2(width, 46.0));
    ui.add(button)
}

/// Interruptor deslizante (como el de los móviles).
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let size = egui::vec2(46.0, 26.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(response.id, *on);
    if ui.is_rect_visible(rect) {
        let alpha = if ui.is_enabled() { 1.0 } else { 0.45 };
        let off = Color32::from_rgb(208, 205, 224);
        let bg = off.lerp_to_gamma(ACCENT, t).gamma_multiply(alpha);
        let r = rect.height() / 2.0;
        ui.painter().rect_filled(rect, r, bg);
        let x = egui::lerp((rect.left() + r)..=(rect.right() - r), t);
        ui.painter().circle_filled(
            egui::pos2(x, rect.center().y),
            r - 3.5,
            Color32::WHITE.gamma_multiply(alpha.max(0.8)),
        );
    }
    response
}

/// Selector segmentado: devuelve true si cambió.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    let width = ui.available_width();
    let height = 44.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect(rect, 14.0, BG, Stroke::new(1.0, BORDER), StrokeKind::Inside);
    let seg_w = (width - 8.0) / options.len().max(1) as f32;
    for (i, (opt, label)) in options.iter().enumerate() {
        let r = Rect::from_min_size(
            egui::pos2(rect.left() + 4.0 + i as f32 * seg_w, rect.top() + 4.0),
            egui::vec2(seg_w, height - 8.0),
        );
        let id = ui.scope_id().with(("seg", i));
        let resp = ui.interact(r, id, Sense::click());
        let selected = *value == *opt;
        if resp.clicked() && !selected {
            *value = *opt;
            changed = true;
        }
        let enabled = ui.is_enabled();
        if selected {
            painter.rect_filled(
                r,
                11.0,
                if enabled {
                    ACCENT
                } else {
                    ACCENT.gamma_multiply(0.5)
                },
            );
        } else if resp.hovered() && enabled {
            painter.rect_filled(r, 11.0, ACCENT_SOFT);
        }
        let color = if selected {
            Color32::WHITE
        } else if enabled {
            TEXT
        } else {
            MUTED
        };
        painter.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            bold_font(14.0),
            color,
        );
    }
    changed
}

/// Logotipo dibujado: cuadrado morado con un objetivo de cámara.
pub fn logo(ui: &mut Ui, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_logo(ui.painter(), rect);
    response
}

pub fn paint_logo(painter: &egui::Painter, rect: Rect) {
    let s = rect.width();
    let radius = s * 0.24;
    painter.rect_filled(rect, radius, Color32::from_rgb(78, 70, 190));
    // Brillo en la mitad superior para dar volumen.
    let top = Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.center().y + s * 0.1));
    painter.rect_filled(
        top,
        CornerRadius {
            nw: radius as u8,
            ne: radius as u8,
            sw: (s * 0.5) as u8,
            se: (s * 0.5) as u8,
        },
        Color32::from_rgb(112, 104, 240),
    );
    let c = rect.center() + egui::vec2(0.0, s * 0.03);
    painter.circle_filled(c, s * 0.34, Color32::from_rgb(232, 236, 255));
    painter.circle_filled(c, s * 0.26, Color32::from_rgb(28, 36, 80));
    painter.circle_filled(c, s * 0.17, Color32::from_rgb(58, 110, 230));
    painter.circle_filled(c, s * 0.08, Color32::from_rgb(18, 22, 50));
    painter.circle_filled(
        c + egui::vec2(-s * 0.07, -s * 0.07),
        s * 0.045,
        Color32::from_white_alpha(220),
    );
    painter.circle_filled(
        rect.right_top() + egui::vec2(-s * 0.2, s * 0.2),
        s * 0.07,
        Color32::from_rgb(255, 122, 89),
    );
}

/// Icono (glifo de la fuente de emojis) en el color de acento.
pub fn icon(glyph: &str, size: f32) -> RichText {
    RichText::new(glyph).size(size).color(ACCENT)
}
