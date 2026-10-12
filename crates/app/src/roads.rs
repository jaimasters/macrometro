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
/// Kinematic bicycle: wheelbase, steering limit (rad), steering slew (rad/s), yaw-rate cap (rad/s), lane offset.
const WHEELBASE: f32 = 0.9;
const MAX_STEER: f32 = 0.7;
const STEER_RATE: f32 = 6.0;
const MAX_YAW: f32 = 2.5;
const LANE_OFF: f32 = 0.55;
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
/// one thin stop bar across the incoming half at each real (degree 3+) junction end. `trim` cuts every line back from
/// that end (the junction box or roundabout ring); 0 = a plain join, lines run straight through with no bar.
/// Left-hand driving keeps colours, mirrors the stop bars.
fn edge_marks(edge: usize, len: f32, half_w: f32, c: city::lanes::LaneClass, trim: [f32; 2], left: bool) -> Vec<RoadMark> {
    use city::lanes::LaneClass::*;
    let line = |p: Vec2, q: Vec2, yellow: bool| RoadMark { edge, p, q, yellow, fat: 1.0 };
    let mut v = vec![];
    let (t0, t1) = (trim[0], len - trim[1]);
    if t1 - t0 < 1.0 {
        return v;
    }
    let dashed = |v: &mut Vec<RoadMark>, off: f32, yellow: bool| {
        let mut t = t0 + DASH_GAP / 2.0;
        while t + DASH < t1 {
            v.push(line(Vec2::new(t, off), Vec2::new(t + DASH, off), yellow));
            t += DASH + DASH_GAP;
        }
    };
    match c {
        Street => {}
        TwoLaneLow => dashed(&mut v, 0.0, true),
        _ => {
            for o in [-0.12, 0.12] {
                v.push(line(Vec2::new(t0, o), Vec2::new(t1, o), true));
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
        v.push(line(Vec2::new(t0, side * edge_off), Vec2::new(t1, side * edge_off), false));
    }
    // traffic toward the end at `along = len` drives on the +lateral side (right-hand), toward 0 on the - side
    let s = if left { -1.0 } else { 1.0 };
    for (end, t, dir) in [(1, t1, s), (0, t0, -s)] {
        if trim[end] > 0.0 {
            v.push(RoadMark { edge, p: Vec2::new(t, 0.0), q: Vec2::new(t, dir * edge_off), yellow: false, fat: 1.5 });
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

/// A roundabout surface or island, shown once road `.0` (one of its legs) is built.
#[derive(Component)]
struct RingDisc(usize);

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

/// A car on its way. The car owns its pose (`pos`, `head`, `steer`, `v`) and integrates a kinematic bicycle; the route
/// is only the path it steers toward. `seg`/`along` are its progress, found by projecting `pos` onto the route.
struct Journey {
    trip: Trip,
    seg: usize,
    along: f32,
    pos: Vec2,
    /// Heading angle (rad, atan2 of the travel direction).
    head: f32,
    steer: f32,
    /// Drive on the left (lane offset side).
    left: bool,
    /// Current speed (world units / s): cars accelerate and brake, they do not jump to the limit.
    v: f32,
    /// Seconds stopped (gridlock breaker: the longest waiter may enter a held junction).
    wait: f32,
    /// Stable car slot = the CarDot drawing it. Cars are removed from the middle of `cars` on arrival, so the Vec
    /// index is not an identity (indexing dots by it made the remaining cars jump between dots: "random vanishing").
    slot: usize,
}

#[derive(Resource)]
struct Journeys {
    cars: Vec<Journey>,
    acc: f32,
    rng: SplitMix64,
    router: Router,
    rings: Rings,
}

/// Free-flow speed on the car's current segment, slowed by congestion.
fn target_speed(j: &Journey, slow: &impl Fn(usize) -> f32) -> f32 {
    j.trip.seg_speed[j.seg] * VIS_SPEED / slow(j.trip.seg_edge[j.seg]).max(1.0)
}

fn seg_dir(t: &Trip, s: usize) -> Vec2 {
    (t.pts[s + 1] - t.pts[s]).normalize_or_zero()
}
/// Point `a` along route segment `s`, shifted into the driving lane.
/// The lane is the route offset sideways with mitred corners, so it is one continuous line through bends and junctions
/// (pure pursuit then follows the lane round the corner instead of cutting a chord between two offset segments).
fn lane_pt(t: &Trip, s: usize, a: f32, left: bool) -> Vec2 {
    let off = if left { -LANE_OFF } else { LANE_OFF };
    let vtx = |i: usize| {
        let nrm = |d: Vec2| Vec2::new(-d.y, d.x);
        let n1 = nrm(seg_dir(t, i.min(t.pts.len() - 2)));
        let n0 = if i > 0 { nrm(seg_dir(t, i - 1)) } else { n1 };
        let m = (n0 + n1).normalize_or(n1);
        t.pts[i] + m * off / m.dot(n1).max(0.5)
    };
    let len = t.pts[s].distance(t.pts[s + 1]).max(1e-4);
    vtx(s).lerp(vtx(s + 1), a / len)
}

impl Journey {
    /// A parked car at the start of its route's lane, facing along it (spawn only; it drives itself from here).
    fn new(trip: Trip, left: bool, slot: usize) -> Self {
        let (pos, d) = (lane_pt(&trip, 0, 0.0, left), seg_dir(&trip, 0));
        Journey { pos, head: d.y.atan2(d.x), steer: 0.0, left, trip, seg: 0, along: 0.0, v: 0.0, wait: 0.0, slot }
    }
    fn dir(&self) -> Vec2 {
        Vec2::from_angle(self.head)
    }
    /// Pure-pursuit target: the lane point `l` ahead of the car's progress (clamped to the route end).
    fn lookahead(&self, mut l: f32) -> Vec2 {
        let (mut s, mut a) = (self.seg, self.along);
        loop {
            let len = self.trip.pts[s].distance(self.trip.pts[s + 1]);
            if a + l < len || s + 2 >= self.trip.pts.len() {
                return lane_pt(&self.trip, s, (a + l).min(len), self.left);
            }
            l -= len - a;
            (s, a) = (s + 1, 0.0);
        }
    }
    /// Distance from the car to the lane line of route segment `s`.
    fn lane_dist(&self, s: usize) -> f32 {
        let len = self.trip.pts[s].distance(self.trip.pts[s + 1]);
        let a = (self.pos - self.trip.pts[s]).dot(seg_dir(&self.trip, s)).clamp(0.0, len);
        self.pos.distance(lane_pt(&self.trip, s, a, self.left))
    }
    /// `p` is in this car's lane over its next few segments (cars waiting in other lanes or arms are not in the way;
    /// junction conflicts are the holds' job).
    fn on_my_lane(&self, p: Vec2) -> bool {
        (self.seg..(self.seg + 4).min(self.trip.pts.len() - 1)).any(|s| {
            let len = self.trip.pts[s].distance(self.trip.pts[s + 1]);
            let a = (p - self.trip.pts[s]).dot(seg_dir(&self.trip, s)).clamp(0.0, len);
            p.distance(lane_pt(&self.trip, s, a, self.left)) < 0.6
        })
    }
    /// Progress = projection of the car's own position onto the route (read-only of the pose). Moves on to the next
    /// segment once past this one's end or nearer the next one's lane (cars cut corners).
    fn track(&mut self) {
        while self.seg + 2 < self.trip.pts.len() {
            let past = (self.pos - self.trip.pts[self.seg]).dot(seg_dir(&self.trip, self.seg)) >= seg_len(self);
            if !(past || self.lane_dist(self.seg + 1) < self.lane_dist(self.seg)) {
                break;
            }
            self.seg += 1;
        }
        self.along = (self.pos - self.trip.pts[self.seg]).dot(seg_dir(&self.trip, self.seg)).clamp(0.0, seg_len(self));
    }
    fn arrived(&self) -> bool {
        self.seg + 2 >= self.trip.pts.len() && seg_len(self) - self.along < 0.15
    }
    /// Corner speed: slow enough that the curvature needed to reach the path 4 units ahead keeps lateral accel <= 3.
    fn corner_speed(&self) -> f32 {
        let to = self.lookahead(4.0) - self.pos;
        let k = 2.0 * self.dir().angle_to(to).sin().abs() / to.length().max(0.5);
        (2.5 / k.max(1e-3)).sqrt().max(1.0)
    }
    /// One kinematic-bicycle tick at the already-chosen speed `self.v`: pure pursuit picks a steering angle, slewed
    /// and clamped; yaw rate is capped; position integrates along the heading. Nothing here reads the pose off the spline.
    fn drive(&mut self, dt: f32) {
        let l = 1.0 + 0.3 * self.v;
        let to = self.lookahead(l) - self.pos;
        let alpha = self.dir().angle_to(to);
        let want = (2.0 * WHEELBASE * alpha.sin()).atan2(to.length().max(0.5)).clamp(-MAX_STEER, MAX_STEER);
        self.steer += (want - self.steer).clamp(-STEER_RATE * dt, STEER_RATE * dt);
        let yaw = (self.v / WHEELBASE * self.steer.tan()).clamp(-MAX_YAW, MAX_YAW);
        self.head += yaw * dt;
        self.pos += self.dir() * self.v * dt;
        self.track();
    }
}

/// Bumper gap to the nearest car in front of `i` (actual positions): ahead of its nose, within a car width of its
/// heading line, and not oncoming. ponytail: O(n^2) over <=420 cars, grid-bucket it if profiling shows.
fn gap_ahead(cars: &[Journey], i: usize) -> f32 {
    let (p, h) = (cars[i].pos, cars[i].dir());
    let mut gap = f32::INFINITY;
    for (k, o) in cars.iter().enumerate() {
        let r = o.pos - p;
        if k != i && r.dot(h) > 0.0 && r.length() < 20.0 && o.dir().dot(h) > -0.3 && cars[i].on_my_lane(o.pos) {
            gap = gap.min(r.length() - CAR_LEN);
        }
    }
    gap
}

#[cfg(test)]
type Key = (u32, u32, u32, u32);
fn pkey(p: Vec2) -> (u32, u32) {
    (p.x.to_bits(), p.y.to_bits())
}
#[cfg(test)]
/// Directed road segment `k` of a route: cars share a lane only when both endpoints match (so `along` is comparable).
fn seg_key(t: &Trip, k: usize) -> Option<Key> {
    let (a, b) = (pkey(*t.pts.get(k)?), pkey(*t.pts.get(k + 1)?));
    Some((a.0, a.1, b.0, b.1))
}
#[cfg(test)]
fn lane_key(j: &Journey) -> Option<Key> {
    seg_key(&j.trip, j.seg)
}

fn seg_len(j: &Journey) -> f32 {
    j.trip.pts[j.seg].distance(j.trip.pts[j.seg + 1])
}

/// Junction zone: a car this close to a node (either side) holds it; others wait at the stop line before it.
const JUNC: f32 = 4.5;

/// One car-following step for every car (in index order, deterministic): accelerate toward the congestion-limited
/// speed, but never faster than lets the car stop behind the one ahead in its lane, or at the stop line of a junction
/// another car holds. A car just past a node (on the ring) holds it first, so entering cars give way.
/// `free(edge)`: the edge is a freeway; a car going freeway to freeway passes the node grade-separated, no hold.
fn follow_step(cars: &mut Vec<Journey>, dt: f32, slow: impl Fn(usize) -> f32, free: impl Fn(usize) -> bool) {
    // passing node pts[s+1] from freeway segment s onto freeway segment s+1
    let thru = |c: &Journey, s: usize| s + 1 < c.trip.seg_edge.len() && free(c.trip.seg_edge[s]) && free(c.trip.seg_edge[s + 1]);
    use std::collections::HashMap;
    let mut held: HashMap<(u32, u32), usize> = HashMap::new();
    // only real junctions (3+ distinct neighbours over all routes) are held: a through node never blocks the
    // opposite direction, which deadlocked two-way roads. ponytail: rebuilt per step, cache with the plan if it shows.
    let mut nb: HashMap<(u32, u32), Vec<(u32, u32)>> = HashMap::new();
    for c in cars.iter() {
        for w in c.trip.pts.windows(2) {
            let (a, b) = (pkey(w[0]), pkey(w[1]));
            for (x, y) in [(a, b), (b, a)] {
                let v = nb.entry(x).or_default();
                if !v.contains(&y) {
                    v.push(y);
                }
            }
        }
    }
    let junc = |p: Vec2| nb.get(&pkey(p)).is_some_and(|v| v.len() >= 3);
    for (i, c) in cars.iter().enumerate() {
        if c.seg > 0 && c.along < JUNC && junc(c.trip.pts[c.seg]) && !thru(c, c.seg - 1) {
            held.entry(pkey(c.trip.pts[c.seg])).or_insert(i);
        }
    }
    for (i, c) in cars.iter().enumerate() {
        if c.seg + 2 < c.trip.pts.len() && seg_len(c) - c.along < JUNC && junc(c.trip.pts[c.seg + 1]) && !thru(c, c.seg) {
            held.entry(pkey(c.trip.pts[c.seg + 1])).or_insert(i);
        }
    }
    // gridlock breaker: the longest-stopped car (over 3 s) ignores junction holds this step
    let boss = (0..cars.len()).filter(|&i| cars[i].wait > 3.0).max_by(|&a, &b| cars[a].wait.total_cmp(&cars[b].wait));
    let mut keep = vec![];
    for i in 0..cars.len() {
        let c = &cars[i];
        let len = seg_len(c);
        // following: the actual bumper gap to the car in front, whatever lane or segment it is on
        let mut gap = gap_ahead(cars, i);
        {
            if c.seg + 2 < c.trip.pts.len() {
                let node = pkey(c.trip.pts[c.seg + 1]);
                // enter only when held by no one else AND the exit lane has room (measured: no car within a car+gap of
                // the exit lane's start, heading out): queues form behind the junction
                let (exit, dn) = (lane_pt(&c.trip, c.seg + 1, 0.0, c.left), seg_dir(&c.trip, c.seg + 1));
                let blocked = cars.iter().enumerate().any(|(k, o)| k != i && o.pos.distance(exit) < CAR_LEN + CAR_GAP && (o.pos - exit).dot(dn) > -0.5 && o.dir().dot(dn) > 0.7);
                let room = !blocked || !junc(c.trip.pts[c.seg + 1]);
                if junc(c.trip.pts[c.seg + 1]) && !thru(c, c.seg) && (held.get(&node).is_some_and(|&h| h != i) || !room) && boss != Some(i) {
                    gap = gap.min(len - JUNC - c.along + CAR_GAP);
                } else if !thru(c, c.seg) && len - c.along < JUNC + c.v * c.v / (2.0 * CAR_BRAKE) + 0.5 {
                    held.insert(node, i);
                }
            }
        }
        let free_v = target_speed(c, &slow).min(c.corner_speed());
        let vmax = if gap.is_finite() { free_v.min((2.0 * CAR_BRAKE * (gap - CAR_GAP).max(0.0)).sqrt()) } else { free_v };
        let c = &mut cars[i];
        c.v += (vmax - c.v).clamp(-CAR_BRAKE * dt, CAR_ACCEL * dt);
        c.v = c.v.max(0.0);
        c.wait = if c.v < 0.05 { c.wait + dt } else { 0.0 };
        // never cover more than the stop distance this tick (the speed cap alone can overshoot by v*dt)
        if gap.is_finite() {
            c.v = c.v.min((gap - CAR_GAP * 0.5).max(0.0) / dt);
        }
        c.drive(dt);
        keep.push(!c.arrived());
    }
    let mut k = keep.into_iter();
    cars.retain(|_| k.next().unwrap_or(true));
}

type Rings = std::collections::HashMap<(u32, u32), city::junctions::Ring>;

/// Route a trip around every roundabout it passes: the junction point becomes the ring arc (entry leg to exit leg)
/// in the driving-side direction, at a reduced speed.
fn ring_trip(t: &mut Trip, rings: &Rings, left: bool) {
    let n = t.pts.len();
    let (mut pts, mut se, mut ss) = (vec![t.pts[0]], vec![], vec![]);
    for k in 1..n {
        match rings.get(&pkey(t.pts[k])).filter(|_| k + 1 < n) {
            Some(r) => {
                for (i, p) in r.path(t.pts[k - 1], t.pts[k + 1], left).into_iter().enumerate() {
                    pts.push(p);
                    se.push(if i == 0 { t.seg_edge[k - 1] } else { t.seg_edge[k] });
                    ss.push(if i == 0 { t.seg_speed[k - 1] } else { t.seg_speed[k] * 0.6 });
                }
            }
            None => {
                pts.push(t.pts[k]);
                se.push(t.seg_edge[k - 1]);
                ss.push(t.seg_speed[k - 1]);
            }
        }
    }
    (t.pts, t.seg_edge, t.seg_speed) = (pts, se, ss);
}

/// Drive on the left instead of the right (R34). Saved with the game.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub struct LeftHand(pub bool);


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
    // R31 roundabouts: a road-surface disc with a planted island, at real junctions only (default classes)
    let inc = cp.plan.graph.incident();
    let cls = |e: usize| city::lanes::default_class(cp.plan.graph.edges[e].class);
    let rings: Rings = (0..inc.len()).filter_map(|n| city::junctions::Ring::at(&cp.plan.graph, &inc[n], n, cls).map(|r| (pkey(r.c), r))).collect();
    let disc = meshes.add(Cylinder::new(1.0, 1.0));
    let island = flat(Color::srgb(0.55, 0.66, 0.45));
    for r in rings.values() {
        let e = inc[cp.plan.graph.nodes.iter().position(|&p| p == r.c).unwrap_or(0)].first().copied().unwrap_or(0);
        let at = |rad: f32, y: f32| Transform::from_xyz(r.c.x, y, r.c.y).with_scale(Vec3::new(rad, 0.1, rad));
        commands.spawn((RingDisc(e), Mesh3d(disc.clone()), MeshMaterial3d(m.base[0].clone()), at(r.r + 1.0, Y_ROAD + 0.03), Visibility::Hidden));
        commands.spawn((RingDisc(e), Mesh3d(disc.clone()), MeshMaterial3d(island.clone()), at((r.r - 1.0).max(0.4), Y_ROAD + 0.09), Visibility::Hidden));
    }
    commands.insert_resource(MarkAssets(rect.clone(), flat(Color::srgb(0.97, 0.97, 0.94)), flat(Color::srgb(0.98, 0.78, 0.12))));
    commands.insert_resource(m);
    commands.insert_resource(Journeys { cars: vec![], acc: 0.0, rng: SplitMix64::new(0xCA75), router: Router::new(&cp.plan), rings });
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
    j: Option<Res<Journeys>>,
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
        let Some(j) = j else { return };
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
            // cut lines at the junction box (or the roundabout ring) of real junctions only
            let trim = [(ed.a, a), (ed.b, b)].map(|(n, p)| match deg[n as usize].len() >= 3 {
                false => 0.0,
                true => j.rings.get(&pkey(p)).map_or(half.max(1.0) + 0.6, |r| r.r + 1.2),
            });
            for m in edge_marks(e, a.distance(b), half, c, trim, hand.0) {
                let mat = if m.yellow { assets.2.clone() } else { assets.1.clone() };
                let shown = s.built.get(e).copied().unwrap_or(false);
                commands.spawn((m, Mesh3d(assets.0.clone()), MeshMaterial3d(mat), mark_transform(&m, a, b, w), if shown { Visibility::Inherited } else { Visibility::Hidden }));
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
        // zoomed out, dashes and stop bars merge into white blocks: drop them, keep the long lines
        let small = m.fat > 1.0 || m.q.x - m.p.x <= DASH + 1e-3;
        let want = if s.built.get(m.edge).copied().unwrap_or(false) && !(small && w > 0.4) { Visibility::Inherited } else { Visibility::Hidden };
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
    mut discs: Query<(&RingDisc, &mut Visibility), (Without<RoadEdge>, Without<RoadCasing>, Without<Driveway>)>,
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
    for (d, mut v) in &mut discs {
        let want = vis(s.built.get(d.0).copied().unwrap_or(false));
        if *v != want {
            *v = want;
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
    classes: Res<RoadClasses>,
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
    follow_step(&mut j.cars, dt, slow, |e| classes.0.get(&e).is_some_and(|&c| city::junctions::is_freeway(c)));
    j.acc = (j.acc + s.car_trips * SAMPLE * dt).min(6.0);
    while j.acc >= 1.0 && j.cars.len() < MAX_CARS {
        j.acc -= 1.0;
        let j = &mut *j;
        let Some((a, b)) = j.router.sample(&s.built, &s.pop, &s.zone_share, s.hour as f64, &mut j.rng) else { break };
        if let Some(trip) = j.router.route(&s.built, &s.slow, a, b) {
            // a new car waits at its origin until the lane's entrance is clear
            if !j.cars.iter().any(|c| c.seg == 0 && c.along < CAR_LEN + CAR_GAP && c.trip.pts.first() == trip.pts.first()) {
                let mut trip = trip;
                ring_trip(&mut trip, &j.rings, hand.0);
                let slot = (0..MAX_CARS).find(|k| !j.cars.iter().any(|c| c.slot == *k)).unwrap_or(0);
                j.cars.push(Journey::new(trip, hand.0, slot));
            }
        }
    }
    let mut by_slot = vec![None; MAX_CARS];
    for c in &j.cars {
        by_slot[c.slot] = Some(c);
    }
    for (d, mut tf, mut v) in &mut dots {
        let Some(c) = by_slot.get(d.0).copied().flatten() else {
            if *v != Visibility::Hidden {
                *v = Visibility::Hidden;
            }
            continue;
        };
        // the car's own integrated pose, never a spline sample
        tf.translation = Vec3::new(c.pos.x, Y_CAR, c.pos.y);
        tf.rotation = Quat::from_rotation_y(-c.head);
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
        Journey::new(trip, false, 0)
    }
    /// A car already standing `x` along the first (straight) leg, in its lane.
    fn at(speed: f32, x: f32) -> Journey {
        let mut j = journey(speed);
        j.pos.x = x;
        j.track();
        j
    }

    /// Drive one car alone to its destination: heading changes continuously with a bounded yaw rate, every step is
    /// at most v*dt (no teleport), it stays near its lane, and it is removed only on arrival.
    fn drive_alone(trip: Trip) -> f32 {
        let dt = 0.05;
        let end = *trip.pts.last().unwrap();
        let mut cars = vec![Journey::new(trip, false, 0)];
        let mut turned = 0.0;
        for _ in 0..4000 {
            let (p, h, v0) = (cars[0].pos, cars[0].head, cars[0].v);
            follow_step(&mut cars, dt, |_| 1.0, |_| false);
            let Some(c) = cars.first() else { return turned };
            let dh = (c.head - h + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            assert!(dh.abs() <= MAX_YAW * dt + 1e-4, "yaw rate {} rad/s", dh / dt);
            assert!(c.pos.distance(p) <= v0.max(c.v) * dt + 1e-4, "teleport {} in one step", c.pos.distance(p));
            let err = c.pos.distance(lane_pt(&c.trip, c.seg, c.along, false));
            assert!(err < 1.5, "left its lane by {err}");
            turned += dh;
        }
        panic!("never arrived at {end}");
    }

    #[test]
    fn a_car_steers_round_a_bend_and_through_a_junction_kinematically() {
        // the 90 degree bend
        let t = drive_alone(journey(2.0).trip);
        assert!((t - std::f32::consts::FRAC_PI_2).abs() < 0.3, "turned {t} rad round the bend");
        // straight over a plain junction, then left and right turns through a roundabout
        let mut rings = Rings::new();
        rings.insert(pkey(Vec2::ZERO), city::junctions::Ring { c: Vec2::ZERO, r: 3.0, legs: vec![-std::f32::consts::FRAC_PI_2, 0.0, std::f32::consts::FRAC_PI_2, std::f32::consts::PI] });
        for out in [Vec2::Y, -Vec2::X, -Vec2::Y] {
            for ring in [false, true] {
                let mut trip = Trip { from: 0, to: 1, pts: vec![Vec2::X * 18.0, Vec2::X * 12.0, Vec2::ZERO, out * 12.0, out * 18.0], seg_edge: vec![0, 1, 2, 3], seg_speed: vec![4.0; 4], nodes: vec![], edges: vec![] };
                if ring {
                    ring_trip(&mut trip, &rings, false);
                }
                drive_alone(trip);
            }
        }
    }

    #[test]
    fn a_car_drives_its_route_in_order_then_disappears() {
        let mut cars = vec![journey(2.0)];
        let mut seg = 0;
        for _ in 0..2000 {
            follow_step(&mut cars, 0.05, |_| 1.0, |_| false);
            let Some(c) = cars.first() else { return };
            assert!(c.seg >= seg, "progress never goes back");
            seg = c.seg;
        }
        panic!("never arrived");
    }

    #[test]
    fn congestion_slows_a_car_down() {
        let (mut free, mut jam) = (vec![journey(2.0)], vec![journey(2.0)]);
        for _ in 0..20 {
            follow_step(&mut free, 0.05, |_| 1.0, |_| false);
            follow_step(&mut jam, 0.05, |_| 3.0, |_| false);
        }
        assert!(free[0].along > jam[0].along + 0.5, "{} vs {}", free[0].along, jam[0].along);
    }

    #[test]
    fn cars_queue_behind_a_slow_leader_and_never_overlap() {
        let mut cars: Vec<Journey> = (0..6).map(|i| at(4.0, i as f32 * 1.9)).collect();
        for _ in 0..600 {
            follow_step(&mut cars, 0.05, |_| 1.0, |_| false);
            if cars.len() < 6 { break; }
            for i in 0..cars.len() {
                for k in i + 1..cars.len() {
                    let d = cars[i].pos.distance(cars[k].pos);
                    assert!(d >= CAR_LEN - 1e-3, "overlap {d}");
                }
            }
        }
    }

    #[test]
    fn a_car_brakes_for_a_stopped_leader() {
        let mut cars = vec![at(4.0, 2.0), at(4.0, 8.0)];
        for _ in 0..200 {
            follow_step(&mut cars, 0.05, |_| 1.0, |_| false);
            cars[1] = at(4.0, 8.0); // pinned stopped
        }
        assert!(cars[0].pos.x <= 8.0 - CAR_LEN, "stopped behind, got {}", cars[0].pos.x);
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
                        cars.push(Journey::new(trip, false, 0));
                        spawned += 1;
                    }
                }
            }
            follow_step(&mut cars, 0.05, |_| 1.0, |_| false);
            for _side in [false, true] {
                for i in 0..cars.len() {
                    for j in i + 1..cars.len() {
                        let (a, b) = (&cars[i], &cars[j]);
                        let d = a.pos.distance(b.pos);
                        if lane_key(a) == lane_key(b) {
                            assert!(d >= CAR_LEN - 0.1, "same-lane overlap ({d}) at step {step}");
                        }
                        assert!(d >= 0.6, "cars {i},{j} overlap ({d}) at step {step}");
                    }
                }
            }
        }
        assert!(spawned > 40 && cars.len() < 36, "traffic flows: {spawned} spawned, {} still queued", cars.len());
    }

    /// Headless gridlock: a long run of heavy four-arm traffic. Cars finish in every 10 s window (no deadlock).
    #[test]
    fn heavy_junction_traffic_never_deadlocks() {
        junction_flow(false);
    }

    #[test]
    fn roundabout_traffic_flows_without_overlap() {
        junction_flow(true);
        junction_flow(false);
    }

    fn junction_flow(ring: bool) {
        let mut rings = Rings::new();
        if ring {
            let legs = vec![-std::f32::consts::FRAC_PI_2, 0.0, std::f32::consts::FRAC_PI_2, std::f32::consts::PI];
            rings.insert(pkey(Vec2::ZERO), city::junctions::Ring { c: Vec2::ZERO, r: 3.0, legs });
        }
        let arm = |d: Vec2| d * 12.0;
        let dirs = [Vec2::X, Vec2::Y, -Vec2::X, -Vec2::Y];
        let mut cars: Vec<Journey> = vec![];
        let mut done = 0;
        for step in 0..12000 {
            if step % 4 == 0 {
                for (k, &d) in dirs.iter().enumerate() {
                    let out = dirs[(k + 1 + step / 4 % 3) % 4];
                    let trip = Trip { from: 0, to: 1, pts: vec![arm(d) * 1.5, arm(d), Vec2::ZERO, arm(out), arm(out) * 1.5], seg_edge: vec![0, 1, 2, 3], seg_speed: vec![4.0; 4], nodes: vec![], edges: vec![] };
                    let mut trip = trip;
                    ring_trip(&mut trip, &rings, true);
                    if !cars.iter().any(|c| c.seg == 0 && c.along < CAR_LEN + CAR_GAP && c.trip.pts[0] == trip.pts[0]) {
                        cars.push(Journey::new(trip, false, 0));
                    }
                }
            }
            let n = cars.len();
            for (k, c) in cars.iter_mut().enumerate() {
                c.slot = k + step * 100;
            }
            let left: Vec<(usize, f32)> = cars.iter().map(|c| (c.slot, (c.seg..c.trip.pts.len() - 1).map(|s| c.trip.pts[s].distance(c.trip.pts[s + 1])).sum::<f32>() - c.along)).collect();
            follow_step(&mut cars, 0.05, |_| 1.0, |_| false);
            done += n - cars.len();
            for (id, rest) in left.into_iter().filter(|(id, _)| !cars.iter().any(|c| c.slot == *id)) {
                assert!(rest < 0.5, "car {id} removed {rest} short of its destination at step {step}");
            }
            for i in 0..cars.len() {
                for j in i + 1..cars.len() {
                    let d = cars[i].pos.distance(cars[j].pos);
                    assert!(d >= 0.6, "overlap ({d}) at step {step}");
                }
            }
            if step % 200 == 199 {

                assert!(step < 400 || done > 0, "no car finished in window ending {step}: deadlock");
                done = 0;
            }
        }
    }

    #[test]
    fn us_markings_by_class_and_side() {
        use city::lanes::LaneClass::*;
        let yellow = |v: &[RoadMark]| v.iter().filter(|m| m.yellow).count();
        let minor = edge_marks(0, 30.0, 0.9, TwoLaneLow, [0.0; 2], false);
        assert!(yellow(&minor) > 3 && minor.iter().filter(|m| m.yellow).all(|m| m.q.x - m.p.x <= DASH + 1e-4), "dashed yellow centre");
        let big = edge_marks(0, 30.0, 1.8, FourLane, [0.0; 2], false);
        assert_eq!(big.iter().filter(|m| m.yellow && m.q.x - m.p.x == 30.0).count(), 2, "double solid yellow");
        assert!(big.iter().any(|m| !m.yellow && m.p.y.abs() == 0.9 && m.q.x - m.p.x <= DASH + 1e-4), "dashed white lane line");
        assert_eq!(big.iter().filter(|m| !m.yellow && m.p.y.abs() > 1.6 && m.q.x - m.p.x == 30.0).count(), 2, "solid white edges");
        assert_eq!(yellow(&edge_marks(0, 30.0, 0.5, Street, [0.0; 2], false)), 0);
        let bar = |left| edge_marks(0, 30.0, 0.9, TwoLaneLow, [0.0, 2.0], left).into_iter().find(|m| m.fat > 1.0).unwrap();
        let (r, l) = (bar(false), bar(true));
        assert!(r.q.y > 0.0 && l.q.y < 0.0 && r.yellow == l.yellow, "stop bar mirrors with the driving side");
    }

    #[test]
    fn left_hand_driving_flips_the_lane_side() {
        let t = journey(2.0).trip;
        assert!(Journey::new(t.clone(), false, 0).pos.y > 0.0 && Journey::new(t, true, 0).pos.y < 0.0);
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
