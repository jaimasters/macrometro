use super::*;
use crate::roadgen::{Parcel, REdge, RoadGraph, TownPlan, ZoneKind};
use transit::CrossingKind;

fn parcel(x: f32, y: f32) -> Parcel {
    Parcel { center: Vec2::new(x, y), town: 0, kind: ZoneKind::Residential, access: 0, angle: 0.0, size: Vec2::new(5.0, 3.0), capacity: 100.0, start_pop: 0.0, core: false }
}

fn plan(parcels: Vec<Parcel>, roads: &[(Vec2, Vec2, RoadClass)]) -> TownPlan {
    let mut g = RoadGraph::default();
    for (a, b, class) in roads {
        g.nodes.extend([*a, *b]);
        let n = g.nodes.len() as u32;
        g.edges.push(REdge { a: n - 2, b: n - 1, class: *class, bridge: false, len: a.distance(*b), town: 0, order: 0 });
    }
    TownPlan { graph: g, parcels, towns: vec![] }
}

fn ctx<'a>(p: &'a TownPlan, built: &'a [bool], pop: &'a [f32], land: &'a dyn Fn(Vec2) -> bool) -> Ctx<'a> {
    Ctx { plan: p, built, pop, gone: &[], land }
}

const LAND: &dyn Fn(Vec2) -> bool = &|_| true;

/// A wall of lots across x = 50 spanning y in [-half, half].
fn wall(half: f32) -> Vec<Parcel> {
    (0..=(half * 2.0 / 4.0) as i32).map(|i| parcel(50.0, -half + i as f32 * 4.0)).collect()
}

#[test]
fn avoids_a_building_when_there_is_room_to_go_round() {
    let p = plan(wall(8.0), &[]);
    let pop = vec![50.0; p.parcels.len()];
    let c = ctx(&p, &[], &pop, LAND);
    let (a, b) = (Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0));
    let straight = place_via(&c, a, b, (a + b) * 0.5, DEFAULT_RADIUS).unwrap();
    assert!(!straight.demolition.zones.is_empty(), "the straight line goes through the wall");
    let best = place_track(&c, a, b, None, DEFAULT_RADIUS).unwrap();
    assert!(best.demolition.zones.is_empty(), "went round: {:?}", best.demolition);
    assert!(best.cost.total < straight.cost.total);
    assert!(best.offset.abs() >= 8.0, "detour {}", best.offset);
}

#[test]
fn the_mouse_picks_the_side() {
    let p = plan(wall(8.0), &[]);
    let pop = vec![50.0; p.parcels.len()];
    let c = ctx(&p, &[], &pop, LAND);
    let (a, b) = (Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0));
    let mean_y = |pl: &Placement| pl.pts.iter().map(|q| q.y).sum::<f32>() / pl.pts.len() as f32;
    let up = place_track(&c, a, b, Some(Vec2::new(50.0, 30.0)), DEFAULT_RADIUS).unwrap();
    let down = place_track(&c, a, b, Some(Vec2::new(50.0, -30.0)), DEFAULT_RADIUS).unwrap();
    assert!(mean_y(&up) > 3.0 && mean_y(&down) < -3.0, "{} {}", mean_y(&up), mean_y(&down));
    assert!(up.demolition.zones.is_empty() && down.demolition.zones.is_empty());
    // a mouse right on the line steers nothing: either side may win, but still no demolition
    assert!(place_track(&c, a, b, Some(Vec2::new(50.0, 0.5)), DEFAULT_RADIUS).unwrap().demolition.zones.is_empty());
}

#[test]
fn unavoidable_buildings_are_priced_by_density() {
    let p = plan(wall(120.0), &[]);
    let (a, b) = (Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0));
    let cost_with = |pop: f32| {
        let pops = vec![pop; p.parcels.len()];
        let c = ctx(&p, &[], &pops, LAND);
        place_track(&c, a, b, None, DEFAULT_RADIUS).unwrap()
    };
    let (empty, dense) = (cost_with(0.0), cost_with(400.0));
    assert!(!empty.demolition.zones.is_empty(), "the wall is too long to go round");
    // compensation = sum over demolished zones of the per-zone formula
    let expect = |pl: &Placement, pop: f64| pl.demolition.zones.len() as f64 * (ED_EMPTY + ED_PER_POP * pop);
    assert!((empty.demolition.cost - expect(&empty, 0.0)).abs() < 1e-9);
    assert!((dense.demolition.cost - expect(&dense, 400.0)).abs() < 1e-9);
    assert!(dense.demolition.cost > empty.demolition.cost && dense.demolition.standing > empty.demolition.standing);
    assert!((dense.cost.total - (dense.cost.track + dense.cost.crossings + dense.cost.eminent)).abs() < 1e-9);
    assert!(dense.demolition.standing <= 0.3 + 1e-12);
    // already-demolished zones cost nothing the second time
    let pops = vec![400.0; p.parcels.len()];
    let gone = vec![true; p.parcels.len()];
    let again = place_track(&Ctx { plan: &p, built: &[], pop: &pops, gone: &gone, land: LAND }, a, b, None, DEFAULT_RADIUS).unwrap();
    assert_eq!(again.demolition, Demolition::default());
}

