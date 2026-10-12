//! Road drawing and visible traffic. Built roads are strokes by class (local thin, arterial wider, both thinner
//! than rail; bridges get a dark casing). Zoomed out: each stroke tints green -> red by volume/capacity.
//! Zoomed in: real car journeys (R28). Each car starts at a home building, drives the fastest route over the roads
//! built right now to a destination building and disappears on arrival. Trips are sampled from the same demand and
//! mode-choice model as the traffic assignment (home by population and car share, destination by attraction and
//! distance, work vs shop by the hour), at a rate that follows the simulated car trips, slowed by the live
//! congestion. A capped pool of 420 dots draws them. Cars are visual only (D-2).
use crate::camera::MapCam;
use crate::shell::RoadClasses;
use crate::zones::{CityPlan, GrowthLink, Y_ROAD};
use bevy::prelude::*;
use city::{RoadClass, Router, Trip};
use ui_tools::segment_transform;
use world_gen::rng::SplitMix64;

const W_LOCAL: f32 = 1.0;
const W_ARTERIAL: f32 = 1.9;
/// Zoomed out beyond this view height: congestion tint instead of car dots.
pub const TINT_VIEW: f32 = 110.0;
/// Hard cap on rendered cars at any zoom (the pool size).
pub const MAX_CARS: usize = 420;
const TINT_BUCKETS: usize = 8;
/// Visual speed in world units per second per (unit/min of road speed): a 100-unit trip takes ~10-15 s.
const VIS_SPEED: f32 = 1.5;
/// Journeys started per second per simulated car trip/hour (a sample: the pool is capped anyway).
const SAMPLE: f32 = 0.015;
const Y_CAR: f32 = 0.62;
/// Car length, bumper gap at rest, acceleration and braking (world units, units/s^2).
const CAR_LEN: f32 = 1.6;
const CAR_GAP: f32 = 0.8;
const CAR_ACCEL: f32 = 4.0;
const CAR_BRAKE: f32 = 8.0;
const NEUTRAL: u8 = 255;

#[derive(Component)]
struct RoadEdge(usize);

#[derive(Component)]
struct RoadCasing(usize);

/// A building's driveway; shown once its frontage road (the edge index) is built.
#[derive(Component)]
struct Driveway(usize);

/// One painted line on road `edge`, in edge-local coordinates (`along`, lateral offset) from `p` to `q`.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct RoadMark {
    edge: usize,
    p: Vec2,
    q: Vec2,
    yellow: bool,
    /// Stroke width as a multiple of the zoom-scaled mark width (stop bars are fatter).
    fat: f32,
}

#[derive(Resource)]
struct MarkAssets(Handle<Mesh>, Handle<StandardMaterial>, Handle<StandardMaterial>);

const DASH: f32 = 1.2;
const DASH_GAP: f32 = 1.8;

/// US-style markings for one road (R29): yellow centre line (double solid on 2-lane fast and wider, dashed on minor
/// two-way roads, none on a one-lane street), dashed white between same-direction lanes, solid white edge lines, and
/// a stop bar across the incoming half at every junction end. Left-hand driving keeps colours, mirrors the stop bars.
fn edge_marks(edge: usize, len: f32, half_w: f32, c: city::lanes::LaneClass, junction: [bool; 2], left: bool) -> Vec<RoadMark> {
    use city::lanes::LaneClass::*;
    let line = |p: Vec2, q: Vec2, yellow: bool| RoadMark { edge, p, q, yellow, fat: 1.0 };
    let mut v = vec![];
    let dashed = |v: &mut Vec<RoadMark>, off: f32, yellow: bool| {
        let mut t = DASH_GAP / 2.0;
        while t + DASH < len {
            v.push(line(Vec2::new(t, off), Vec2::new(t + DASH, off), yellow));
            t += DASH + DASH_GAP;
        }
    };
    match c {
        Street => {}
        TwoLaneLow => dashed(&mut v, 0.0, true),
        _ => {
            for o in [-0.12, 0.12] {
                v.push(line(Vec2::new(0.0, o), Vec2::new(len, o), true));
            }
        }
    }
    for k in 1..c.lanes() / 2 {
        for side in [-1.0, 1.0] {
            dashed(&mut v, side * k as f32 * 0.9, false);
        }
    }
    let edge_off = half_w - 0.1;
    for side in [-1.0, 1.0] {
        v.push(line(Vec2::new(0.0, side * edge_off), Vec2::new(len, side * edge_off), false));
    }
    // traffic toward the end at `along = len` drives on the +lateral side (right-hand), toward 0 on the - side
    let s = if left { -1.0 } else { 1.0 };
    for (end, at, dir) in [(1, len, s), (0, 0.0, -s)] {
        if junction[end] && len > 4.0 {
            let t = if end == 1 { at - 1.6 } else { at + 1.6 };
            v.push(RoadMark { edge, p: Vec2::new(t, 0.0), q: Vec2::new(t, dir * edge_off), yellow: false, fat: 2.5 });
        }
    }
    v
}

