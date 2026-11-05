//! Backoff exponencial con jitter determinista (semilla fija => secuencia reproducible).

#[derive(Debug)]
pub struct Backoff {
    base_ms: u64,
    max_ms: u64,
    jitter_pct: u64,
    attempt: u32,
    rng: u64,
}

impl Backoff {
    /// Espera = min(base·2^intento, max), ±`jitter_pct`% (0..=100), nunca por encima de `max_ms`.
    pub fn new(base_ms: u64, max_ms: u64, jitter_pct: u64, seed: u64) -> Self {
        assert!(
            base_ms > 0 && base_ms <= max_ms && jitter_pct <= 100,
            "parámetros inválidos"
        );
        Self {
            base_ms,
            max_ms,
            jitter_pct,
            attempt: 0,
            rng: seed,
        }
    }

    /// splitmix64: suficiente para jitter, sin dependencias.
    fn next_rand(&mut self) -> u64 {
        self.rng = self.rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Siguiente espera en ms; avanza el intento.
    pub fn next_delay_ms(&mut self) -> u64 {
        let shifted = if self.attempt < 63 {
            self.base_ms.saturating_mul(1u64 << self.attempt)
        } else {
            u64::MAX
        };
        let exp = shifted.min(self.max_ms);
        self.attempt = self.attempt.saturating_add(1);
        let span = exp / 100 * self.jitter_pct;
        let delay = exp - span + self.next_rand() % (2 * span + 1);
        delay.min(self.max_ms)
    }

    /// Llamar tras una conexión correcta.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_without_jitter_and_capped() {
        let mut b = Backoff::new(500, 8000, 0, 1);
        let v: Vec<u64> = (0..7).map(|_| b.next_delay_ms()).collect();
        assert_eq!(v, [500, 1000, 2000, 4000, 8000, 8000, 8000]);
    }

    #[test]
    fn jitter_stays_in_range_and_is_deterministic() {
        let run = || {
            let mut b = Backoff::new(500, 8000, 20, 42);
            (0..10).map(|_| b.next_delay_ms()).collect::<Vec<_>>()
        };
        let a = run();
        assert_eq!(a, run());
        for (i, d) in a.iter().enumerate() {
            let exp = (500u64 << i).min(8000);
            assert!(
                *d >= exp * 80 / 100 && *d <= 8000.min(exp * 120 / 100),
                "intento {i}: {d}"
            );
        }
    }

    #[test]
    fn reset_restarts_sequence_and_huge_attempts_do_not_overflow() {
        let mut b = Backoff::new(500, 8000, 0, 1);
        for _ in 0..200 {
            assert!(b.next_delay_ms() <= 8000);
        }
        b.reset();
        assert_eq!(b.next_delay_ms(), 500);
    }
}
