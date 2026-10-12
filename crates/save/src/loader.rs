//! Serialise / load with version-gated migrations.
use crate::format::{SaveFile, MAGIC};
use ron::Value;
use serde::Deserialize;

#[derive(Debug, PartialEq)]
pub enum LoadError {
    Parse(String),
    BadMagic(String),
    /// File is newer than this build understands.
    FutureVersion { found: u32, supported: u32 },
    /// version 0 or no migration registered.
    UnsupportedVersion(u32),
    Migration { from: u32, msg: String },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{self:?}") }
}
impl std::error::Error for LoadError {}

/// One step: transforms a version-v document into version v+1 (the loader bumps header.version).
pub type Migration = fn(Value) -> Result<Value, String>;

/// MIGRATIONS[v-1] migrates v -> v+1. Empty while CURRENT_VERSION == 1. Append one per format bump.
pub const MIGRATIONS: &[Migration] = &[];

#[derive(Deserialize)]
struct Probe { header: ProbeHeader }
#[derive(Deserialize)]
struct ProbeHeader { magic: String, version: u32 }

/// Canonical text form: fixed pretty config so equal data => identical bytes.
pub fn to_ron(save: &SaveFile) -> String {
    let cfg = ron::ser::PrettyConfig::new().indentor("  ".to_string()).new_line("\n".to_string());
    ron::ser::to_string_pretty(save, cfg).expect("SaveFile serialises") + "\n"
}

pub fn from_ron(text: &str) -> Result<SaveFile, LoadError> {
    load_with(text, MIGRATIONS)
}

/// Loader against an explicit migration table (exposed for tests; target = table.len()+1).
pub fn load_with(text: &str, table: &[Migration]) -> Result<SaveFile, LoadError> {
    let target = table.len() as u32 + 1;
    let probe: Probe = ron::from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
    if probe.header.magic != MAGIC { return Err(LoadError::BadMagic(probe.header.magic)); }
    let v = probe.header.version;
    if v > target { return Err(LoadError::FutureVersion { found: v, supported: target }); }
    if v == 0 { return Err(LoadError::UnsupportedVersion(v)); }
    let mut doc: Value = ron::from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
    for from in v..target {
        doc = table[(from - 1) as usize](doc).map_err(|msg| LoadError::Migration { from, msg })?;
        set_version(&mut doc, from + 1).map_err(|msg| LoadError::Migration { from, msg })?;
    }
    doc.into_rust().map_err(|e| LoadError::Parse(e.to_string()))
}

fn set_version(doc: &mut Value, v: u32) -> Result<(), String> {
    let Value::Map(root) = doc else { return Err("root not a struct".into()) };
    let Some(Value::Map(h)) = root.get_mut(&Value::String("header".into())) else { return Err("no header".into()) };
    h.insert(Value::String("version".into()), Value::Number(ron::value::Number::from(v as i64)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::*; // also brings CURRENT_VERSION for tests

    fn sample() -> SaveFile {
        let mut s = SaveFile::new(7, 100);
        s.world.chunk_overrides.push(ChunkOverride { x: -1, z: 2, height_delta_mm: -300 });
        s.cities.push(CitySave {
            id: 0,
            region: RegionSave { center: (3, -4), radius: 2 },
            zones: vec![ZoneSave { id: 1, chunk: (3, -4), kind: "res".into(), population: 50 }],
            network: NetworkSave { nodes: vec![(0, 0), (1, 1)], edges: vec![(0, 1)] },
            lines: vec![LineSave { id: 0, rail_class: "Tram".into(), stops: vec![0, 1] }],
            metrics: MetricsSave { population: 50, funds: -10 },
        });
        s
    }

    #[test]
    fn roundtrip_byte_identical() {
        let a = to_ron(&sample());
        let loaded = from_ron(&a).unwrap();
        assert_eq!(loaded, sample());
        assert_eq!(to_ron(&loaded), a);
    }

    #[test]
    fn loader_rejects_future_version() {
        let text = to_ron(&sample()).replace("version: 1", "version: 2");
        assert_eq!(from_ron(&text), Err(LoadError::FutureVersion { found: 2, supported: CURRENT_VERSION }));
    }

    #[test]
    fn loader_rejects_bad_magic_and_garbage() {
        let text = to_ron(&sample()).replace("MMSAVE", "NOPE");
        assert!(matches!(from_ron(&text), Err(LoadError::BadMagic(_))));
        assert!(matches!(from_ron("not ron {"), Err(LoadError::Parse(_))));
        let v0 = to_ron(&sample()).replace("version: 1", "version: 0");
        assert_eq!(from_ron(&v0), Err(LoadError::UnsupportedVersion(0)));
    }

    #[test]
    fn migrations_chain_applies_in_order() {
        // Fake history: v1 stored `tick`; pretend v2 moved it +1 and v3 doubles it.
        fn bump_tick(v: Value, f: impl Fn(u64) -> u64) -> Result<Value, String> {
            let Value::Map(mut root) = v else { return Err("root".into()) };
            let hk = Value::String("header".into());
            let Some(Value::Map(h)) = root.get_mut(&hk) else { return Err("hdr".into()) };
            let tk = Value::String("tick".into());
            let Some(Value::Number(n)) = h.get(&tk).cloned() else { return Err("tick".into()) };
            let cur = n.into_f64() as u64;
            h.insert(tk, Value::Number(ron::value::Number::from(f(cur) as i64)));
            Ok(Value::Map(root))
        }
        let table: &[Migration] = &[|v| bump_tick(v, |t| t + 1), |v| bump_tick(v, |t| t * 2)];
        let loaded = load_with(&to_ron(&sample()), table).unwrap(); // v1 -> v3
        assert_eq!(loaded.header.version, 3);
        assert_eq!(loaded.header.tick, (100 + 1) * 2);
        // A v2 file only gets the second step.
        let v2 = to_ron(&sample()).replace("version: 1", "version: 2");
        assert_eq!(load_with(&v2, table).unwrap().header.tick, 200);
        // Too-new for this table is rejected.
        let v4 = to_ron(&sample()).replace("version: 1", "version: 4");
        assert!(matches!(load_with(&v4, table), Err(LoadError::FutureVersion { .. })));
    }
}