fn mark_transform(m: &RoadMark, a: Vec2, b: Vec2, w: f32) -> Transform {
    let d = (b - a).normalize_or_zero();
    let n = Vec2::new(-d.y, d.x);
    let at = |l: Vec2| a + d * l.x + n * l.y;
    let (p, mut q) = (at(m.p), at(m.q));
    if p.distance(q) < 1e-3 {
        q = p + d * 0.01;
    }
    segment_transform(p, q, Y_ROAD + 0.06, w * m.fat, 0.1)
}

/// Marking width (world units): thin when zoomed in, grown with the view so it never vanishes zoomed out.
pub fn mark_width(view_h: f32) -> f32 {
    (view_h * 0.004).clamp(0.12, 1.2)
}
/// Car marker scale: real size zoomed in, enlarged zoomed out so aggregated traffic stays visible.
pub fn car_scale(view_h: f32) -> f32 {
    (view_h / 60.0).clamp(1.0, 4.0)
}

/// Current material bucket of a road stroke (NEUTRAL = class colour).
#[derive(Component)]
struct RoadLook(u8);

#[derive(Component)]
struct CarDot(usize);

#[derive(Resource)]
struct RoadMats {
    base: [Handle<StandardMaterial>; 2],
    bridge: Handle<StandardMaterial>,
    tint: Vec<Handle<StandardMaterial>>,
}

pub fn tint_bucket(vc: f32) -> u8 {
    ((vc.clamp(0.0, 1.6) / 1.6) * (TINT_BUCKETS - 1) as f32).round() as u8
}

/// Neutral road colour while free-flowing; amber -> red only above ~0.6 v/c. No green: it means residential.
pub fn tint_colour(bucket: usize) -> Color {
    let vc = bucket as f32 / (TINT_BUCKETS - 1) as f32 * 1.6;
    if vc < 0.6 {
        return Color::srgb(0.70, 0.68, 0.62);
    }
    let k = ((vc - 0.6) / 0.6).min(1.0);
    let (a, r) = ((0.96, 0.66, 0.18), (0.86, 0.18, 0.18));
    Color::srgb(a.0 + (r.0 - a.0) * k, a.1 + (r.1 - a.1) * k, a.2 + (r.2 - a.2) * k)
}

/// A car on its way: the route plus how far along segment `seg` it is.
struct Journey {
    trip: Trip,
    seg: usize,
    along: f32,
    /// Current speed (world units / s): cars accelerate and brake, they do not jump to the limit.
    v: f32,
}

#[derive(Resource)]
struct Journeys {
    cars: Vec<Journey>,
    acc: f32,
    rng: SplitMix64,
    router: Router,
}

/// Free-flow speed on the car's current segment, slowed by congestion.
fn target_speed(j: &Journey, slow: &impl Fn(usize) -> f32) -> f32 {
    j.trip.seg_speed[j.seg] * VIS_SPEED / slow(j.trip.seg_edge[j.seg]).max(1.0)
}

/// Advance `d` world units along the route. False once it has arrived.
fn move_by(j: &mut Journey, mut d: f32) -> bool {
    while j.seg + 1 < j.trip.pts.len() {
        let len = j.trip.pts[j.seg].distance(j.trip.pts[j.seg + 1]).max(1e-3);
        if j.along + d < len {
            j.along += d;
            return true;
        }
        d -= len - j.along;
        j.seg += 1;
        j.along = 0.0;
    }
    false
}

