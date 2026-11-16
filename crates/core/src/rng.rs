//! Deterministic seeded PRNG derived from `(seed, event_id)` for reproducible draws.

use monoagens_protocol::EventId;

/// Deterministic 64-bit SplitMix64 generator seeded from `(global_seed, event_id)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeededRng {
    state: u64,
}

/// Result of a deterministic gate Bernoulli draw for `(seed, event_id)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GateDraw {
    pub draw: f64,
    pub passed: bool,
}

impl SeededRng {
    /// Creates a deterministic PRNG for a given `(seed, event_id)` pair.
    #[must_use]
    pub const fn for_event(seed: u64, event_id: EventId) -> Self {
        let mixed = seed ^ event_id.as_u64().wrapping_mul(0x9E37_79B9_7F4A_7C15);
        Self {
            state: splitmix64(mixed),
        }
    }

    /// Returns the next deterministic `u64`.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        splitmix64(self.state)
    }

    /// Returns a deterministic `f64` in `[0.0, 1.0)`.
    pub fn next_unit_f64(&mut self) -> f64 {
        let bits = self.next_u64() >> 11;
        (bits as f64) * (1.0 / ((1u64 << 53) as f64))
    }

    /// Performs a deterministic unit draw in `[0.0, 1.0)` for `(seed, event_id)`.
    #[must_use]
    pub fn draw_unit(seed: u64, event_id: EventId) -> f64 {
        Self::for_event(seed, event_id).next_unit_f64()
    }

    /// Performs a deterministic Bernoulli draw against `probability` for `(seed, event_id)`.
    #[must_use]
    pub fn draw_bernoulli(seed: u64, event_id: EventId, probability: f64) -> GateDraw {
        let draw = Self::draw_unit(seed, event_id);
        let p = if probability.is_nan() {
            0.0
        } else {
            probability.clamp(0.0, 1.0)
        };
        let passed = if p <= 0.0 {
            false
        } else if p >= 1.0 {
            true
        } else {
            draw < p
        };
        GateDraw { draw, passed }
    }
}

const fn splitmix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::SeededRng;
    use monoagens_protocol::EventId;

    #[test]
    fn draw_is_deterministic_per_seed_and_event_id() {
        let a = SeededRng::draw_unit(42, EventId::new(10));
        let b = SeededRng::draw_unit(42, EventId::new(10));
        let c = SeededRng::draw_unit(42, EventId::new(11));
        let d = SeededRng::draw_unit(43, EventId::new(10));

        assert_eq!(a, b);
        assert!((0.0..1.0).contains(&a));
        assert_ne!(a, c);
        assert_ne!(a, d);

        let g0 = SeededRng::draw_bernoulli(42, EventId::new(10), 0.0);
        assert!(!g0.passed);
        let g1 = SeededRng::draw_bernoulli(42, EventId::new(10), 1.0);
        assert!(g1.passed);
    }
}
