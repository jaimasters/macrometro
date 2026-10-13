use super::*;
use world_gen::generate_cities;

const SEEDS: [u64; 6] = [1, 5, 7, 42, 99, 123];

fn plan(seed: u64) -> TownPlan {
    plan_towns(seed, &generate_cities(seed, 3)[..2])
}

#[test]
fn same_seed_same_graph_different_seed_differs() {
    assert_eq!(plan(7), plan(7));
    assert_ne!(plan(7).graph, plan(8).graph);
}

#[test]
fn no_road_on_water_except_bridges() {
    let mut bridges = 0;
    for seed in SEEDS {
        let p = plan(seed);
        for n in &p.graph.nodes {
            assert!(is_land(seed, *n), "seed {seed}: node {n:?} in water");
        }
        for (i, e) in p.graph.edges.iter().enumerate() {
            let (a, b) = p.graph.pos(i);
            let wr = water_run(seed, a, b);
            if e.bridge {
                bridges += 1;
                assert_eq!(e.class, RoadClass::Arterial);
                assert!(wr > 0.0 && wr <= MAX_BRIDGE_ABS, "seed {seed}: bridge run {wr}");
            } else {
                assert_eq!(wr, 0.0, "seed {seed}: edge {i} crosses water without a bridge");
            }
        }
    }
    eprintln!("bridges across seeds: {bridges}");
}

#[test]
fn towns_are_connected_with_real_structure() {
    for seed in SEEDS {
        let p = plan(seed);
        assert_eq!(p.towns.len(), 2);
        for t in &p.towns {
            let es: Vec<usize> = (0..p.graph.edges.len()).filter(|&i| p.graph.edges[i].town == t.town).collect();
            assert!(es.iter().all(|&i| p.graph.edges[i].order < 9999), "seed {seed}: disconnected edge");
            let art = es.iter().filter(|&&i| p.graph.edges[i].class == RoadClass::Arterial).count();
            let loc = es.len() - art;
            let seeded = es.iter().filter(|&&i| p.graph.edges[i].order <= SEED_ORDER).count();
            let parcels = p.parcels.iter().filter(|z| z.town == t.town).count();
            eprintln!("seed {seed} town {} {:?}: {art} arterial {loc} local edges, {seeded} seed-built, {parcels} parcels", t.town, t.kind);
            assert!(art >= 6, "seed {seed}: only {art} arterial edges");
            assert!(loc >= 40, "seed {seed}: only {loc} local edges");
            assert!(seeded >= 4 && seeded * 5 < es.len() * 3, "seed {seed}: {seeded}/{} seed-built", es.len());
            assert!((40..=600).contains(&parcels), "seed {seed}: {parcels} parcels");
            // junction mix: real towns have T-junctions and some dead ends
            let mut deg = vec![0usize; p.graph.nodes.len()];
            for &i in &es {
                deg[p.graph.edges[i].a as usize] += 1;
                deg[p.graph.edges[i].b as usize] += 1;
            }
            let used: Vec<usize> = deg.iter().copied().filter(|&d| d > 0).collect();
            assert!(used.iter().filter(|&&d| d >= 3).count() >= 10, "seed {seed}: too few junctions");
            assert!(used.iter().filter(|&&d| d == 1).count() >= 2, "seed {seed}: no cul-de-sacs");
        }
    }
}

#[test]
fn parcels_are_on_land_clear_of_roads_and_have_access() {
    for seed in SEEDS {
        let p = plan(seed);
        for z in &p.parcels {
            assert!(footprint_on_land(seed, z.center), "seed {seed}: parcel in water");
            let (a, b) = p.graph.pos(z.access);
            assert!(seg_dist(z.center, a, b) < 7.0, "seed {seed}: frontage too far");
            for i in 0..p.graph.edges.len() {
                let (a, b) = p.graph.pos(i);
                assert!(seg_dist(z.center, a, b) >= 1.3 + z.size.y * 0.5 - 1e-3, "seed {seed}: parcel sits on road {i}");
            }
        }
        for (i, z) in p.parcels.iter().enumerate() {
            for o in &p.parcels[i + 1..] {
                assert!(z.center.distance(o.center) >= PARCEL_SPACING - 0.6, "seed {seed}: parcels overlap");
            }
        }
        // seed zones have built frontage
        for z in p.parcels.iter().filter(|z| z.start_pop > 0.0) {
            assert!(p.graph.edges[z.access].order <= SEED_ORDER || z.core, "seed {seed}: seeded zone on unbuilt road");
        }
    }
}

