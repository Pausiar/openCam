//! Métricas del receptor: RTT suavizado (EWMA), fps decodificados y descartes por ventana.
//! El reloj lo aporta el llamador (`now_ms`), así los tests son deterministas.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub rtt_ms: u32,
    pub decoded_fps: u32,
    pub dropped: u32,
}

impl Snapshot {
    /// Payload JSON de `MsgType::Stats`.
    pub fn to_json(&self, target_bitrate_kbps: u32) -> String {
        format!(
            "{{\"rtt_ms\":{},\"dropped\":{},\"decoded_fps\":{},\"target_bitrate_kbps\":{}}}",
            self.rtt_ms, self.dropped, self.decoded_fps, target_bitrate_kbps
        )
    }
}

#[derive(Debug)]
pub struct Metrics {
    window_ms: u64,
    window_start: u64,
    rtt: Option<f64>,
    decoded: u32,
    dropped: u32,
}

impl Metrics {
    /// `window_ms` > 0; `now_ms` abre la primera ventana.
    pub fn new(window_ms: u64, now_ms: u64) -> Self {
        assert!(window_ms > 0, "la ventana debe ser > 0");
        Self { window_ms, window_start: now_ms, rtt: None, decoded: 0, dropped: 0 }
    }

    /// EWMA con alpha = 1/8 (el primer valor se toma tal cual).
    pub fn on_rtt(&mut self, sample_ms: u32) {
        let s = sample_ms as f64;
        self.rtt = Some(self.rtt.map_or(s, |r| r + (s - r) / 8.0));
    }

    pub fn on_decoded(&mut self) {
        self.decoded += 1;
    }

    pub fn on_dropped(&mut self, n: u32) {
        self.dropped += n;
    }

    /// Cierra la ventana si ha transcurrido y devuelve sus métricas; `None` si aún no.
    pub fn poll(&mut self, now_ms: u64) -> Option<Snapshot> {
        let elapsed = now_ms.saturating_sub(self.window_start);
        if elapsed < self.window_ms {
            return None;
        }
        let snap = Snapshot {
            rtt_ms: self.rtt.map_or(0, |r| r.round() as u32),
            decoded_fps: (self.decoded as u64 * 1000 / elapsed) as u32,
            dropped: self.dropped,
        };
        self.window_start = now_ms;
        self.decoded = 0;
        self.dropped = 0;
        Some(snap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ewma_smooths_rtt() {
        let mut m = Metrics::new(1000, 0);
        m.on_rtt(40);
        m.on_rtt(80); // 40 + 40/8 = 45
        assert_eq!(m.poll(1000).unwrap().rtt_ms, 45);
    }

    #[test]
    fn window_counts_and_resets() {
        let mut m = Metrics::new(1000, 0);
        for _ in 0..30 {
            m.on_decoded();
        }
        m.on_dropped(2);
        m.on_dropped(1);
        assert_eq!(m.poll(999), None);
        assert_eq!(m.poll(1000), Some(Snapshot { rtt_ms: 0, decoded_fps: 30, dropped: 3 }));
        assert_eq!(m.poll(2000), Some(Snapshot { rtt_ms: 0, decoded_fps: 0, dropped: 0 }));
    }

    #[test]
    fn fps_uses_real_elapsed_time() {
        let mut m = Metrics::new(1000, 0);
        for _ in 0..60 {
            m.on_decoded();
        }
        assert_eq!(m.poll(2000).unwrap().decoded_fps, 30);
    }

    #[test]
    fn json_matches_protocol() {
        let s = Snapshot { rtt_ms: 12, decoded_fps: 30, dropped: 1 };
        assert_eq!(s.to_json(8000), r#"{"rtt_ms":12,"dropped":1,"decoded_fps":30,"target_bitrate_kbps":8000}"#);
    }
}