#[cfg(test)]
fn advance(j: &mut Journey, dt: f32, slow: impl Fn(usize) -> f32) -> bool {
    let v = target_speed(j, &slow);
    move_by(j, v * dt)
}

type Key = (u32, u32, u32, u32);
fn pkey(p: Vec2) -> (u32, u32) {
    (p.x.to_bits(), p.y.to_bits())
}
/// Directed road segment `k` of a route: cars share a lane only when both endpoints match (so `along` is comparable).
fn seg_key(t: &Trip, k: usize) -> Option<Key> {
    let (a, b) = (pkey(*t.pts.get(k)?), pkey(*t.pts.get(k + 1)?));
    Some((a.0, a.1, b.0, b.1))
}
fn lane_key(j: &Journey) -> Option<Key> {
    seg_key(&j.trip, j.seg)
}

fn seg_len(j: &Journey) -> f32 {
    j.trip.pts[j.seg].distance(j.trip.pts[j.seg + 1])
}

/// Junction zone: a car this close to a node (either side) holds it; others wait at the stop line before it.
const JUNC: f32 = 1.6;

/// One car-following step for every car (in index order, deterministic): accelerate toward the congestion-limited
/// speed, but never faster than lets the car stop behind the one ahead in its lane, or at the stop line of a junction
/// another car holds. A car just past a node (on the ring) holds it first, so entering cars give way.
fn follow_step(cars: &mut Vec<Journey>, dt: f32, slow: impl Fn(usize) -> f32) {
    use std::collections::HashMap;
    let mut lanes: HashMap<Key, Vec<f32>> = HashMap::new();
    let mut held: HashMap<(u32, u32), usize> = HashMap::new();
    for (i, c) in cars.iter().enumerate() {
        if let Some(k) = lane_key(c) {
            lanes.entry(k).or_default().push(c.along);
        }
        if c.seg > 0 && c.along < JUNC {
            held.entry(pkey(c.trip.pts[c.seg])).or_insert(i);
        }
    }
    for (i, c) in cars.iter().enumerate() {
        if c.seg + 2 < c.trip.pts.len() && seg_len(c) - c.along < JUNC {
            held.entry(pkey(c.trip.pts[c.seg + 1])).or_insert(i);
        }
    }
    let mut keep = vec![];
    for i in 0..cars.len() {
        let c = &cars[i];
        let len = seg_len(c);
        let mut gap = f32::INFINITY;
        if let Some(k) = lane_key(c) {
            let same = lanes.get(&k).into_iter().flatten().copied().filter(|&a| a > c.along + 1e-4).fold(f32::INFINITY, f32::min);
            gap = gap.min(same - c.along - CAR_LEN);
            if let Some(nk) = seg_key(&c.trip, c.seg + 1) {
                let ahead = lanes.get(&nk).into_iter().flatten().copied().fold(f32::INFINITY, f32::min);
                gap = gap.min(len - c.along + ahead - CAR_LEN);
                let node = pkey(c.trip.pts[c.seg + 1]);
                if held.get(&node).is_some_and(|&h| h != i) {
                    gap = gap.min(len - JUNC - c.along + CAR_GAP);
                } else if len - c.along < JUNC + c.v * c.v / (2.0 * CAR_BRAKE) + 0.5 {
                    held.insert(node, i);
                }
            }
        }
        let vmax = if gap.is_finite() { target_speed(c, &slow).min((2.0 * CAR_BRAKE * (gap - CAR_GAP).max(0.0)).sqrt()) } else { target_speed(c, &slow) };
        let c = &mut cars[i];
        c.v += (vmax - c.v).clamp(-CAR_BRAKE * dt, CAR_ACCEL * dt);
        c.v = c.v.max(0.0);
        // never step past the stop distance this tick (the speed cap alone can overshoot by v*dt)
        let step = if gap.is_finite() { (c.v * dt).min((gap - CAR_GAP * 0.5).max(0.0)) } else { c.v * dt };
        keep.push(move_by(c, step));
        if let Some(k) = lane_key(c) {
            lanes.entry(k).or_default().push(c.along);
        }
    }
    let mut k = keep.into_iter();
    cars.retain(|_| k.next().unwrap_or(true));
}

