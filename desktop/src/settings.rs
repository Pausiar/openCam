//! Ajustes avanzados de vídeo: los que se pueden cambiar en vivo durante la sesión.
//!
//! Los campos `Option` son "no tocar": si el usuario no los fijó, no van en CONFIG y el
//! móvil conserva lo que tenga (p. ej. lo elegido en la propia app). Ver docs/PROTOCOL.md.

use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Facing {
    Back,
    Front,
}

impl Facing {
    pub fn as_str(self) -> &'static str {
        match self {
            Facing::Back => "back",
            Facing::Front => "front",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "back" | "trasera" => Some(Facing::Back),
            "front" | "frontal" => Some(Facing::Front),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Focus {
    /// Autoenfoque continuo de vídeo.
    Auto,
    /// Enfoque manual: 0.0 = infinito, 1.0 = lo más cerca que permita la lente.
    Manual(f32),
}

impl Focus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Focus::Auto),
            "inf" | "infinito" => Some(Focus::Manual(0.0)),
            n => n
                .parse::<f32>()
                .ok()
                .filter(|v| (0.0..=1.0).contains(v))
                .map(Focus::Manual),
        }
    }
}

/// Ajustes cambiables en vivo. Se comparten entre reconexiones: lo cambiado en una sesión
/// se vuelve a pedir en el handshake siguiente.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LiveSettings {
    pub bitrate_kbps: u32,
    pub facing: Option<Facing>,
    pub stabilization: Option<bool>,
    /// Factor de zoom (1.0 = sin zoom). El móvil lo limita a su máximo.
    pub zoom: Option<f32>,
    pub focus: Option<Focus>,
    /// Espejo horizontal: se aplica en el escritorio, el móvil no lo ve.
    pub mirror: bool,
}

