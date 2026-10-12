//! Seeded, platform-independent hashing/RNG. No external RNG: output must never change across builds.

/// SplitMix64 finaliser: a strong 64-bit mix.
pub fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Hash a lattice point under a seed and a salt (salt separates independent streams).
pub fn hash3(seed: u64, salt: u64, x: i64, z: i64) -> u64 {
    mix(mix(mix(seed ^ salt.wrapping_mul(0xD6E8_FEB8_6659_FD93)) ^ x as u64) ^ z as u64)
}

/// Uniform in [0,1) from a hash (53 bits).
pub fn unit(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Tiny sequential RNG (SplitMix64) for scenario generation.
#[derive(Debug, Clone)]
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    pub fn new(seed: u64) -> Self { Self(seed) }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix(self.0.wrapping_sub(0x9E37_79B9_7F4A_7C15))
    }
    /// Uniform integer in [0, n). n must be > 0.
    pub fn below(&mut self, n: u64) -> u64 { self.next_u64() % n }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rng_golden_values() {
        // Pinned: changing these breaks every saved seed.
        assert_eq!(mix(0), 0xE220_A839_7B1D_CDAF);
        let mut r = SplitMix64::new(0);
        assert_eq!(r.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_ne!(r.next_u64(), r.next_u64());
        let u = unit(hash3(1, 2, 3, 4));
        assert!((0.0..1.0).contains(&u));
    }
}
