//! Deterministic terrain height: value-noise fBm over world cell coordinates. Pure; f64 add/mul only.
use crate::rng::{hash3, unit};

/// Heights below this are water (not buildable).
pub const SEA_LEVEL: f32 = 0.35;
const OCTAVES: u32 = 4;
/// Feature size of the lowest-frequency octave, in cells.
const BASE_WAVELENGTH: f64 = 96.0;

fn smooth(t: f64) -> f64 { t * t * (3.0 - 2.0 * t) }

fn lattice(seed: u64, octave: u32, ix: i64, iz: i64) -> f64 {
    unit(hash3(seed, 0x7E44 + octave as u64, ix, iz))
}

fn value_noise(seed: u64, octave: u32, x: f64, z: f64) -> f64 {
    let (fx, fz) = (x.floor(), z.floor());
    let (ix, iz) = (fx as i64, fz as i64);
    let (tx, tz) = (smooth(x - fx), smooth(z - fz));
    let a = lattice(seed, octave, ix, iz);
    let b = lattice(seed, octave, ix + 1, iz);
    let c = lattice(seed, octave, ix, iz + 1);
    let d = lattice(seed, octave, ix + 1, iz + 1);
    let top = a + (b - a) * tx;
    let bot = c + (d - c) * tx;
    top + (bot - top) * tz
}

/// Terrain height in [0,1) at world cell coordinates (x,z). Continuous, seed-dependent.
pub fn height(seed: u64, x: f64, z: f64) -> f32 {
    let (mut sum, mut amp, mut norm, mut freq) = (0.0, 1.0, 0.0, 1.0 / BASE_WAVELENGTH);
    for o in 0..OCTAVES {
        sum += amp * value_noise(seed, o, x * freq, z * freq);
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    (sum / norm).min(0.999_999) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_deterministic() {
        assert_eq!(height(42, 10.5, -3.25), height(42, 10.5, -3.25));
        assert_eq!(height(42, 1e6, -1e6), height(42, 1e6, -1e6));
    }

    #[test]
    fn terrain_seed_sensitive() {
        let diff = (0..50).filter(|i| height(1, *i as f64 * 7.0, 3.0) != height(2, *i as f64 * 7.0, 3.0)).count();
        assert!(diff > 40);
    }

    #[test]
    fn terrain_in_range_and_continuous() {
        let mut prev = height(7, -500.0, 12.0);
        for i in -499..500 {
            let h = height(7, i as f64, 12.0);
            assert!((0.0..1.0).contains(&h));
            assert!((h - prev).abs() < 0.1, "jump at {i}");
            prev = h;
        }
    }

    #[test]
    fn terrain_has_land_and_water() {
        let (mut land, mut water) = (false, false);
        for i in 0..200 {
            for j in 0..200 {
                if height(5, i as f64 * 10.0, j as f64 * 10.0) >= SEA_LEVEL { land = true } else { water = true }
            }
        }
        assert!(land && water);
    }
}
