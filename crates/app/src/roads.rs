//! Road drawing and visible traffic. Built roads are strokes by class (local thin, arterial wider, both thinner
//! than rail; bridges get a dark casing). Zoomed out: each stroke tints green -> red by volume/capacity.
//! Zoomed in: real car journeys (R28). Each car starts at a home building, drives the fastest route over the roads
//! built right now to a destination building and disappears on arrival. Trips are sampled from the same demand and
//! mode-choice model as the traffic assignment (home by population and car share, destination by attraction and
//! distance, work vs shop by the hour), at a rate that follows the simulated car trips, slowed by the live
//! congestion. A capped pool of 420 dots draws them. Cars are visual only (D-2).
use crate::camera::MapCam;
use crate::zones::{CityPlan, GrowthLink, Y_ROAD};
use bevy::prelude::*;
use city::{RoadClass, Router, Trip};
use ui_tools::segment_transform;
use world_gen::rng::SplitMix64;

const W_LOCAL: f32 = 1.0;
const W_ARTERIAL: f32 = 1.9;
/// Zoomed out beyond this view height: congestion tint instead of car dots.
pub const TINT_VIEW: f32 = 110.0;
/// Car dots are shown at or below this view height.
pub const CAR_VIEW: f32 = 120.0;
/// Hard cap on rendered cars at any zoom (the pool size).
pub const MAX_CARS: usize = 420;
const TINT_BUCKETS: usize = 8;
/// Visual speed in world units per second per (unit/min of road speed): a 100-unit trip takes ~10-15 s.
const VIS_SPEED: f32 = 1.5;
/// Journeys started per second per simulated car trip/hour (a sample: the pool is capped anyway).
const SAMPLE: f32 = 0.015;
const Y_CAR: f32 = 0.62;
const NEUTRAL: u8 = 255;

#[derive(Component)]
struct RoadEdge(usize);

#[derive(Component)]
struct RoadCasing(usize);

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
}

#[derive(Resource)]
struct Journeys {
    cars: Vec<Journey>,
    acc: f32,
    rng: SplitMix64,
    router: Router,
}

/// Move a journey `dt` seconds along its route; `slow(edge)` = congestion slowdown. False once it has arrived.
fn advance(j: &mut Journey, mut dt: f32, slow: impl Fn(usize) -> f32) -> bool {
    while j.seg + 1 < j.trip.pts.len() {
        let len = j.trip.pts[j.seg].distance(j.trip.pts[j.seg + 1]).max(1e-3);
        let speed = j.trip.seg_speed[j.seg] * VIS_SPEED / slow(j.trip.seg_edge[j.seg]).max(1.0);
        let left = (len - j.along) / speed;
        if dt < left {
            j.along += speed * dt;
            return true;
        }
        dt -= left;
        j.seg += 1;
        j.along = 0.0;
    }
    false
}

/// Position and heading of a journey (driving on one side of the road).
fn journey_pose(j: &Journey) -> (Vec2, Vec2) {
    let (a, b) = (j.trip.pts[j.seg], j.trip.pts[j.seg + 1]);
    let d = (b - a).normalize_or_zero();
    (a + d * j.along + Vec2::new(-d.y, d.x) * 0.55, d)
}

pub struct RoadsPlugin;

impl Plugin for RoadsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_roads).add_systems(Update, (update_roads, drive_cars));
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
    commands.insert_resource(m);
    commands.insert_resource(Journeys { cars: vec![], acc: 0.0, rng: SplitMix64::new(0xCA75), router: Router::new(&cp.plan) });
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
    mut j: ResMut<Journeys>,
    mut was_on: Local<bool>,
    mut dots: Query<(&CarDot, &mut Transform, &mut Visibility)>,
) {
    let on = cam.as_ref().is_some_and(|c| c.view_h <= CAR_VIEW);
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
    j.cars.retain_mut(|c| advance(c, dt, slow));
    j.acc = (j.acc + s.car_trips * SAMPLE * dt).min(6.0);
    while j.acc >= 1.0 && j.cars.len() < MAX_CARS {
        j.acc -= 1.0;
        let j = &mut *j;
        let Some((a, b)) = j.router.sample(&s.built, &s.pop, &s.zone_share, s.hour as f64, &mut j.rng) else { break };
        if let Some(trip) = j.router.route(&s.built, &s.slow, a, b) {
            j.cars.push(Journey { trip, seg: 0, along: 0.0 });
        }
    }
    for (d, mut tf, mut v) in &mut dots {
        let Some(c) = j.cars.get(d.0) else {
            if *v != Visibility::Hidden {
                *v = Visibility::Hidden;
            }
            continue;
        };
        let (p, h) = journey_pose(c);
        tf.translation = Vec3::new(p.x, Y_CAR, p.y);
        tf.rotation = Quat::from_rotation_y(-h.y.atan2(h.x));
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
        Journey { trip, seg: 0, along: 0.0 }
    }

    #[test]
    fn a_car_drives_its_route_in_order_then_disappears() {
        let mut j = journey(2.0); // 3 world units/s
        assert!(advance(&mut j, 2.0, |_| 1.0));
        assert_eq!(j.seg, 0);
        assert!((journey_pose(&j).0.x - 6.0).abs() < 1e-4);
        assert!(advance(&mut j, 2.0, |_| 1.0), "rounds the corner onto the next road");
        assert_eq!(j.seg, 1);
        assert!(journey_pose(&j).0.y > 1.0 && journey_pose(&j).1.y > 0.9);
        assert!(!advance(&mut j, 10.0, |_| 1.0), "arrived: despawn");
    }

    #[test]
    fn congestion_slows_a_car_down() {
        let (mut free, mut jam) = (journey(2.0), journey(2.0));
        advance(&mut free, 1.0, |_| 1.0);
        advance(&mut jam, 1.0, |_| 3.0);
        assert!((free.along - 3.0).abs() < 1e-4 && (jam.along - 1.0).abs() < 1e-4);
    }
}