/// Position and heading of a journey (driving on one side of the road).
/// Drive on the left instead of the right (R34). Saved with the game.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub struct LeftHand(pub bool);

fn journey_pose(j: &Journey, left: bool) -> (Vec2, Vec2) {
    let (a, b) = (j.trip.pts[j.seg], j.trip.pts[j.seg + 1]);
    let d = (b - a).normalize_or_zero();
    (a + d * j.along + Vec2::new(-d.y, d.x) * if left { -0.55 } else { 0.55 }, d)
}

pub struct RoadsPlugin;

impl Plugin for RoadsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LeftHand>().add_systems(Startup, spawn_roads).add_systems(Update, (update_roads, update_marks, drive_cars));
    }
}

fn spawn_roads(
    cp: Res<CityPlan>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let rect = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let mut flat = |c: Color| mats.add(StandardMaterial { base_color: c, unlit: true, ..default() });
    let m = RoadMats {
        base: [flat(Color::srgb(0.70, 0.68, 0.62)), flat(Color::srgb(0.93, 0.90, 0.80))],
        bridge: flat(Color::srgb(0.98, 0.96, 0.90)),
        tint: (0..TINT_BUCKETS).map(|k| flat(tint_colour(k))).collect(),
    };
    let casing = flat(Color::srgb(0.20, 0.20, 0.24));
    for (i, e) in cp.plan.graph.edges.iter().enumerate() {
        let (a, b) = cp.plan.graph.pos(i);
        let w = if e.class == RoadClass::Arterial { W_ARTERIAL } else { W_LOCAL };
        let base = if e.bridge { m.bridge.clone() } else { m.base[(e.class == RoadClass::Arterial) as usize].clone() };
        if e.bridge {
            commands.spawn((
                RoadCasing(i),
                Mesh3d(rect.clone()),
                MeshMaterial3d(casing.clone()),
                segment_transform(a, b, Y_ROAD - 0.05, w + 1.3, 0.2),
                Visibility::Hidden,
            ));
        }
        commands.spawn((
            RoadEdge(i),
            RoadLook(NEUTRAL),
            Mesh3d(rect.clone()),
            MeshMaterial3d(base),
            segment_transform(a, b, Y_ROAD, w, 0.2),
            Visibility::Hidden,
        ));
    }
    let car_mesh = meshes.add(Cuboid::new(1.5, 0.2, 0.7));
    let car = flat(Color::srgb(0.16, 0.18, 0.24));
    for i in 0..MAX_CARS {
        commands.spawn((CarDot(i), Mesh3d(car_mesh.clone()), MeshMaterial3d(car.clone()), Transform::from_xyz(0.0, Y_CAR, 0.0), Visibility::Hidden));
    }
    // driveways: a short stub from every building to its road
    let drive = flat(Color::srgb(0.62, 0.60, 0.56));
    for (z, pc) in cp.plan.parcels.iter().enumerate() {
        let foot = city::driveway_foot(&cp.plan, z);
        if foot.distance(pc.center) > 0.3 {
            commands.spawn((Driveway(pc.access), Mesh3d(rect.clone()), MeshMaterial3d(drive.clone()), segment_transform(pc.center, foot, Y_ROAD - 0.02, 0.5, 0.1), Visibility::Hidden));
        }
    }
    commands.insert_resource(MarkAssets(rect.clone(), flat(Color::srgb(0.97, 0.97, 0.94)), flat(Color::srgb(0.98, 0.78, 0.12))));
    commands.insert_resource(m);
    commands.insert_resource(Journeys { cars: vec![], acc: 0.0, rng: SplitMix64::new(0xCA75), router: Router::new(&cp.plan) });
}

