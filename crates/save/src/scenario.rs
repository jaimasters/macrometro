//! Seeded scenario generator. world_gen data is real; the sim run is a STUB: zones/lines/metrics are synthesised
//! from a seeded RNG instead of running the headless sim (TODO when sim_core/growth expose a headless runner).
use crate::format::*;
use world_gen::rng::{mix, SplitMix64};
use world_gen::{generate_cities, CityRegion};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario { Fresh, MidGrowth, SprawlCrisis }

impl Scenario {
    pub const ALL: [Scenario; 3] = [Scenario::Fresh, Scenario::MidGrowth, Scenario::SprawlCrisis];
    pub fn name(self) -> &'static str {
        match self { Scenario::Fresh => "fresh", Scenario::MidGrowth => "mid_growth", Scenario::SprawlCrisis => "sprawl_crisis" }
    }
    /// (cities, tick, zone fill percent of region chunks, lines per city, funds)
    fn params(self) -> (usize, u64, u64, u32, i64) {
        match self {
            Scenario::Fresh => (1, 0, 5, 0, 10_000),
            Scenario::MidGrowth => (2, 3_600, 40, 2, 4_000),
            Scenario::SprawlCrisis => (3, 14_400, 90, 5, -2_500),
        }
    }
}

const KINDS: [&str; 3] = ["res", "com", "ind"];
const CLASSES: [&str; 5] = ["Tram", "LightRail", "HeavyRail", "HighSpeed", "DoubleDecker"];

/// Deterministic: same (scenario, seed, FORMAT version) => equal SaveFile.
pub fn generate(s: Scenario, seed: u64) -> SaveFile {
    let (n, tick, fill, lines, funds) = s.params();
    let mut rng = SplitMix64::new(mix(seed ^ (s as u64 + 1) * 0x5CE4_A210));
    let mut save = SaveFile::new(seed, tick);
    for region in generate_cities(seed, n) {
        save.cities.push(city(&region, &mut rng, fill, lines, funds));
    }
    save
}

fn city(r: &CityRegion, rng: &mut SplitMix64, fill: u64, lines: u32, funds: i64) -> CitySave {
    let mut zones = vec![];
    let mut nodes = vec![];
    for c in r.chunks() {
        if rng.below(100) >= fill { continue; }
        let id = zones.len() as u32;
        zones.push(ZoneSave { id, chunk: (c.x, c.z), kind: KINDS[rng.below(3) as usize].into(), population: 10 + rng.below(990) as u32 });
        if rng.below(4) == 0 { nodes.push((c.x, c.z)); }
    }
    let edges = (1..nodes.len() as u32).map(|i| (i - 1, i)).collect();
    let stop_pool = nodes.len() as u64;
    let lines = if stop_pool < 2 { vec![] } else {
        (0..lines).map(|id| LineSave {
            id,
            rail_class: CLASSES[rng.below(5) as usize].into(),
            stops: (0..2 + rng.below(stop_pool.min(5) - 1)).map(|_| rng.below(stop_pool) as u32).collect(),
        }).collect()
    };
    let population = zones.iter().map(|z| z.population as u64).sum();
    CitySave {
        id: r.id.0,
        region: RegionSave { center: (r.center.x, r.center.z), radius: r.radius },
        zones,
        network: NetworkSave { nodes, edges },
        lines,
        metrics: MetricsSave { population, funds },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::{from_ron, to_ron};

    #[test]
    fn gen_scenarios_deterministic() {
        for s in Scenario::ALL {
            assert_eq!(to_ron(&generate(s, 42)), to_ron(&generate(s, 42)), "{}", s.name());
            assert_ne!(to_ron(&generate(s, 42)), to_ron(&generate(s, 43)), "{}", s.name());
        }
    }

    #[test]
    fn scenarios_shape_and_roundtrip() {
        let sizes: Vec<_> = Scenario::ALL.iter().map(|s| generate(*s, 1)).collect();
        assert_eq!(sizes.iter().map(|f| f.cities.len()).collect::<Vec<_>>(), [1, 2, 3]);
        assert!(sizes[0].header.tick < sizes[1].header.tick && sizes[1].header.tick < sizes[2].header.tick);
        for f in &sizes {
            assert_eq!(from_ron(&to_ron(f)).unwrap(), *f);
            assert!(f.cities.iter().all(|c| c.lines.iter().all(|l| l.stops.iter().all(|s| (*s as usize) < c.network.nodes.len()))));
        }
    }
}
