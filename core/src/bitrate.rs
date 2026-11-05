//! Controlador AIMD del bitrate objetivo (se publica en `STATS`).
//!
//! Congestión = RTT > base + umbral, o más descartes que en la ventana anterior:
//! bitrate ×0.8. Si no hay congestión ni descartes durante `stable_windows`
//! ventanas seguidas (histéresis): +250 kbps.

#[derive(Debug, Clone, Copy)]
pub struct BitrateConfig {
    pub min_kbps: u32,
    pub max_kbps: u32,
    pub rtt_threshold_ms: u32,
    pub stable_windows: u32,
}

impl Default for BitrateConfig {
    fn default() -> Self {
        Self {
            min_kbps: 1000,
            max_kbps: 25_000,
            rtt_threshold_ms: 50,
            stable_windows: 3,
        }
    }
}

const STEP_KBPS: u32 = 250;

#[derive(Debug)]
pub struct BitrateController {
    cfg: BitrateConfig,
    current: u32,
    // shortcut: base = mínimo RTT visto, nunca sube; si la ruta empeora de forma permanente, crear otro controlador.
    base_rtt: Option<u32>,
    prev_dropped: u32,
    stable: u32,
}

impl BitrateController {
    pub fn new(cfg: BitrateConfig, initial_kbps: u32) -> Self {
        assert!(cfg.min_kbps <= cfg.max_kbps, "min > max");
        Self {
            cfg,
            current: initial_kbps.clamp(cfg.min_kbps, cfg.max_kbps),
            base_rtt: None,
            prev_dropped: 0,
            stable: 0,
        }
    }

    pub fn target_kbps(&self) -> u32 {
        self.current
    }

    /// Una llamada por ventana de métricas. Devuelve el bitrate objetivo.
    pub fn update(&mut self, rtt_ms: u32, dropped: u32) -> u32 {
        let base = self.base_rtt.map_or(rtt_ms, |b| b.min(rtt_ms));
        self.base_rtt = Some(base);
        let congested = rtt_ms > base + self.cfg.rtt_threshold_ms || dropped > self.prev_dropped;
        self.prev_dropped = dropped;
        if congested {
            self.stable = 0;
            self.current = (self.current * 4 / 5).max(self.cfg.min_kbps);
        } else if dropped == 0 {
            self.stable += 1;
            if self.stable >= self.cfg.stable_windows {
                self.stable = 0;
                self.current = (self.current + STEP_KBPS).min(self.cfg.max_kbps);
            }
        } else {
            self.stable = 0; // descartes constantes: ni baja ni sube
        }
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctl() -> BitrateController {
        BitrateController::new(BitrateConfig::default(), 8000)
    }

    #[test]
    fn decreases_on_rtt_spike() {
        let mut c = ctl();
        c.update(20, 0); // base = 20
        assert_eq!(c.update(100, 0), 6400);
        assert_eq!(c.update(100, 0), 5120);
    }

    #[test]
    fn decreases_when_drops_rise_but_not_when_constant() {
        let mut c = ctl();
        c.update(20, 0);
        assert_eq!(c.update(20, 3), 6400); // suben
        assert_eq!(c.update(20, 3), 6400); // constantes: sin cambio
        assert_eq!(c.update(20, 5), 5120); // suben otra vez
    }

    #[test]
    fn increases_only_after_stable_windows() {
        let mut c = ctl();
        assert_eq!(c.update(20, 0), 8000);
        assert_eq!(c.update(20, 0), 8000);
        assert_eq!(c.update(20, 0), 8250);
        assert_eq!(c.update(20, 0), 8250);
    }

    #[test]
    fn congestion_resets_hysteresis() {
        let mut c = ctl();
        c.update(20, 0);
        c.update(20, 0);
        c.update(200, 0); // baja a 6400, stable = 0
        c.update(20, 0);
        assert_eq!(c.update(20, 0), 6400); // solo 2 ventanas estables
        assert_eq!(c.update(20, 0), 6650);
    }

    #[test]
    fn respects_min_and_max() {
        let mut c = BitrateController::new(
            BitrateConfig {
                max_kbps: 8100,
                ..Default::default()
            },
            8000,
        );
        for _ in 0..9 {
            c.update(20, 0);
        }
        assert_eq!(c.target_kbps(), 8100);
        let mut c = BitrateController::new(BitrateConfig::default(), 1100);
        c.update(20, 0);
        assert_eq!(c.update(500, 0), 1000);
        assert_eq!(c.update(500, 0), 1000);
    }

    #[test]
    fn initial_is_clamped() {
        assert_eq!(
            BitrateController::new(BitrateConfig::default(), 99_999).target_kbps(),
            25_000
        );
    }
}