#[test]
fn roads_become_level_crossings_water_a_priced_bridge() {
    let road = (Vec2::new(40.0, -30.0), Vec2::new(40.0, 30.0), RoadClass::Local);
    let p = plan(vec![], &[road]);
    let c = ctx(&p, &[true], &[], LAND);
    let (a, b) = (Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0));
    let t = place_track(&c, a, b, None, DEFAULT_RADIUS).unwrap();
    assert_eq!(t.crossings.len(), 1);
    assert_eq!((t.crossings[0].kind, t.crossings[0].road), (CrossingKind::Level, 0), "cheapest valid option by default");
    assert!((t.cost.crossings - LEVEL_COST).abs() < 1e-9 && t.vcap <= LEVEL_VCAP);
    assert!((t.crossings[0].at - 40.0).abs() < 1.0);
    // upgrading a level crossing costs the difference; there is nothing to upgrade from a bridge
    assert_eq!(crossing_upgrade_cost(CrossingKind::Level, CrossingKind::Bridge), Some(ROAD_BRIDGE_COST - LEVEL_COST));
    assert_eq!(crossing_upgrade_cost(CrossingKind::Bridge, CrossingKind::Underpass), None);
    // an unbuilt road is not crossed
    assert!(place_track(&ctx(&p, &[false], &[], LAND), a, b, None, DEFAULT_RADIUS).unwrap().crossings.is_empty());
    // a river 10 wide: one bridge priced by span
    let river: &dyn Fn(Vec2) -> bool = &|q| !(60.0..70.0).contains(&q.x);
    let w = place_track(&ctx(&plan(vec![], &[]), &[], &[], river), a, b, None, DEFAULT_RADIUS).unwrap();
    assert_eq!(w.crossings.len(), 1);
    assert_eq!(w.crossings[0].kind, CrossingKind::Bridge);
    assert!((w.cost.crossings - (WATER_BRIDGE_BASE + WATER_BRIDGE_PER_LEN * 10.0)).abs() < 1.5 * WATER_BRIDGE_PER_LEN);
    let lake: &dyn Fn(Vec2) -> bool = &|q| !(20.0..80.0).contains(&q.x);
    assert!(place_track(&ctx(&plan(vec![], &[]), &[], &[], lake), a, b, None, DEFAULT_RADIUS).is_none(), "too wide to bridge");
}

#[test]
fn a_bigger_radius_is_faster_but_sweeps_more() {
    assert!(curve_speed(80.0) > curve_speed(40.0) && curve_speed(40.0) > curve_speed(10.0));
    // an L-bend with lots tucked inside the corner
    let p = plan(vec![parcel(88.0, 12.0), parcel(80.0, 20.0)], &[]);
    let pop = [300.0, 300.0];
    let c = ctx(&p, &[], &pop, LAND);
    let (a, b, corner) = (Vec2::new(0.0, 0.0), Vec2::new(100.0, 100.0), Vec2::new(100.0, 0.0));
    let tight = place_via(&c, a, b, corner, 10.0).unwrap();
    let wide = place_via(&c, a, b, corner, 80.0).unwrap();
    assert!(wide.vcap > tight.vcap && wide.radius > tight.radius);
    assert!(wide.demolition.zones.len() > tight.demolition.zones.len(), "{:?} vs {:?}", wide.demolition, tight.demolition);
    assert!(wide.len < tight.len, "the arc cuts the corner");
    // the upgrade path re-lays the same bend
    let up = with_radius(&c, &tight, 80.0).unwrap();
    assert_eq!(up.pts, wide.pts);
    // no hard corners: smooth polyline, turning a few degrees per vertex
    for w in tight.pts.windows(3) {
        let (u, v) = ((w[1] - w[0]).normalize_or_zero(), (w[2] - w[1]).normalize_or_zero());
        assert!(u.dot(v) > 0.9 || u == Vec2::ZERO || v == Vec2::ZERO, "sharp corner at {:?}", w[1]);
    }
}
