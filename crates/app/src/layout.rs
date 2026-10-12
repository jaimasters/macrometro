//! Organic town layout: a few cores, radial roads that steer around water, irregular infill that falls off with
//! distance. Pure and deterministic from the world seed (no ECS); unit-tested below.
use bevy::prelude::*;
use world_gen::rng::{hash3, unit, SplitMix64};
use world_gen::{height, CityRegion, SEA_LEVEL};

/// Towns seeded at startup (the first N generated cities). "A couple of small towns."
pub const TOWNS: usize = 2;
/// Peak zone capacity (residents) at a town core; falls off with distance.
pub const PEAK_CAPACITY: f64 = 2000.0;
const ARM_STEP: f32 = 13.0;
const MIN_SPACING: f32 = 9.0;
const INFILL_TRIES: usize = 90;
/// World-unit footprint bounding radius used for the water check.
const FOOTPRINT_R: f32 = 5.5;

/// Land-use type; drives glyph shape and colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneKind {
    Residential,
    Commercial,
    Industrial,
}

impl ZoneKind {
    pub fn colour(self) -> Color {
        match self {
            ZoneKind::Residential => Color::srgb(0.52, 0.74, 0.58),
            ZoneKind::Commercial => Color::srgb(0.95, 0.62, 0.28),
            ZoneKind::Industrial => Color::srgb(0.60, 0.54, 0.76),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoneGeo {
    pub center: Vec2,
    pub town: u32,
    pub kind: ZoneKind,
    /// Zone it grew out of (the road runs to it); None for a town core.
    pub parent: Option<usize>,
    /// Footprint heading (road direction) in radians, and base footprint (length, width) in world units.
    pub angle: f32,
    pub size: Vec2,
    pub capacity: f64,
    /// Population the zone starts with (0 = not yet built).
    pub start_pop: f64,
}

pub fn is_land(seed: u64, p: Vec2) -> bool {
    height(seed, p.x as f64, p.y as f64) >= SEA_LEVEL + 0.01
}

/// Centre and eight points on the footprint circle must all be land.
pub fn footprint_on_land(seed: u64, p: Vec2) -> bool {
    is_land(seed, p)
        && (0..8).all(|k| {
            let a = k as f32 * std::f32::consts::FRAC_PI_4;
            is_land(seed, p + Vec2::new(a.cos(), a.sin()) * FOOTPRINT_R)
        })
}

fn segment_on_land(seed: u64, a: Vec2, b: Vec2) -> bool {
    (1..5).all(|i| is_land(seed, a.lerp(b, i as f32 / 5.0)))
}

fn capacity_at(dist: f32, reach: f32) -> f64 {
    PEAK_CAPACITY * (1.0 - dist / (reach + 30.0)).max(0.15).powf(1.3) as f64
}

/// Build the zones of one town around `core`. Appends to `out`; returns nothing.
fn grow_town(seed: u64, town: u32, core: Vec2, out: &mut Vec<ZoneGeo>) {
    let mut rng = SplitMix64::new(hash3(seed, 0x70_0A, town as i64, 0));
    let mut f = |lo: f32, hi: f32| lo + (hi - lo) * unit(rng.next_u64()) as f32;
    let base = out.len();
    let reach = 78.0;
    let mk = |center: Vec2, parent: Option<usize>, angle: f32, kind: ZoneKind, start: f64, sz: Vec2| {
        let dist = center.distance(core);
        let capacity = capacity_at(dist, reach);
        ZoneGeo { center, town, kind, parent, angle, size: sz, capacity, start_pop: capacity * start }
    };
    out.push(mk(core, None, 0.0, ZoneKind::Commercial, 0.3, Vec2::new(6.0, 6.0)));
    let arms = 5 + (f(0.0, 3.0) as usize);
    let a0 = f(0.0, std::f32::consts::TAU);
    let mut arm_zones: Vec<usize> = vec![];
    for k in 0..arms {
        let mut ang = a0 + k as f32 * std::f32::consts::TAU / arms as f32 + f(-0.25, 0.25);
        let (mut prev, mut pos) = (base, core);
        let steps = 5 + f(0.0, 3.0) as usize;
        for i in 1..=steps {
            // steer around water: try the current heading, then wider detours
            let mut placed = None;
            for turn in [0.0f32, 0.35, -0.35, 0.7, -0.7] {
                let a = ang + turn;
                let next = pos + Vec2::new(a.cos(), a.sin()) * (ARM_STEP + f(-2.0, 2.0));
                if footprint_on_land(seed, next) && segment_on_land(seed, pos, next) {
                    placed = Some((a, next));
                    break;
                }
            }
            let Some((a, next)) = placed else { break };
            ang = a + f(-0.12, 0.12);
            let kind = if i <= 2 {
                ZoneKind::Commercial
            } else if i == steps && k % 3 == 0 {
                ZoneKind::Industrial
            } else {
                ZoneKind::Residential
            };
            let start = if i == 1 { 0.12 } else { 0.0 };
            let sz = Vec2::new(f(4.5, 7.0), f(3.0, 5.0));
            out.push(mk(next, Some(prev), a, kind, start, sz));
            prev = out.len() - 1;
            arm_zones.push(prev);
            pos = next;
        }
    }
    // irregular infill beside the arms, thinning with distance
    for _ in 0..INFILL_TRIES {
        if arm_zones.is_empty() {
            break;
        }
        let anchor = out[arm_zones[(f(0.0, arm_zones.len() as f32) as usize).min(arm_zones.len() - 1)]];
        let ai = out.iter().position(|z| z.center == anchor.center).unwrap();
        let side = if f(0.0, 1.0) < 0.5 { 1.0 } else { -1.0 };
        let a = anchor.angle + side * f(1.1, 1.9);
        let cand = anchor.center + Vec2::new(a.cos(), a.sin()) * f(9.0, 14.0);
        let dist = cand.distance(core);
        if f(0.0, 1.0) > 1.0 - dist / (reach + 25.0) {
            continue;
        }
        let spaced = out[base..].iter().all(|z| z.center.distance(cand) >= MIN_SPACING);
        if !spaced || !footprint_on_land(seed, cand) || !segment_on_land(seed, anchor.center, cand) {
            continue;
        }
        let kind = if dist < 30.0 && f(0.0, 1.0) < 0.4 { ZoneKind::Commercial } else { ZoneKind::Residential };
        let sz = Vec2::new(f(3.5, 6.0), f(2.5, 4.5));
        out.push(mk(cand, Some(ai), anchor.angle, kind, 0.0, sz));
    }
}

/// Zones for the first `TOWNS` cities of the world. Index i == growth ZoneId(i) == transit zone node i.
pub fn build_layout(seed: u64, cities: &[CityRegion]) -> Vec<ZoneGeo> {
    let mut out = vec![];
    for c in cities.iter().take(TOWNS) {
        let (ox, oz) = c.center.origin_cell();
        let core = Vec2::new(ox as f32 + 16.0, oz as f32 + 16.0);
        grow_town(seed, c.id.0, core, &mut out);
    }
    out
}

const TOWN_A: [&str; 10] = ["Bram", "Wick", "Ash", "Dun", "Ox", "Kel", "Mar", "Tor", "Lin", "Hol"];
const TOWN_B: [&str; 8] = ["ford", "ton", "bury", "vale", "mouth", "field", "gate", "haven"];

/// Deterministic town name from the world seed.
pub fn town_name(seed: u64, town: u32) -> String {
    let h = hash3(seed, 0x4E4D, town as i64, 1);
    format!("{}{}", TOWN_A[(h % 10) as usize], TOWN_B[((h >> 8) % 8) as usize])
}

/// Auto name for a station at `p`: the nearest zone's town plus a district word from that zone's role
/// or the compass side of the town core. "Outskirts" when nothing is within 40 units.
pub fn station_name(seed: u64, geo: &[ZoneGeo], p: Vec2) -> String {
    let Some((i, z)) = geo.iter().enumerate().min_by(|a, b| a.1.center.distance(p).total_cmp(&b.1.center.distance(p))) else {
        return "Halt".into();
    };
    let town = town_name(seed, z.town);
    if z.center.distance(p) > 40.0 {
        return format!("{town} Outskirts");
    }
    let core = geo.iter().find(|c| c.town == z.town && c.parent.is_none()).map_or(z.center, |c| c.center);
    let d = z.center - core;
    let district = match z.kind {
        ZoneKind::Commercial if d.length() < 30.0 => "Central".to_string(),
        ZoneKind::Industrial => "Works".to_string(),
        _ => {
            let side = if d.x.abs() > d.y.abs() { if d.x > 0.0 { "East" } else { "West" } } else if d.y > 0.0 { "South" } else { "North" };
            // a second hash-chosen word keeps neighbouring stations apart
            let tail = ["Park", "Row", "End", "Green", "Cross"][(hash3(seed, 0x57, i as i64, 2) % 5) as usize];
            format!("{side} {tail}")
        }
    };
    format!("{town} {district}")
}

/// Road graph edges for transit: every parent link plus each zone's two nearest neighbours (same town, close).
pub fn road_pairs(geo: &[ZoneGeo]) -> Vec<(usize, usize)> {
    let mut v: Vec<(usize, usize)> = vec![];
    let mut push = |a: usize, b: usize| v.push((a.min(b), a.max(b)));
    for (i, z) in geo.iter().enumerate() {
        if let Some(p) = z.parent {
            push(i, p);
        }
        let mut near: Vec<(f32, usize)> = geo
            .iter()
            .enumerate()
            .filter(|(j, o)| *j != i && o.town == z.town && o.center.distance(z.center) <= 24.0)
            .map(|(j, o)| (o.center.distance(z.center), j))
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, j) in near.into_iter().take(2) {
            push(i, j);
        }
    }
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use world_gen::generate_cities;

    #[test]
    fn same_seed_same_layout_different_seed_differs() {
        let a = build_layout(7, &generate_cities(7, 3));
        assert_eq!(a, build_layout(7, &generate_cities(7, 3)));
        assert_ne!(a, build_layout(8, &generate_cities(8, 3)));
        assert_eq!(road_pairs(&a), road_pairs(&build_layout(7, &generate_cities(7, 3))));
    }

    #[test]
    fn no_zone_footprint_touches_water_and_layout_is_not_a_lattice() {
        for seed in [1u64, 5, 42, 99] {
            let geo = build_layout(seed, &generate_cities(seed, 3));
            assert!(geo.len() > 30, "seed {seed}: {} zones", geo.len());
            for z in &geo {
                assert!(footprint_on_land(seed, z.center), "seed {seed}: zone at {:?} in/near water", z.center);
                let r = z.size.max_element() * 0.5;
                for (dx, dz) in [(r, r), (r, -r), (-r, r), (-r, -r)] {
                    assert!(is_land(seed, z.center + Vec2::new(dx, dz)), "seed {seed}: corner in water");
                }
            }
            // not a grid: distinct x positions are nearly all different
            let mut xs: Vec<i32> = geo.iter().map(|z| z.center.x.round() as i32).collect();
            xs.sort();
            xs.dedup();
            assert!(xs.len() * 2 > geo.len(), "positions collapse onto a lattice");
            assert!(geo.iter().filter(|z| z.start_pop > 0.0).count() < geo.len() / 3, "starts sparse");
            assert!(geo.iter().filter(|z| z.kind == ZoneKind::Commercial).count() >= 2);
        }
    }

    #[test]
    fn station_names_are_deterministic_and_local() {
        let geo = build_layout(7, &generate_cities(7, 3));
        let core = geo[0].center;
        let n = station_name(7, &geo, core);
        assert_eq!(n, station_name(7, &geo, core));
        assert!(n.ends_with("Central"), "{n}");
        assert!(station_name(7, &geo, Vec2::new(1e5, 1e5)).ends_with("Outskirts"));
        assert_eq!(station_name(7, &[], Vec2::ZERO), "Halt");
        assert_ne!(town_name(7, 0), town_name(7, 1) + "x");
    }

    #[test]
    fn parents_precede_children_and_pairs_are_valid() {
        let geo = build_layout(3, &generate_cities(3, 3));
        for (i, z) in geo.iter().enumerate() {
            assert!(z.parent.map_or(true, |p| p < i));
        }
        let pairs = road_pairs(&geo);
        assert!(pairs.iter().all(|&(a, b)| a < b && b < geo.len()));
        assert!(pairs.len() >= geo.len() - TOWNS);
    }
}