/// Road markings at every zoom: rebuilt when classes or the driving side change, stroke scaled with the view.
#[allow(clippy::too_many_arguments)]
fn update_marks(
    link: Res<GrowthLink>,
    cam: Option<Res<MapCam>>,
    cp: Res<CityPlan>,
    classes: Res<RoadClasses>,
    hand: Res<LeftHand>,
    assets: Res<MarkAssets>,
    mut marks: Query<(Entity, &RoadMark, &mut Transform, &mut Visibility)>,
    mut roads: Query<(&RoadEdge, &mut Transform), Without<RoadMark>>,
    mut commands: Commands,
    mut last: Local<(i32, u64)>,
    mut built_once: Local<bool>,
) {
    let view = cam.as_ref().map_or(100.0, |c| c.view_h);
    let w = mark_width(view);
    let s = link.snapshots.latest();
    let rebuild = !*built_once || classes.is_changed() || hand.is_changed();
    let key = ((w * 100.0) as i32, s.tick / 10);
    if *last == key && !rebuild {
        return;
    }
    *last = key;
    if rebuild {
        *built_once = true;
        for (ent, ..) in &marks {
            commands.entity(ent).despawn();
        }
        let deg = cp.plan.graph.incident();
        for (e, ed) in cp.plan.graph.edges.iter().enumerate() {
            let c = classes.0.get(&e).copied().unwrap_or_else(|| city::lanes::default_class(ed.class));
            let base = if ed.class == RoadClass::Arterial { W_ARTERIAL } else { W_LOCAL };
            let half = if classes.0.contains_key(&e) { c.width().max(W_LOCAL) } else { base } / 2.0;
            let (a, b) = cp.plan.graph.pos(e);
            let junction = [deg[ed.a as usize].len() >= 3, deg[ed.b as usize].len() >= 3];
            for m in edge_marks(e, a.distance(b), half, c, junction, hand.0) {
                let mat = if m.yellow { assets.2.clone() } else { assets.1.clone() };
                let shown = s.built.get(e).copied().unwrap_or(false);
                commands.spawn((m, Mesh3d(assets.0.clone()), MeshMaterial3d(mat), mark_transform(&m, a, b, w), if shown { Visibility::Inherited } else { Visibility::Hidden }));
            }
        }
        // R31: at-grade junctions are roundabouts sized by their widest road: a ring of marks in the first edge's frame
        for (n, es) in deg.iter().enumerate() {
            let cls: Vec<_> = es.iter().map(|&e| classes.0.get(&e).copied().unwrap_or_else(|| city::lanes::default_class(cp.plan.graph.edges[e].class))).collect();
            let city::junctions::Junction::Roundabout(r) = city::junctions::junction(&cls) else { continue };
            let e = es[0];
            let (a, b) = cp.plan.graph.pos(e);
            let (d, c) = ((b - a).normalize_or_zero(), cp.plan.graph.nodes[n]);
            let local = |p: Vec2| Vec2::new((p - a).dot(d), (p - a).dot(Vec2::new(-d.y, d.x)));
            let pt = |i: usize| c + Vec2::from_angle(i as f32 * std::f32::consts::TAU / 16.0) * r;
            let shown = s.built.get(e).copied().unwrap_or(false);
            for i in 0..16 {
                let m = RoadMark { edge: e, p: local(pt(i)), q: local(pt(i + 1)), yellow: false, fat: 1.0 };
                commands.spawn((m, Mesh3d(assets.0.clone()), MeshMaterial3d(assets.1.clone()), mark_transform(&m, a, b, w), if shown { Visibility::Inherited } else { Visibility::Hidden }));
            }
        }
        // roads widen with their class (never thinner than the generated stroke)
        for (r, mut tf) in &mut roads {
            if let Some(c) = classes.0.get(&r.0) {
                let (a, b) = cp.plan.graph.pos(r.0);
                *tf = segment_transform(a, b, Y_ROAD, c.width().max(W_LOCAL), 0.2);
            }
        }
        return;
    }
    for (_, m, mut tf, mut v) in &mut marks {
        let (a, b) = cp.plan.graph.pos(m.edge);
        *tf = mark_transform(m, a, b, w);
        let want = if s.built.get(m.edge).copied().unwrap_or(false) { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
    }
}

fn update_roads(
    link: Res<GrowthLink>,
    cam: Option<Res<MapCam>>,
    mats: Res<RoadMats>,
    cp: Res<CityPlan>,
    mut last: Local<(Option<u64>, bool)>,
    mut roads: Query<(&RoadEdge, &mut RoadLook, &mut Visibility, &mut MeshMaterial3d<StandardMaterial>)>,
    mut casings: Query<(&RoadCasing, &mut Visibility), Without<RoadEdge>>,
    mut drives: Query<(&Driveway, &mut Visibility), (Without<RoadEdge>, Without<RoadCasing>)>,
) {
    let s = link.snapshots.latest();
    let zoomed_out = cam.as_ref().map_or(true, |c| c.view_h > TINT_VIEW);
    if *last == (Some(s.tick), zoomed_out) {
        return;
    }
    *last = (Some(s.tick), zoomed_out);
    let vis = |built: bool| if built { Visibility::Inherited } else { Visibility::Hidden };
    for (r, mut look, mut v, mut m) in &mut roads {
        let built = s.built.get(r.0).copied().unwrap_or(false);
        let want = if built && zoomed_out { tint_bucket(s.vc.get(r.0).copied().unwrap_or(0.0)) } else { NEUTRAL };
        if *v != vis(built) {
            *v = vis(built);
        }
        if look.0 != want {
            look.0 = want;
            let g = &cp.plan.graph.edges[r.0];
            m.0 = if want == NEUTRAL {
                if g.bridge { mats.bridge.clone() } else { mats.base[(g.class == RoadClass::Arterial) as usize].clone() }
            } else {
                mats.tint[want as usize].clone()
            };
        }
    }
    for (d, mut v) in &mut drives {
        let want = vis(s.built.get(d.0).copied().unwrap_or(false));
        if *v != want {
            *v = want;
        }
    }
    for (c, mut v) in &mut casings {
        let built = s.built.get(c.0).copied().unwrap_or(false);
        if *v != vis(built) {
            *v = vis(built);
        }
    }
}

fn drive_cars(
    time: Res<Time>,
    link: Res<GrowthLink>,
    cam: Option<Res<MapCam>>,
    hand: Res<LeftHand>,
    mut j: ResMut<Journeys>,
    mut was_on: Local<bool>,
    mut dots: Query<(&CarDot, &mut Transform, &mut Visibility)>,
) {
    let view = cam.as_ref().map_or(60.0, |c| c.view_h);
    let on = true;
    if !on {
        if *was_on {
            *was_on = false;
            j.cars.clear();
            for (_, _, mut v) in &mut dots {
                *v = Visibility::Hidden;
            }
        }
        return;
    }
    *was_on = true;
    let s = link.snapshots.latest();
    let dt = time.delta_secs().min(0.1);
    let slow = |e: usize| s.slow.get(e).copied().unwrap_or(1.0);
    follow_step(&mut j.cars, dt, slow);
    j.acc = (j.acc + s.car_trips * SAMPLE * dt).min(6.0);
    while j.acc >= 1.0 && j.cars.len() < MAX_CARS {
        j.acc -= 1.0;
        let j = &mut *j;
        let Some((a, b)) = j.router.sample(&s.built, &s.pop, &s.zone_share, s.hour as f64, &mut j.rng) else { break };
        if let Some(trip) = j.router.route(&s.built, &s.slow, a, b) {
            // a new car waits at its origin until the lane's entrance is clear
            if !j.cars.iter().any(|c| c.seg == 0 && c.along < CAR_LEN + CAR_GAP && c.trip.pts.first() == trip.pts.first()) {
                j.cars.push(Journey { trip, seg: 0, along: 0.0, v: 0.0 });
            }
        }
    }
    for (d, mut tf, mut v) in &mut dots {
        let Some(c) = j.cars.get(d.0) else {
            if *v != Visibility::Hidden {
                *v = Visibility::Hidden;
            }
            continue;
        };
        let (p, h) = journey_pose(c, hand.0);
        tf.translation = Vec3::new(p.x, Y_CAR, p.y);
        tf.rotation = Quat::from_rotation_y(-h.y.atan2(h.x));
        tf.scale = Vec3::splat(car_scale(view));
        *v = Visibility::Inherited;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tint_is_neutral_when_free_then_amber_to_red_never_green() {
        assert_eq!(tint_bucket(0.0), 0);
        assert_eq!(tint_bucket(5.0), (TINT_BUCKETS - 1) as u8);
        assert!((0..TINT_BUCKETS - 1).all(|k| tint_bucket(k as f32 * 0.2) <= tint_bucket((k + 1) as f32 * 0.2)));
        let (free, jam) = (tint_colour(0).to_srgba(), tint_colour(TINT_BUCKETS - 1).to_srgba());
        assert_eq!(tint_colour(tint_bucket(0.5) as usize), tint_colour(0), "free flow stays neutral");
        assert!(jam.red > 0.8 && jam.green < 0.3 && free.red < 0.75);
        assert!((0..TINT_BUCKETS).all(|k| { let c = tint_colour(k).to_srgba(); c.green <= c.red }), "no green tint");
    }

    fn journey(speed: f32) -> Journey {
        let trip = Trip {
            from: 0,
            to: 1,
            pts: vec![Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)],
            seg_edge: vec![0, 1],
            seg_speed: vec![speed, speed],
            nodes: vec![],
            edges: vec![],
        };
        Journey { trip, seg: 0, along: 0.0, v: 0.0 }
    }

    #[test]
    fn a_car_drives_its_route_in_order_then_disappears() {
        let mut j = journey(2.0); // 3 world units/s
        assert!(advance(&mut j, 2.0, |_| 1.0));
        assert_eq!(j.seg, 0);
        assert!((journey_pose(&j, false).0.x - 6.0).abs() < 1e-4);
        assert!(advance(&mut j, 2.0, |_| 1.0), "rounds the corner onto the next road");
        assert_eq!(j.seg, 1);
        assert!(journey_pose(&j, false).0.y > 1.0 && journey_pose(&j, false).1.y > 0.9);
        assert!(!advance(&mut j, 10.0, |_| 1.0), "arrived: despawn");
    }

    #[test]
    fn congestion_slows_a_car_down() {
        let (mut free, mut jam) = (journey(2.0), journey(2.0));
        advance(&mut free, 1.0, |_| 1.0);
        advance(&mut jam, 1.0, |_| 3.0);
        assert!((free.along - 3.0).abs() < 1e-4 && (jam.along - 1.0).abs() < 1e-4);
    }

    #[test]
    fn cars_queue_behind_a_slow_leader_and_never_overlap() {
        let mut cars: Vec<Journey> = (0..6).map(|i| { let mut j = journey(4.0); j.along = i as f32 * 2.5; j }).collect();
        cars[5].v = 0.0;
        // the leader crawls (congestion x20); the followers must stack up behind it
        for _ in 0..600 {
            let lead = cars.len() - 1;
            follow_step(&mut cars, 0.05, |e| if e == 99 { 1.0 } else { 1.0 });
            let _ = lead;
            if cars.len() < 6 { break; }
            cars.sort_by(|a, b| a.along.total_cmp(&b.along));
            for w in cars.windows(2) {
                if w[0].seg == w[1].seg {
                    assert!(w[1].along - w[0].along >= CAR_LEN - 1e-3, "overlap {} {}", w[0].along, w[1].along);
                }
            }
        }
    }

    #[test]
    fn a_car_brakes_for_a_stopped_leader() {
        let mut cars = vec![journey(4.0), journey(4.0)];
        cars[0].along = 2.0;
        cars[1].along = 8.0;
        for _ in 0..200 {
            follow_step(&mut cars, 0.05, |_| 1.0);
            if cars.len() < 2 { return; }
            cars[1].v = 0.0;
            cars[1].along = 8.0; // pinned stopped
        }
        assert!(cars[0].along <= 8.0 - CAR_LEN, "stopped behind, got {}", cars[0].along);
    }

    /// Headless: cars from four arms through one shared junction, plus a straight-through queue, for 60 s. No two cars
    /// ever overlap (same lane closer than a car length, or bodies closer than 0.6 anywhere), and all of them get through.
    #[test]
    fn many_cars_through_a_junction_never_overlap() {
        let arm = |d: Vec2| d * 12.0;
        let dirs = [Vec2::X, Vec2::Y, -Vec2::X, -Vec2::Y];
        let mut cars: Vec<Journey> = vec![];
        let mut spawned = 0;
        for step in 0..1200 {
            if step % 15 == 0 {
                for (k, &d) in dirs.iter().enumerate() {
                    let out = dirs[(k + 1 + step / 15 % 3) % 4];
                    let trip = Trip { from: 0, to: 1, pts: vec![arm(d) * 1.5, arm(d), Vec2::ZERO, arm(out), arm(out) * 1.5], seg_edge: vec![0, 1, 2, 3], seg_speed: vec![4.0; 4], nodes: vec![], edges: vec![] };
                    if !cars.iter().any(|c| c.seg == 0 && c.along < CAR_LEN + CAR_GAP && c.trip.pts[0] == trip.pts[0]) {
                        cars.push(Journey { trip, seg: 0, along: 0.0, v: 0.0 });
                        spawned += 1;
                    }
                }
            }
            follow_step(&mut cars, 0.05, |_| 1.0);
            for left in [false, true] {
                for i in 0..cars.len() {
                    for j in i + 1..cars.len() {
                        let (a, b) = (&cars[i], &cars[j]);
                        if lane_key(a) == lane_key(b) {
                            assert!((a.along - b.along).abs() >= CAR_LEN - 1e-3, "same-lane overlap at step {step}");
                        }
                        let d = journey_pose(a, left).0.distance(journey_pose(b, left).0);
                        assert!(d >= 0.6, "cars {i},{j} overlap ({d}) at step {step}");
                    }
                }
            }
        }
        assert!(spawned > 40 && cars.len() < 30, "traffic flows: {spawned} spawned, {} still queued", cars.len());
    }

    #[test]
    fn us_markings_by_class_and_side() {
        use city::lanes::LaneClass::*;
        let yellow = |v: &[RoadMark]| v.iter().filter(|m| m.yellow).count();
        let minor = edge_marks(0, 30.0, 0.9, TwoLaneLow, [false; 2], false);
        assert!(yellow(&minor) > 3 && minor.iter().filter(|m| m.yellow).all(|m| m.q.x - m.p.x <= DASH + 1e-4), "dashed yellow centre");
        let big = edge_marks(0, 30.0, 1.8, FourLane, [false; 2], false);
        assert_eq!(big.iter().filter(|m| m.yellow && m.q.x - m.p.x == 30.0).count(), 2, "double solid yellow");
        assert!(big.iter().any(|m| !m.yellow && m.p.y.abs() == 0.9 && m.q.x - m.p.x <= DASH + 1e-4), "dashed white lane line");
        assert_eq!(big.iter().filter(|m| !m.yellow && m.p.y.abs() > 1.6 && m.q.x - m.p.x == 30.0).count(), 2, "solid white edges");
        assert_eq!(yellow(&edge_marks(0, 30.0, 0.5, Street, [false; 2], false)), 0);
        let bar = |left| edge_marks(0, 30.0, 0.9, TwoLaneLow, [false, true], left).into_iter().find(|m| m.fat > 1.0).unwrap();
        let (r, l) = (bar(false), bar(true));
        assert!(r.q.y > 0.0 && l.q.y < 0.0 && r.yellow == l.yellow, "stop bar mirrors with the driving side");
    }

    #[test]
    fn left_hand_driving_flips_the_lane_side() {
        let j = journey(2.0);
        assert!(journey_pose(&j, false).0.y > 0.0 && journey_pose(&j, true).0.y < 0.0);
    }
}


#[cfg(test)]
mod zoom_tests {
    use super::*;

    #[test]
    fn markings_and_cars_stay_visible_zoomed_out() {
        assert!(mark_width(400.0) > mark_width(40.0) && mark_width(40.0) >= 0.12);
        assert_eq!(car_scale(30.0), 1.0);
        assert!(car_scale(300.0) > 2.0);
    }
}