#[test]
fn land_use_follows_access() {
    let (mut com_art, mut com, mut res_art, mut res, mut ind_far) = (0, 0, 0, 0, 0);
    let mut ind = 0;
    for seed in SEEDS {
        let p = plan(seed);
        for z in &p.parcels {
            let art = p.graph.edges[z.access].class == RoadClass::Arterial;
            let core = p.towns.iter().find(|t| t.town == z.town).unwrap().core;
            match z.kind {
                ZoneKind::Commercial => {
                    com += 1;
                    com_art += art as usize;
                }
                ZoneKind::Residential => {
                    res += 1;
                    res_art += art as usize;
                }
                ZoneKind::Industrial => {
                    ind += 1;
                    ind_far += (z.center.distance(core) > 20.0) as usize;
                }
            }
        }
    }
    eprintln!("commercial {com_art}/{com} on arterials, residential {res_art}/{res}, industrial {ind} ({ind_far} away from core)");
    assert!(com >= 12 && res > com && ind >= 1, "land-use mix degenerate");
    assert!(com_art as f64 / com as f64 > res_art as f64 / res as f64 + 0.1, "commercial is not concentrated on arterials");
    assert_eq!(ind, ind_far, "industry in the core");
}

#[test]
fn seed_kinds_are_assigned() {
    let kinds: Vec<SeedKind> = (0..40u64).flat_map(|s| plan(s).towns.into_iter().map(|t| t.kind)).collect();
    eprintln!("river crossings: {} of {}", kinds.iter().filter(|k| **k == SeedKind::RiverCrossing).count(), kinds.len());
    assert!(kinds.contains(&SeedKind::Crossroads));
}

/// Debug aid: MM_DUMP=/path.txt dumps seed 7 geometry as text lines for plotting. Inert otherwise.
#[test]
fn dump_for_plotting() {
    let Ok(path) = std::env::var("MM_DUMP") else { return };
    let seed: u64 = std::env::var("MM_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(7);
    let p = plan(seed);
    let mut s = String::new();
    for (i, e) in p.graph.edges.iter().enumerate() {
        let (a, b) = p.graph.pos(i);
        s += &format!("E {} {} {} {} {} {} {}\n", a.x, a.y, b.x, b.y, e.class as u8, e.bridge as u8, e.order);
    }
    for z in &p.parcels {
        s += &format!("P {} {} {} {} {} {}\n", z.center.x, z.center.y, z.kind as u8, z.size.x, z.size.y, z.angle);
    }
    // water mask over the towns' bounding box
    let (mut lo, mut hi) = (Vec2::splat(1e9), Vec2::splat(-1e9));
    for n in &p.graph.nodes {
        lo = lo.min(*n);
        hi = hi.max(*n);
    }
    let mut y = lo.y - 20.0;
    while y < hi.y + 20.0 {
        let mut x = lo.x - 20.0;
        while x < hi.x + 20.0 {
            if !is_land(seed, Vec2::new(x, y)) {
                s += &format!("W {x} {y}\n");
            }
            x += 2.0;
        }
        y += 2.0;
    }
    std::fs::write(path, s).unwrap();
}

/// R23: buildings sit beside roads, never on a junction/road node, even at full-grown footprint (1.1x, industrial 1.3x/1.1x).
#[test]
fn lots_sit_on_road_sides_not_on_nodes() {
    for seed in SEEDS {
        let p = plan(seed);
        for z in &p.parcels {
            let half_depth = z.size.y * 0.5 * 1.1 * 1.1; // largest footprint (industrial yard at full fill)
            for n in &p.graph.nodes {
                assert!(z.center.distance(*n) > 3.2, "seed {seed}: lot centre on road node {n:?}");
            }
            let (a, b) = p.graph.pos(z.access);
            let d = seg_dist(z.center, a, b);
            assert!(d >= half_depth + 1.0 && d <= 7.0, "seed {seed}: lot not set back beside its road (d={d})");
            // faces the road: long axis parallel to the frontage edge
            let dir = (b - a).normalize();
            let ax = Vec2::new(z.angle.cos(), z.angle.sin());
            assert!(dir.dot(ax).abs() > 0.999, "seed {seed}: lot not aligned to road");
        }
    }
}

