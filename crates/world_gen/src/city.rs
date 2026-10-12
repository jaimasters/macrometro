//! City regions: deterministic placement on a coarse super-grid, one candidate per super-cell.
use crate::chunk::{ChunkCoord, CHUNK_SIZE};
use crate::rng::{hash3, unit};
use crate::terrain::{height, SEA_LEVEL};
use serde::{Deserialize, Serialize};

/// Identifier of a city within the shared logical world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CityId(pub u32);

/// Super-cell edge in chunks. Centres keep a 4-chunk margin and radius <= 4, so regions never overlap.
const SUPER: i64 = 24;
const MARGIN: i64 = 4;
const SPAWN_CHANCE: f64 = 0.6;
const MAX_RING: i64 = 64;

/// Square (Chebyshev) region of chunks around a centre chunk (LOD 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CityRegion { pub id: CityId, pub center: ChunkCoord, pub radius: u32 }

impl CityRegion {
    pub fn contains(&self, c: ChunkCoord) -> bool {
        c.lod == 0
            && (c.x - self.center.x).unsigned_abs() <= self.radius as u64
            && (c.z - self.center.z).unsigned_abs() <= self.radius as u64
    }
    /// All chunks of the region, row-major (z major).
    pub fn chunks(&self) -> impl Iterator<Item = ChunkCoord> + '_ {
        let r = self.radius as i64;
        (-r..=r).flat_map(move |dz| (-r..=r).map(move |dx| ChunkCoord::new(self.center.x + dx, self.center.z + dz)))
    }
}

fn is_land(seed: u64, c: ChunkCoord) -> bool {
    let (ox, oz) = c.origin_cell();
    let m = (CHUNK_SIZE / 2) as i64;
    let (cx, cz) = ((ox + m) as f64, (oz + m) as f64);
    // Centre and four points 1.5 chunks out must all be buildable land.
    let d = 1.5 * CHUNK_SIZE as f64;
    [(0.0, 0.0), (d, 0.0), (-d, 0.0), (0.0, d), (0.0, -d)]
        .iter()
        .all(|(dx, dz)| height(seed, cx + dx, cz + dz) >= SEA_LEVEL + 0.03)
}

/// Candidate for a super-cell (without id), or None if the cell has no city.
fn candidate(seed: u64, sx: i64, sz: i64) -> Option<(ChunkCoord, u32)> {
    let h = hash3(seed, 0xC17E, sx, sz);
    if unit(h) >= SPAWN_CHANCE { return None; }
    let span = (SUPER - 2 * MARGIN) as u64;
    let c = ChunkCoord::new(
        sx * SUPER + MARGIN + (hash3(seed, 0xC17F, sx, sz) % span) as i64,
        sz * SUPER + MARGIN + (hash3(seed, 0xC180, sx, sz) % span) as i64,
    );
    is_land(seed, c).then(|| (c, 2 + (hash3(seed, 0xC181, sx, sz) % 3) as u32))
}

/// The first `n` cities in outward spiral order from the origin (ids 0..n). May return fewer if land runs out.
pub fn generate_cities(seed: u64, n: usize) -> Vec<CityRegion> {
    let mut out = Vec::with_capacity(n);
    for ring in 0..=MAX_RING {
        for sz in -ring..=ring {
            for sx in -ring..=ring {
                if sx.abs().max(sz.abs()) != ring { continue; }
                if out.len() == n { return out; }
                if let Some((center, radius)) = candidate(seed, sx, sz) {
                    out.push(CityRegion { id: CityId(out.len() as u32), center, radius });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cities_deterministic() {
        assert_eq!(generate_cities(11, 5), generate_cities(11, 5));
        assert_ne!(generate_cities(11, 5), generate_cities(12, 5));
    }

    #[test]
    fn cities_prefix_stable() {
        let a = generate_cities(3, 6);
        assert_eq!(a.len(), 6);
        assert_eq!(&generate_cities(3, 3)[..], &a[..3]);
        assert!(a.iter().enumerate().all(|(i, c)| c.id == CityId(i as u32)));
    }

    #[test]
    fn cities_disjoint_and_on_land() {
        for seed in [0u64, 1, 99] {
            let cs = generate_cities(seed, 8);
            assert_eq!(cs.len(), 8);
            for (i, a) in cs.iter().enumerate() {
                assert!(is_land(seed, a.center));
                for b in &cs[i + 1..] {
                    assert!(!a.chunks().any(|c| b.contains(c)), "overlap {a:?} {b:?}");
                }
            }
        }
    }

    #[test]
    fn region_contains_matches_chunks() {
        let r = CityRegion { id: CityId(0), center: ChunkCoord::new(-5, 7), radius: 2 };
        assert_eq!(r.chunks().count(), 25);
        assert!(r.chunks().all(|c| r.contains(c)));
        assert!(!r.contains(ChunkCoord::new(-8, 7)));
        assert!(!r.contains(ChunkCoord { lod: 1, ..ChunkCoord::new(-5, 7) }));
    }
}