impl LiveSettings {
    /// Campos avanzados de CONFIG (con coma inicial), para añadir tras los básicos.
    pub fn config_fields(&self) -> String {
        let mut s = String::new();
        if let Some(f) = self.facing {
            let _ = write!(s, r#","facing":"{}""#, f.as_str());
        }
        if let Some(on) = self.stabilization {
            let _ = write!(s, r#","stabilization":{on}"#);
        }
        if let Some(z) = self.zoom {
            let _ = write!(s, r#","zoom":{}"#, fmt_f32(z));
        }
        match self.focus {
            Some(Focus::Auto) => s.push_str(r#","focus":"auto""#),
            Some(Focus::Manual(d)) => {
                let _ = write!(s, r#","focus":"manual","focus_distance":{}"#, fmt_f32(d));
            }
            None => {}
        }
        s
    }

    /// Aplica una orden de consola. Devuelve si cambia algo que deba enviarse al móvil.
    pub fn apply(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::Bitrate(k) => self.bitrate_kbps = k,
            Command::Facing(f) => self.facing = Some(f),
            Command::Stabilization(on) => self.stabilization = Some(on),
            Command::Zoom(z) => self.zoom = Some(z),
            Command::Focus(f) => self.focus = Some(f),
            Command::Mirror(on) => {
                self.mirror = on;
                return false;
            }
        }
        true
    }
}

/// Valor decimal corto y siempre JSON válido (nada de `NaN` ni `inf`).
fn fmt_f32(v: f32) -> String {
    let v = if v.is_finite() { v } else { 0.0 };
    let s = format!("{:.2}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

pub fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "on" | "si" | "sí" | "true" | "1" => Some(true),
        "off" | "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

pub fn parse_zoom(s: &str) -> Option<f32> {
    s.trim_end_matches('x')
        .parse::<f32>()
        .ok()
        .filter(|z| z.is_finite() && *z >= 1.0 && *z <= 100.0)
}

/// Órdenes que se escriben en la consola mientras se transmite.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    Bitrate(u32),
    Facing(Facing),
    Stabilization(bool),
    Zoom(f32),
    Focus(Focus),
    Mirror(bool),
}

pub const CONTROLS_HELP: &str = "\
Órdenes en vivo (escribe y pulsa Intro):
    zoom <1.0..>         zoom, p. ej. `zoom 2` o `zoom 1.5x`
    enfoque auto|<0..1>  autoenfoque, o manual: 0 = infinito, 1 = lo más cerca
    estabilizacion on|off
    camara frontal|trasera
    espejo on|off        voltea la imagen en horizontal (en el PC)
    bitrate <kbps>       tasa de bits inicial; el control automático sigue ajustándola
    ayuda                muestra esta lista
";

impl Command {
    /// Interpreta una línea de consola. `Ok(None)` = línea vacía.
    pub fn parse(line: &str) -> Result<Option<Command>, String> {
        let mut it = line.split_whitespace();
        let Some(name) = it.next() else {
            return Ok(None);
        };
        let arg = it.next().unwrap_or("");
        let bad = |what: &str| Err(format!("valor inválido para {name}: {what}"));
        let cmd = match name.to_lowercase().as_str() {
            "zoom" => match parse_zoom(arg) {
                Some(z) => Command::Zoom(z),
                None => return bad("usa un número ≥ 1, p. ej. 2"),
            },
            "enfoque" | "focus" => match Focus::parse(arg) {
                Some(f) => Command::Focus(f),
                None => return bad("usa auto o un número entre 0 y 1"),
            },
            "estabilizacion" | "estabilización" | "stab" | "stabilization" => {
                match parse_bool(arg) {
                    Some(b) => Command::Stabilization(b),
                    None => return bad("usa on u off"),
                }
            }
            "camara" | "cámara" | "camera" | "facing" => match Facing::parse(arg) {
                Some(f) => Command::Facing(f),
                None => return bad("usa frontal o trasera"),
            },
            "espejo" | "mirror" => match parse_bool(arg) {
                Some(b) => Command::Mirror(b),
                None => return bad("usa on u off"),
            },
            "bitrate" => match arg.parse::<u32>().ok().filter(|k| *k > 0) {
                Some(k) => Command::Bitrate(k),
                None => return bad("usa kbps, p. ej. 6000"),
            },
            "ayuda" | "help" | "?" => return Err(CONTROLS_HELP.to_string()),
            o => return Err(format!("orden desconocida: {o} (escribe `ayuda`)")),
        };
        Ok(Some(cmd))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_fields_are_not_sent() {
        assert_eq!(LiveSettings::default().config_fields(), "");
    }

    #[test]
    fn all_fields_serialize() {
        let s = LiveSettings {
            bitrate_kbps: 8000,
            facing: Some(Facing::Front),
            stabilization: Some(true),
            zoom: Some(2.5),
            focus: Some(Focus::Manual(0.25)),
            mirror: true,
        };
        assert_eq!(
            s.config_fields(),
            r#","facing":"front","stabilization":true,"zoom":2.5,"focus":"manual","focus_distance":0.25"#
        );
        let auto = LiveSettings {
            focus: Some(Focus::Auto),
            zoom: Some(1.0),
            ..Default::default()
        };
        assert_eq!(auto.config_fields(), r#","zoom":1,"focus":"auto""#);
    }

    #[test]
    fn parses_commands() {
        assert_eq!(Command::parse("zoom 2"), Ok(Some(Command::Zoom(2.0))));
        assert_eq!(Command::parse("zoom 1.5x"), Ok(Some(Command::Zoom(1.5))));
        assert_eq!(
            Command::parse("enfoque auto"),
            Ok(Some(Command::Focus(Focus::Auto)))
        );
        assert_eq!(
            Command::parse("enfoque 0.5"),
            Ok(Some(Command::Focus(Focus::Manual(0.5))))
        );
        assert_eq!(
            Command::parse("camara frontal"),
            Ok(Some(Command::Facing(Facing::Front)))
        );
        assert_eq!(
            Command::parse("estabilizacion off"),
            Ok(Some(Command::Stabilization(false)))
        );
        assert_eq!(Command::parse("espejo on"), Ok(Some(Command::Mirror(true))));
        assert_eq!(
            Command::parse("bitrate 6000"),
            Ok(Some(Command::Bitrate(6000)))
        );
        assert_eq!(Command::parse("   "), Ok(None));
        assert!(Command::parse("zoom 0.5").is_err());
        assert!(Command::parse("enfoque 3").is_err());
        assert!(Command::parse("volar alto").is_err());
    }

    #[test]
    fn mirror_is_local_only() {
        let mut s = LiveSettings::default();
        assert!(!s.apply(Command::Mirror(true)));
        assert!(s.mirror);
        assert!(s.apply(Command::Zoom(3.0)));
        assert_eq!(s.zoom, Some(3.0));
    }
}
