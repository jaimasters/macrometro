//! Road drawing and visible traffic. Built roads are strokes by class (local thin, arterial wider, both thinner
//! than rail; bridges get a dark casing). Zoomed out: each stroke tints green -> red by volume/capacity.
//! Zoomed in: a capped pool of car dots whose number follows q*t on each edge, slowing and queueing toward the
//! downstream end as the edge saturates. Cars are visual only (D-2): positions come from snapshot flows + clock.
use crate::camera::MapCam;
use crate::zones::{CityPlan, GrowthLink, Y_ROAD};
use bevy::prelude::*;
use city::{RoadClass, SPEED_ARTERIAL, SPEED_LOCAL};
use ui_tools::segment_transform;
use world_gen::rng::{hash3, unit};

const W_LOCAL: f32 = 1.0;
const W_ARTERIAL: f32 = 1.9;
/// Zoomed out beyond this view height: congestion tint instead of car dots.
pub const TINT_VIEW: f32 = 110.0;
/// Car dots are shown at or below this view height.
pub const CAR_VIEW: f32 = 120.0;
/// Hard cap on rendered cars at any zoom (the pool size).
pub const MAX_CARS: usize = 420;
const MAX_PER_EDGE: u32 = 7;
const TINT_BUCKETS: usize = 8;
/// Dots per real car on an edge (low demand still shows life) and visual speed in world units/s per u/min.
const VIS_SCALE: f32 = 2.0;
const VIS_SPEED: f32 = 0.5;
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

fn h01(e: usize, k: u32) -> f32 {
    unit(hash3(0xCA, 0xD07, e as i64, k as i64)) as f32
}

/// Cars to draw on an edge: vehicles on the link (Little's law q*t) scaled for visibility, stably rounded.
pub fn cars_on_edge(flow_vph: f32, minutes: f32, e: usize) -> u32 {
    let x = (flow_vph * minutes / 60.0 * VIS_SCALE).max(0.0);
    let base = x.floor();
    (base as u32 + (h01(e, 99) < x - base) as u32).min(MAX_PER_EDGE)
}

/// Position and heading of car `k` on edge a->b at time `t`. `congestion` = volume/capacity: cars slow and bunch
/// toward their downstream end (the junction), so a saturated edge visibly queues.
pub fn car_pose(a: Vec2, b: Vec2, e: usize, k: u32, t: f32, speed: f32, congestion: f32) -> (Vec2, Vec2) {
    let len = a.distance(b).max(0.1);
    let dir = if k % 2 == 0 { 1.0 } else { -1.0 };
    let phase = (h01(e, k) + dir * t * speed / len).rem_euclid(1.0);
    let pow = 1.0 + 2.5 * congestion.clamp(0.0, 2.0);
    let u = if dir > 0.0 { 1.0 - (1.0 - phase).powf(pow) } else { phase.powf(pow) };
    let d = (b - a) / len;
    (a.lerp(b, u) + Vec2::new(-d.y, d.x) * 0.55 * dir, d * dir)
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
    cp: Res<CityPlan>,
    cam: Option<Res<MapCam>>,
    mut was_on: Local<bool>,
    mut dots: Query<(&CarDot, &mut Transform, &mut Visibility)>,
) {
    let on = cam.as_ref().is_some_and(|c| c.view_h <= CAR_VIEW);
    if !on {
        if *was_on {
            *was_on = false;
            for (_, _, mut v) in &mut dots {
                *v = Visibility::Hidden;
            }
        }
        return;
    }
    *was_on = true;
    let (s, c) = (link.snapshots.latest(), cam.unwrap());
    let half = c.view_h * 0.95 + 10.0;
    // visible edges nearest the camera first, so the cap thins the periphery
    let mut near: Vec<(f32, usize, u32)> = vec![];
    for (e, ed) in cp.plan.graph.edges.iter().enumerate() {
        if !s.built.get(e).copied().unwrap_or(false) {
            continue;
        }
        let (a, b) = cp.plan.graph.pos(e);
        let mid = (a + b) * 0.5;
        if (mid - c.focus).abs().max_element() > half {
            continue;
        }
        let speed = if ed.class == RoadClass::Arterial { SPEED_ARTERIAL } else { SPEED_LOCAL } as f32;
        let slow = s.slow[e].max(1.0);
        let n = cars_on_edge(s.flow[e], ed.len / speed * slow, e);
        if n > 0 {
            near.push((mid.distance(c.focus), e, n));
        }
    }
    near.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
    let slots: Vec<(usize, u32)> = near.iter().flat_map(|&(_, e, n)| (0..n).map(move |k| (e, k))).take(MAX_CARS).collect();
    let t = time.elapsed_secs();
    for (d, mut tf, mut v) in &mut dots {
        let Some(&(e, k)) = slots.get(d.0) else {
            if *v != Visibility::Hidden {
                *v = Visibility::Hidden;
            }
            continue;
        };
        let ed = &cp.plan.graph.edges[e];
        let (a, b) = cp.plan.graph.pos(e);
        let speed = (if ed.class == RoadClass::Arterial { SPEED_ARTERIAL } else { SPEED_LOCAL }) as f32 * VIS_SPEED / s.slow[e].max(1.0);
        let (p, h) = car_pose(a, b, e, k, t, speed, s.vc[e]);
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

    #[test]
    fn fewer_cars_when_flow_drops_and_never_above_the_cap() {
        let total = |q: f32| (0..200).map(|e| cars_on_edge(q, 1.0, e)).sum::<u32>();
        assert!(total(150.0) > total(60.0) && total(60.0) > total(20.0), "{} {} {}", total(150.0), total(60.0), total(20.0));
        assert_eq!(total(0.0), 0);
        assert!((0..50).all(|e| cars_on_edge(1e6, 5.0, e) <= MAX_PER_EDGE));
    }

    #[test]
    fn cars_queue_toward_the_downstream_end_when_congested() {
        let (a, b) = (Vec2::ZERO, Vec2::new(10.0, 0.0));
        let mean_x = |cong: f32| {
            let v: Vec<f32> = (0..40u32).step_by(2).map(|k| car_pose(a, b, 3, k, 1.0, 1.0, cong).0.x).collect();
            v.iter().sum::<f32>() / v.len() as f32
        };
        assert!(mean_x(1.5) > mean_x(0.0) + 1.0, "{} vs {}", mean_x(1.5), mean_x(0.0));
        let (p, h) = car_pose(a, b, 3, 1, 0.5, 1.0, 0.0); // odd cars drive the other way, in the other lane
        assert!(h.x < 0.0 && p.y.abs() > 0.3);
    }
}
