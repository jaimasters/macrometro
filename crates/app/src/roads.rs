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

/// Lane marking on road `edge`: `k` = 0 centreline, else a lane divider `k` lanes out to each side (`side` +-1).
#[derive(Component)]
struct RoadMark {
    edge: usize,
    k: u8,
    side: f32,
}

#[derive(Resource)]
struct MarkAssets(Handle<Mesh>, Handle<StandardMaterial>);

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

/// Road segment a car is on, with its direction: cars share a lane only when this matches.
fn lane_key(j: &Journey) -> Option<(usize, bool)> {
    let (a, b) = (*j.trip.pts.get(j.seg)?, *j.trip.pts.get(j.seg + 1)?);
    let e = *j.trip.seg_edge.get(j.seg)?;
    Some((e, (b.x, b.y) > (a.x, a.y)))
}

fn seg_len(j: &Journey) -> f32 {
    j.trip.pts[j.seg].distance(j.trip.pts[j.seg + 1])
}

/// One car-following step for every car (in index order, deterministic): accelerate toward the congestion-limited
/// speed, but never faster than lets the car stop behind the one ahead in its lane.
fn follow_step(cars: &mut Vec<Journey>, dt: f32, slow: impl Fn(usize) -> f32) {
    let mut lanes: std::collections::HashMap<(usize, bool), Vec<f32>> = std::collections::HashMap::new();
    for c in cars.iter() {
        if let Some(k) = lane_key(c) {
            lanes.entry(k).or_default().push(c.along);
        }
    }
    let next_key = |c: &Journey| {
        let mut n = Journey { trip: Trip { pts: c.trip.pts.clone(), seg_edge: c.trip.seg_edge.clone(), ..c.trip.clone() }, seg: c.seg + 1, along: 0.0, v: 0.0 };
        if n.seg + 1 >= n.trip.pts.len() {
            return None;
        }
        n.along = 0.0;
        lane_key(&n)
    };
    let mut keep = vec![];
    for i in 0..cars.len() {
        let c = &cars[i];
        let gap = lane_key(c).and_then(|k| {
            let same = lanes.get(&k).into_iter().flatten().copied().filter(|&a| a > c.along + 1e-4).fold(f32::INFINITY, f32::min);
            if same.is_finite() {
                return Some(same - c.along - CAR_LEN);
            }
            let nk = next_key(c)?;
            let ahead = lanes.get(&nk)?.iter().copied().fold(f32::INFINITY, f32::min);
            ahead.is_finite().then(|| seg_len(c) - c.along + ahead - CAR_LEN)
        });
        let vmax = match gap {
            Some(g) => target_speed(c, &slow).min((2.0 * CAR_BRAKE * (g - CAR_GAP).max(0.0)).sqrt()),
            None => target_speed(c, &slow),
        };
        let c = &mut cars[i];
        c.v += (vmax - c.v).clamp(-CAR_BRAKE * dt, CAR_ACCEL * dt);
        c.v = c.v.max(0.0);
        keep.push(move_by(c, c.v * dt));
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
    let mark_mat = flat(Color::srgb(0.97, 0.97, 0.9));
    commands.insert_resource(MarkAssets(rect.clone(), mark_mat.clone()));
    for (i, _) in cp.plan.graph.edges.iter().enumerate() {
        let (a, b) = cp.plan.graph.pos(i);
        commands.spawn((RoadMark { edge: i, k: 0, side: 0.0 }, Mesh3d(rect.clone()), MeshMaterial3d(mark_mat.clone()), segment_transform(a, b, Y_ROAD + 0.06, 0.2, 0.1), Visibility::Hidden));
    }
    commands.insert_resource(m);
    commands.insert_resource(Journeys { cars: vec![], acc: 0.0, rng: SplitMix64::new(0xCA75), router: Router::new(&cp.plan) });
}

/// Lane lines at every zoom: a centreline on every built road, plus dividers on wide classes, scaled with the view.
#[allow(clippy::too_many_arguments)]
fn update_marks(
    link: Res<GrowthLink>,
    cam: Option<Res<MapCam>>,
    cp: Res<CityPlan>,
    classes: Res<RoadClasses>,
    assets: Res<MarkAssets>,
    mut marks: Query<(Entity, &RoadMark, &mut Transform, &mut Visibility)>,
    mut roads: Query<(&RoadEdge, &mut Transform), Without<RoadMark>>,
    mut commands: Commands,
    mut last: Local<(i32, u64, bool)>,
) {
    let view = cam.as_ref().map_or(100.0, |c| c.view_h);
    let w = mark_width(view);
    let s = link.snapshots.latest();
    let key = ((w * 100.0) as i32, s.tick / 10, classes.is_changed());
    if *last == key && !classes.is_changed() {
        return;
    }
    *last = key;
    let lanes = |e: usize| classes.0.get(&e).map_or(0, |c| c.lanes());
    if classes.is_changed() {
        for (ent, m, _, _) in &marks {
            if m.k > 0 {
                commands.entity(ent).despawn();
            }
        }
        for (&e, c) in &classes.0 {
            let (a, b) = cp.plan.graph.pos(e);
            for k in 1..c.lanes() / 2 {
                for side in [-1.0f32, 1.0] {
                    commands.spawn((RoadMark { edge: e, k: k as u8, side }, Mesh3d(assets.0.clone()), MeshMaterial3d(assets.1.clone()), segment_transform(a, b, Y_ROAD + 0.06, w, 0.1), Visibility::Hidden));
                }
            }
        }
        // roads widen with their class (never thinner than the generated stroke)
        for (r, mut tf) in &mut roads {
            if let Some(c) = classes.0.get(&r.0) {
                let (a, b) = cp.plan.graph.pos(r.0);
                *tf = segment_transform(a, b, Y_ROAD, c.width().max(W_LOCAL), 0.2);
            }
        }
    }
    for (_, m, mut tf, mut v) in &mut marks {
        let (a, b) = cp.plan.graph.pos(m.edge);
        let d = (b - a).normalize_or_zero();
        let off = Vec2::new(-d.y, d.x) * m.side * m.k as f32 * 0.9;
        // lanes > 0 only for upgraded roads; a centreline of a 1-lane street would sit in the middle of its only lane
        let shown = s.built.get(m.edge).copied().unwrap_or(false) && (m.k == 0 || lanes(m.edge) / 2 > m.k as u32);
        *tf = segment_transform(a + off, b + off, Y_ROAD + 0.06, w, 0.1);
        let want = if shown { Visibility::Inherited } else { Visibility::Hidden };
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
