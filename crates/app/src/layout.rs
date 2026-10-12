//! Town layout = the city crate's generated road graph and parcels (docs/arch/city-gen.md). This file keeps the
//! app-facing names: the planning entry point, town and station names.
use bevy::prelude::*;
use world_gen::rng::hash3;
use world_gen::CityRegion;

pub use city::{Parcel as ZoneGeo, TownPlan, ZoneKind};

/// Towns seeded at startup (the first N generated cities). "A couple of small towns."
pub const TOWNS: usize = 2;

/// Plan the first `TOWNS` cities: road graph + zone parcels (parcel index == growth ZoneId).
pub fn plan_layout(seed: u64, cities: &[CityRegion]) -> TownPlan {
    match (crate::persist::fresh_preset(), cities.first()) {
        // R43: after a reset, one fresh town from the chosen preset and a new seed, at the first city's core
        (Some((preset, s)), Some(c)) if !cfg!(test) => {
            let (ox, oz) = c.center.origin_cell();
            city::organic::plan_preset(s, Vec2::new(ox as f32 + 16.0, oz as f32 + 16.0), preset)
        }
        _ => city::plan_towns(seed, &cities[..cities.len().min(TOWNS)]),
    }
}

/// Zone parcels only (parcel index == growth ZoneId).
pub fn build_layout(seed: u64, cities: &[CityRegion]) -> Vec<ZoneGeo> {
    plan_layout(seed, cities).parcels
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
    let core = geo.iter().find(|c| c.town == z.town && c.core).map_or(z.center, |c| c.center);
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

#[cfg(test)]
mod tests {
    use super::*;
    use world_gen::generate_cities;

    #[test]
    fn same_seed_same_layout_different_seed_differs() {
        let a = plan_layout(7, &generate_cities(7, 3));
        assert_eq!(a, plan_layout(7, &generate_cities(7, 3)));
        assert_ne!(a, plan_layout(8, &generate_cities(8, 3)));
    }

    #[test]
    fn layout_has_zones_in_every_land_use() {
        for seed in [1u64, 5, 42, 99] {
            let geo = build_layout(seed, &generate_cities(seed, 3));
            assert!(geo.len() > 60, "seed {seed}: {} zones", geo.len());
            assert!(geo.iter().filter(|z| z.start_pop > 0.0).count() < geo.len() / 3, "starts sparse");
            assert!(geo.iter().filter(|z| z.kind == ZoneKind::Commercial).count() >= 2);
            assert_eq!(geo.iter().filter(|z| z.core).count(), TOWNS.min(2));
        }
    }

    #[test]
    fn station_names_are_deterministic_and_local() {
        let geo = build_layout(7, &generate_cities(7, 3));
        let core = geo.iter().find(|z| z.core).unwrap().center;
        let n = station_name(7, &geo, core);
        assert_eq!(n, station_name(7, &geo, core));
        assert!(n.ends_with("Central"), "{n}");
        assert!(station_name(7, &geo, Vec2::new(1e5, 1e5)).ends_with("Outskirts"));
        assert_eq!(station_name(7, &[], Vec2::ZERO), "Halt");
        assert_ne!(town_name(7, 0), town_name(7, 1) + "x");
    }
}
