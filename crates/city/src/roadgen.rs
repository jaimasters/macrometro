//! Town plan types and terrain helpers. Towns are grown by organic.rs (one seed+growth algorithm).
use bevy::math::Vec2;
use bevy::prelude::Color;
use std::f32::consts::FRAC_PI_2;
use world_gen::rng::hash3;
use world_gen::{height, CityRegion, SEA_LEVEL};

/// Edges with order <= this are built at game start (the seed plus the preset's growth steps).
pub const SEED_ORDER: u32 = 0;
/// Narrowest-water rule: arterials may bridge water up to this wide (world units).
pub const MAX_BRIDGE: f32 = 16.0;
const MAX_BRIDGE_ABS: f32 = 22.0;
const SNAP_NODE: f32 = 3.2;
const SNAP_EDGE: f32 = 2.6;
const CELL: f32 = 4.0;
const ARTERIAL_STEP: f32 = 12.0;
const STREET_STEP: f32 = 5.0;
const MAX_TOWN_EDGES: usize = 560;
/// Peak zone capacity (residents) at a town core; falls off with distance.
pub const PEAK_CAPACITY: f64 = 2000.0;
const FOOTPRINT_R: f32 = 4.5;
const PARCEL_SPACING: f32 = 7.2;
const ROAD_CLEAR: f32 = 2.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoadClass {
    Local,
    Arterial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedKind {
    Crossroads,
    RiverCrossing,
}

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
    pub fn name(self) -> &'static str {
        match self {
            ZoneKind::Residential => "Residential",
            ZoneKind::Commercial => "Commercial",
            ZoneKind::Industrial => "Industrial",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct REdge {
    pub a: u32,
    pub b: u32,
    pub class: RoadClass,
    pub bridge: bool,
    pub len: f32,
    pub town: u32,
    /// Graph distance from the town core (build priority; <= SEED_ORDER starts built).
    pub order: u32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoadGraph {
    pub nodes: Vec<Vec2>,
    pub edges: Vec<REdge>,
}

impl RoadGraph {
    pub fn pos(&self, e: usize) -> (Vec2, Vec2) {
        (self.nodes[self.edges[e].a as usize], self.nodes[self.edges[e].b as usize])
    }
    /// Incident edge ids per node.
    pub fn incident(&self) -> Vec<Vec<usize>> {
        let mut v = vec![vec![]; self.nodes.len()];
        for (i, e) in self.edges.iter().enumerate() {
            v[e.a as usize].push(i);
            v[e.b as usize].push(i);
        }
        v
    }
}

/// A buildable parcel = one growth zone. `access` is the frontage edge: the zone may grow once it is built.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Parcel {
    pub center: Vec2,
    pub town: u32,
    pub kind: ZoneKind,
    pub access: usize,
    pub angle: f32,
    pub size: Vec2,
    pub capacity: f64,
    /// Population the zone starts with (0 = not yet built).
    pub start_pop: f64,
    /// The town's core zone (the centre of the village).
    pub core: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TownMeta {
    pub town: u32,
    pub core: Vec2,
    pub core_node: u32,
    pub kind: SeedKind,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TownPlan {
    pub graph: RoadGraph,
    pub parcels: Vec<Parcel>,
    pub towns: Vec<TownMeta>,
}

impl TownPlan {
    /// Road node a parcel connects to: the nearer end of its frontage edge.
    pub fn access_node(&self, z: usize) -> u32 {
        let (p, e) = (&self.parcels[z], &self.graph.edges[self.parcels[z].access]);
        let (a, b) = (self.graph.nodes[e.a as usize], self.graph.nodes[e.b as usize]);
        if a.distance(p.center) <= b.distance(p.center) { e.a } else { e.b }
    }
    /// Parcel ids per frontage edge.
    pub fn fronting(&self) -> Vec<Vec<usize>> {
        let mut v = vec![vec![]; self.graph.edges.len()];
        for (z, p) in self.parcels.iter().enumerate() {
            v[p.access].push(z);
        }
        v
    }
}

pub fn is_land(seed: u64, p: Vec2) -> bool {
    height(seed, p.x as f64, p.y as f64) >= SEA_LEVEL + 0.01
}

/// Longest contiguous water stretch (world units) on the straight line p..q.
pub fn water_run(seed: u64, p: Vec2, q: Vec2) -> f32 {
    let l = p.distance(q);
    let n = l.ceil().max(1.0) as usize;
    let (mut best, mut cur) = (0.0f32, 0.0f32);
    for i in 0..=n {
        if is_land(seed, p.lerp(q, i as f32 / n as f32)) {
            cur = 0.0;
        } else {
            cur += l / n as f32;
            best = best.max(cur);
        }
    }
    best
}

pub fn footprint_on_land(seed: u64, p: Vec2) -> bool {
    is_land(seed, p)
        && (0..8).all(|k| {
            let a = k as f32 * FRAC_PI_2 / 2.0;
            is_land(seed, p + Vec2::new(a.cos(), a.sin()) * FOOTPRINT_R)
        })
}

pub fn seg_closest(p: Vec2, a: Vec2, b: Vec2) -> (Vec2, f32) {
    let ab = b - a;
    let l2 = ab.length_squared();
    let t = if l2 > 0.0 { ((p - a).dot(ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (a + ab * t, t)
}

pub fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    p.distance(seg_closest(p, a, b).0)
}

/// Plan the given city regions (one town each, grown by organic's one growth algorithm). Parcel index == growth ZoneId.
pub fn plan_towns(seed: u64, regions: &[CityRegion]) -> TownPlan {
    let mut plan = TownPlan::default();
    for r in regions {
        let (ox, oz) = r.center.origin_cell();
        let core = Vec2::new(ox as f32 + 16.0, oz as f32 + 16.0);
        let t = crate::organic::plan_preset(seed, hash3(seed, 0x70_0A, r.id.0 as i64, 7), core, crate::organic::Preset::SmallTown);
        let (n0, e0) = (plan.graph.nodes.len() as u32, plan.graph.edges.len());
        plan.graph.nodes.extend(t.graph.nodes);
        plan.graph.edges.extend(t.graph.edges.into_iter().map(|e| REdge { a: e.a + n0, b: e.b + n0, town: r.id.0, ..e }));
        plan.parcels.extend(t.parcels.into_iter().map(|p| Parcel { town: r.id.0, access: p.access + e0, ..p }));
        plan.towns.extend(t.towns.into_iter().map(|m| TownMeta { town: r.id.0, core_node: m.core_node + n0, ..m }));
    }
    plan
}

#[cfg(test)]
#[path = "roadgen_tests.rs"]
mod tests;
