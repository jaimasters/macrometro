//! Chunk addressing and per-chunk height field. Chunks are views of the world function, so neighbours always agree.
use crate::terrain::height;
use serde::{Deserialize, Serialize};

/// Cells per chunk edge at LOD 0.
pub const CHUNK_SIZE: u32 = 32;

/// Integer chunk coordinate at a given level of detail (0 = finest).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChunkCoord { pub x: i64, pub z: i64, pub lod: u8 }

impl ChunkCoord {
    pub fn new(x: i64, z: i64) -> Self { Self { x, z, lod: 0 } }
    /// Chunk containing world cell (cx, cz) at LOD 0.
    pub fn from_cell(cx: i64, cz: i64) -> Self {
        Self::new(cx.div_euclid(CHUNK_SIZE as i64), cz.div_euclid(CHUNK_SIZE as i64))
    }
    /// World cell coordinate of this chunk's min corner (LOD 0 grid).
    pub fn origin_cell(&self) -> (i64, i64) {
        let s = (CHUNK_SIZE as i64) << self.lod;
        (self.x * s, self.z * s)
    }
}

/// CHUNK_SIZE x CHUNK_SIZE heights, row-major (z major). LOD n samples every 2^n cells.
pub fn chunk_heights(seed: u64, c: ChunkCoord) -> Vec<f32> {
    let (ox, oz) = c.origin_cell();
    let step = 1i64 << c.lod;
    let n = CHUNK_SIZE as i64;
    let mut v = Vec::with_capacity((n * n) as usize);
    for j in 0..n {
        for i in 0..n {
            v.push(height(seed, (ox + i * step) as f64, (oz + j * step) as f64));
        }
    }
    v
}

/// FNV-1a over height bit patterns: cheap stable fingerprint for determinism tests.
pub fn chunk_hash(seed: u64, c: ChunkCoord) -> u64 {
    chunk_heights(seed, c).iter().fold(0xCBF2_9CE4_8422_2325u64, |h, f| {
        (h ^ f.to_bits() as u64).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_hash_deterministic_and_seeded() {
        let c = ChunkCoord::new(3, -2);
        assert_eq!(chunk_hash(9, c), chunk_hash(9, c));
        assert_ne!(chunk_hash(9, c), chunk_hash(10, c));
        assert_ne!(chunk_hash(9, c), chunk_hash(9, ChunkCoord::new(4, -2)));
    }

    #[test]
    fn chunk_hash_golden() {
        // Pinned fingerprint: update only with a deliberate world-format bump (see DECISIONS D-2).
        assert_eq!(chunk_hash(1, ChunkCoord::new(0, 0)), GOLDEN);
    }
    const GOLDEN: u64 = 10748460090843694632;

    #[test]
    fn chunk_edges_match_neighbour() {
        let (a, b) = (ChunkCoord::new(0, 0), ChunkCoord::new(1, 0));
        let ha = chunk_heights(4, a);
        let hb = chunk_heights(4, b);
        // Neighbour's first column equals world function at a's would-be column 32.
        for j in 0..CHUNK_SIZE as usize {
            assert_eq!(hb[j * CHUNK_SIZE as usize], height(4, 32.0, j as f64));
        }
        assert_eq!(ha.len(), (CHUNK_SIZE * CHUNK_SIZE) as usize);
    }

    #[test]
    fn from_cell_negative() {
        assert_eq!(ChunkCoord::from_cell(-1, 0), ChunkCoord::new(-1, 0));
        assert_eq!(ChunkCoord::from_cell(32, 31), ChunkCoord::new(1, 0));
        assert_eq!(ChunkCoord::new(-1, 1).origin_cell(), (-32, 32));
    }
}
