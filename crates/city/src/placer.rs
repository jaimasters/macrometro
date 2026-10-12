//! Rail track placement (R21, C1): auto-routes a track between two stations as circular arcs, steered by the mouse,
//! avoiding buildings and roads where there is room, and prices what it cannot avoid: buildings in the swept
//! footprint are demolished at eminent-domain cost (scaled by density) plus a hit to standing, roads become
//! level crossings (upgradeable to a bridge or underpass), water needs a bridge priced by span.
//! `eminent_domain` is generic over a swept strip so road widening (R24) can reuse it.
use crate::roadgen::{RoadClass, TownPlan};
use bevy::math::Vec2;
use transit::{Crossing, CrossingKind};

pub const DEFAULT_RADIUS: f32 = 10.0;
/// Radii the player can pick when building or upgrade to later.
pub const RADII: [f32; 4] = [10.0, 20.0, 40.0, 80.0];
/// Lateral acceleration allowed on a curve: speed limit = sqrt(A_LAT * radius).
const A_LAT: f32 = 4.0;
pub const TRACK_HALF: f32 = 1.2;
pub const RAIL_COST_PER_LEN: f64 = 2.0;
pub const LEVEL_COST: f64 = 15.0;
pub const ROAD_BRIDGE_COST: f64 = 90.0;
pub const UNDERPASS_COST: f64 = 110.0;
pub const WATER_BRIDGE_BASE: f64 = 60.0;
pub const WATER_BRIDGE_PER_LEN: f64 = 5.0;
/// Rail speed cap over a level crossing.
pub const LEVEL_VCAP: f64 = 6.0;
pub const MAX_WATER_SPAN: f32 = 30.0;
pub const ED_EMPTY: f64 = 6.0;
pub const ED_PER_POP: f64 = 0.4;
const ED_STANDING_ZONE: f64 = 0.002;
const ED_STANDING_PER_POP: f64 = 0.0002;
const ED_STANDING_MAX: f64 = 0.3;
const STEER_DEAD: f32 = 2.0;
const OFFSET_STEP: f32 = 4.0;
const OFFSET_STEPS: u32 = 16;
const SAMPLE: f32 = 1.0;
const ARC_STEP: f32 = 5.0 * std::f32::consts::PI / 180.0;

/// Speed limit (world units / s) on a bend of this radius.
pub fn curve_speed(radius: f32) -> f64 {
    (A_LAT * radius.max(0.1)).sqrt() as f64
}

/// Whether a rail may cross this road at grade: local and arterial streets only (never a motorway, R24).
pub fn level_allowed(class: RoadClass) -> bool {
    matches!(class, RoadClass::Local | RoadClass::Arterial)
}

/// Price of a crossing of `kind` over `span` world units of water (span ignored for roads).
pub fn crossing_cost(kind: CrossingKind, water_span: Option<f32>) -> f64 {
    match (kind, water_span) {
        (_, Some(s)) => WATER_BRIDGE_BASE + WATER_BRIDGE_PER_LEN * s as f64,
        (CrossingKind::Level, None) => LEVEL_COST,
        (CrossingKind::Bridge, None) => ROAD_BRIDGE_COST,
        (CrossingKind::Underpass, None) => UNDERPASS_COST,
    }
}

