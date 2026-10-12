//! Save data model (RON, serde). Generated terrain is NOT stored: it regenerates from `seed`.
//! Sim-owned sections (zones/network/lines/metrics) are STUBS until sim/transit/economy publish their types.
use serde::{Deserialize, Serialize};

pub const MAGIC: &str = "MMSAVE";
/// Current on-disk version. Equals `MIGRATIONS.len() + 1` (see loader).
pub const CURRENT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header { pub magic: String, pub version: u32, pub seed: u64, pub tick: u64 }

/// Player edit of generated terrain at one chunk (placeholder payload).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkOverride { pub x: i64, pub z: i64, pub height_delta_mm: i32 }

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WorldSection { pub seed: u64, pub chunk_overrides: Vec<ChunkOverride> }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionSave { pub center: (i64, i64), pub radius: u32 }

// ---- STUB sim sections (owners: growth / transit / economy). ----
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoneSave { pub id: u32, pub chunk: (i64, i64), pub kind: String, pub population: u32 }
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NetworkSave { pub nodes: Vec<(i64, i64)>, pub edges: Vec<(u32, u32)> }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineSave { pub id: u32, pub rail_class: String, pub stops: Vec<u32> }
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MetricsSave { pub population: u64, pub funds: i64 }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitySave {
    pub id: u32,
    pub region: RegionSave,
    pub zones: Vec<ZoneSave>,
    pub network: NetworkSave,
    pub lines: Vec<LineSave>,
    pub metrics: MetricsSave,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveFile { pub header: Header, pub world: WorldSection, pub cities: Vec<CitySave> }

impl SaveFile {
    pub fn new(seed: u64, tick: u64) -> Self {
        Self {
            header: Header { magic: MAGIC.into(), version: CURRENT_VERSION, seed, tick },
            world: WorldSection { seed, chunk_overrides: vec![] },
            cities: vec![],
        }
    }
}
