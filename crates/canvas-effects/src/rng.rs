//! A small seeded PRNG (xorshift64*), so every effect has reproducible randomness.
//!
//! Each effect owns one generator. In normal use the seed comes from entropy; in tests
//! the seed is fixed, which makes every effect's output deterministic.

/// xorshift64* generator. Cheap, no allocation, good enough for visual effects.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Seeds the generator. A zero seed is replaced by a fixed non-zero constant, since
    /// xorshift cannot leave the zero state.
    pub fn new(seed: u64) -> Self {
        Self { state: if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed } }
    }

    /// Seeds from the OS entropy source. Falls back to a fixed constant if the OS
    /// source fails, which only costs determinism, never correctness.
    pub fn from_entropy() -> Self {
        use std::collections::hash_map::RandomState;
        use std::hash::{BuildHasher, Hasher};
        // RandomState's keys are seeded by the OS, and differ per process.
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(0x9E37_79B9_7F4A_7C15);
        Self::new(hasher.finish())
    }

    /// The raw xorshift64* step.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in 0..1.
    pub fn next_f32(&mut self) -> f32 {
        // 24 bits of mantissa gives a float in [0, 1).
        ((self.next_u64() >> 40) as u32) as f32 / (1u32 << 24) as f32
    }

    /// Uniform in `lo..hi`.
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next_f32()
    }

    /// Uniform integer in `0..n` (`n > 0`).
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        (self.next_u64() % u64::from(n)) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_seed_is_replaced() {
        let mut a = Rng::new(0);
        let mut b = Rng::new(0x9E37_79B9_7F4A_7C15);
        assert_eq!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn same_seed_gives_same_sequence() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..64 {
            assert_eq!(a.next_f32(), b.next_f32());
        }
    }

    #[test]
    fn values_stay_in_range() {
        let mut r = Rng::new(7);
        for _ in 0..10_000 {
            let v = r.next_f32();
            assert!((0.0..1.0).contains(&v), "out of range: {v}");
            let w = r.range(-3.0, 5.0);
            assert!((-3.0..5.0).contains(&w), "out of range: {w}");
            assert!(r.below(4) < 4);
        }
    }

    #[test]
    fn distribution_is_roughly_flat() {
        let mut r = Rng::new(1234);
        let mut buckets = [0u32; 10];
        for _ in 0..100_000 {
            buckets[(r.next_f32() * 10.0) as usize] += 1;
        }
        for b in buckets {
            assert!((8_500..11_500).contains(&b), "skewed bucket: {b}");
        }
    }
}