/// Cost to change a road crossing later: only level -> bridge/underpass (pay the difference).
pub fn crossing_upgrade_cost(from: CrossingKind, to: CrossingKind) -> Option<f64> {
    (from == CrossingKind::Level && to != CrossingKind::Level).then(|| crossing_cost(to, None) - LEVEL_COST)
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Demolition {
    pub zones: Vec<usize>,
    pub cost: f64,
    /// Standing lost (0..=0.3).
    pub standing: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cost {
    pub track: f64,
    pub crossings: f64,
    pub eminent: f64,
    pub total: f64,
}

/// What the placer needs to know about the world.
pub struct Ctx<'a> {
    pub plan: &'a TownPlan,
    pub built: &'a [bool],
    /// Current population per zone (density drives compensation).
    pub pop: &'a [f32],
    /// Zones already demolished.
    pub gone: &'a [bool],
    pub land: &'a dyn Fn(Vec2) -> bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub a: Vec2,
    pub b: Vec2,
    pub waypoint: Vec2,
    pub pts: Vec<Vec2>,
    pub len: f64,
    /// Smallest bend radius actually used (INFINITY = straight).
    pub radius: f32,
    pub vcap: f64,
    pub crossings: Vec<Crossing>,
    pub crossing_costs: Vec<f64>,
    pub demolition: Demolition,
    pub cost: Cost,
    /// Signed sideways offset of the waypoint from the straight line (+ = left of a->b).
    pub offset: f32,
}

fn rect_dist(p: Vec2, centre: Vec2, angle: f32, half: Vec2) -> f32 {
    let (s, c) = angle.sin_cos();
    let d = p - centre;
    let local = Vec2::new(d.x * c + d.y * s, -d.x * s + d.y * c);
    let q = local.abs() - half;
    q.max(Vec2::ZERO).length() + q.max_element().min(0.0)
}

fn resample(path: &[Vec2], step: f32) -> Vec<Vec2> {
    let mut out = vec![];
    for w in path.windows(2) {
        let n = ((w[0].distance(w[1]) / step).ceil() as usize).max(1);
        out.extend((0..n).map(|i| w[0].lerp(w[1], i as f32 / n as f32)));
    }
    out.extend(path.last());
    out
}

/// Zones whose building is touched by a strip of half-width `half_width` around `strip`, with the compensation owed
/// (empty plots are cheap, dense ones expensive) and the standing lost.
pub fn eminent_domain(plan: &TownPlan, pop: &[f32], gone: &[bool], strip: &[Vec2], half_width: f32) -> Demolition {
    let samples = resample(strip, SAMPLE);
    let mut d = Demolition::default();
    for (z, p) in plan.parcels.iter().enumerate() {
        if gone.get(z).copied().unwrap_or(false) {
            continue;
        }
        let reach = p.size.length() * 0.5 + half_width + 0.5;
        if !samples.iter().any(|s| s.distance(p.center) <= reach && rect_dist(*s, p.center, p.angle, p.size * 0.5) < half_width) {
            continue;
        }
        let pop = pop.get(z).copied().unwrap_or(0.0) as f64;
        d.zones.push(z);
        d.cost += ED_EMPTY + ED_PER_POP * pop;
        d.standing += ED_STANDING_ZONE + ED_STANDING_PER_POP * pop;
    }
    d.standing = d.standing.min(ED_STANDING_MAX);
    d
}

/// a -> (arc around waypoint w, radius r) -> b as a polyline; also the radius actually used (shrunk to fit).
pub fn arc_path(a: Vec2, w: Vec2, b: Vec2, r: f32) -> (Vec<Vec2>, f32) {
    let (d1, d2) = ((w - a).length(), (b - w).length());
    if d1 < 1e-3 || d2 < 1e-3 {
        return (vec![a, b], f32::INFINITY);
    }
    let (u1, u2) = ((w - a) / d1, (b - w) / d2);
    let theta = u1.dot(u2).clamp(-1.0, 1.0).acos();
    if theta < 0.02 {
        return (vec![a, b], f32::INFINITY);
    }
    let tan_half = (theta / 2.0).tan();
    let r_eff = r.min(0.5 * d1.min(d2) / tan_half);
    let t = r_eff * tan_half;
    let (t1, t2) = (w - u1 * t, w + u2 * t);
    let sign = if u1.perp_dot(u2) >= 0.0 { 1.0 } else { -1.0 };
    let centre = t1 + Vec2::new(-u1.y, u1.x) * sign * r_eff;
    let a0 = (t1 - centre).to_angle();
    let steps = (theta / ARC_STEP).ceil().max(1.0) as usize;
    let mut pts = vec![a, t1];
    pts.extend((1..steps).map(|i| centre + Vec2::from_angle(a0 + sign * theta * i as f32 / steps as f32) * r_eff));
    pts.extend([t2, b]);
    (pts, r_eff)
}

fn seg_hit(p: Vec2, q: Vec2, r: Vec2, s: Vec2) -> Option<f32> {
    let (d1, d2) = (q - p, s - r);
    let den = d1.perp_dot(d2);
    if den.abs() < 1e-6 {
        return None;
    }
    let (t, u) = ((r - p).perp_dot(d2) / den, (r - p).perp_dot(d1) / den);
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some(t)
}

/// Place a track a -> b bending around `waypoint` with arc radius `radius`, and price it. None if it crosses too
/// much water for one bridge.
pub fn place_via(ctx: &Ctx, a: Vec2, b: Vec2, waypoint: Vec2, radius: f32) -> Option<Placement> {
    let (pts, r_eff) = arc_path(a, waypoint, b, radius);
    let mut cum = vec![0.0f32];
    for w in pts.windows(2) {
        cum.push(cum.last().unwrap() + w[0].distance(w[1]));
    }
    let len = *cum.last().unwrap();
    // roads: every built street the track cuts, once per ~2 units of track
    let (lo, hi) = (pts.iter().fold(Vec2::MAX, |m, p| m.min(*p)) - Vec2::splat(1.0), pts.iter().fold(Vec2::MIN, |m, p| m.max(*p)) + Vec2::splat(1.0));
    let mut crossings: Vec<Crossing> = vec![];
    let mut costs = vec![];
    let mut hits: Vec<(f32, usize)> = vec![];
    for (e, ed) in ctx.plan.graph.edges.iter().enumerate().filter(|(e, _)| ctx.built.get(*e).copied().unwrap_or(false)) {
        let (p, q) = ctx.plan.graph.pos(e);
        if p.max(q).cmplt(lo).any() || p.min(q).cmpgt(hi).any() {
            continue;
        }
        for (i, w) in pts.windows(2).enumerate() {
            if let Some(t) = seg_hit(w[0], w[1], p, q) {
                hits.push((cum[i] + t * w[0].distance(w[1]), e));
            }
        }
        let _ = ed;
    }
    hits.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
    let mut last = f32::NEG_INFINITY;
    for (at, e) in hits {
        if at - last < 2.0 {
            continue;
        }
        last = at;
        let kind = if level_allowed(ctx.plan.graph.edges[e].class) { CrossingKind::Level } else { CrossingKind::Bridge };
        crossings.push(Crossing { kind, at: at as f64, road: e as u32 });
        costs.push(crossing_cost(kind, None));
    }
    // water: a bridge over each stretch of water, priced by span
    let samples = resample(&pts, SAMPLE);
    let mut run: Option<usize> = None;
    for i in 0..=samples.len() {
        let wet = samples.get(i).is_some_and(|p| !(ctx.land)(*p));
        match (wet, run) {
            (true, None) => run = Some(i),
            (false, Some(s)) => {
                run = None;
                let span = (i - s) as f32 * SAMPLE;
                if span > MAX_WATER_SPAN {
                    return None;
                }
                crossings.push(Crossing { kind: CrossingKind::Bridge, at: ((s + i) as f32 * 0.5 * SAMPLE) as f64, road: u32::MAX });
                costs.push(crossing_cost(CrossingKind::Bridge, Some(span)));
            }
            _ => {}
        }
    }
    crossings.sort_by(|x, y| x.at.total_cmp(&y.at));
    let demolition = eminent_domain(ctx.plan, ctx.pop, ctx.gone, &pts, TRACK_HALF);
    let any_level = crossings.iter().any(|c| c.kind == CrossingKind::Level);
    let mut vcap = if r_eff.is_finite() { curve_speed(r_eff) } else { f64::INFINITY };
    if any_level {
        vcap = vcap.min(LEVEL_VCAP);
    }
    let track = len as f64 * RAIL_COST_PER_LEN;
    let crossing_total: f64 = costs.iter().sum();
    let chord = (b - a).normalize_or_zero();
    Some(Placement {
        a,
        b,
        waypoint,
        pts,
        len: len as f64,
        radius: r_eff,
        vcap,
        crossings,
        crossing_costs: costs,
        cost: Cost { track, crossings: crossing_total, eminent: demolition.cost, total: track + crossing_total + demolition.cost },
        demolition,
        offset: (waypoint - a).dot(Vec2::new(-chord.y, chord.x)),
    })
}

/// Auto-route a -> b. `steer` (the mouse) picks the side: drag to one side and the track goes round that side, the
/// long way if need be; without it the cheapest side wins. Obstacles are avoided where there is room; what is left
/// is priced into the returned placement.
pub fn place_track(ctx: &Ctx, a: Vec2, b: Vec2, steer: Option<Vec2>, radius: f32) -> Option<Placement> {
    let chord = b - a;
    let clen = chord.length();
    if clen < 1.0 {
        return None;
    }
    let nrm = Vec2::new(-chord.y, chord.x) / clen;
    let side = steer.map(|s| (s - a).dot(nrm)).filter(|o| o.abs() > STEER_DEAD).map(f32::signum);
    let mut cands: Vec<Vec2> = vec![];
    if side.is_none() {
        cands.push(a + chord * 0.5);
    }
    for t in [0.5, 0.35, 0.65] {
        for k in 1..=OFFSET_STEPS {
            let d = k as f32 * OFFSET_STEP;
            for s in match side {
                Some(s) => vec![s],
                None => vec![1.0, -1.0],
            } {
                cands.push(a + chord * t + nrm * s * d);
            }
        }
    }
    let mut best: Option<Placement> = None;
    for w in cands {
        let Some(p) = place_via(ctx, a, b, w, radius) else { continue };
        let better = best.as_ref().map_or(true, |c| {
            p.cost.total < c.cost.total - 1e-6 || ((p.cost.total - c.cost.total).abs() <= 1e-6 && p.len < c.len - 1e-6)
        });
        if better {
            best = Some(p);
        }
    }
    best
}

/// Re-lay an existing placement with a different radius (a later upgrade). The cost to charge is
/// `new.cost.eminent` (demolishing what the wider arc now sweeps) plus the change in track length.
pub fn with_radius(ctx: &Ctx, old: &Placement, radius: f32) -> Option<Placement> {
    place_via(ctx, old.a, old.b, old.waypoint, radius)
}

#[cfg(test)]
#[path = "placer_tests.rs"]
mod tests;
