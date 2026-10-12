//! Procedural town generator: terrain-cost arterials + tensor-field local streets + access-based parcels.
//! Pure and deterministic from (world seed, city region). Design and rationale: docs/arch/city-gn.md.
use bevy::math::Vec2;
use bevy::prelude::Color;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::f32::consts::{FRAC_PI_2, PI, TAU};
use world_gen::rng::{hash3, unit, SplitMix64};
use world_gen::{height, CityRegion, SEA_LEVEL};

/// Edges within this graph distance of the core are built at game start (the village).
pub const SEED_ORDER: u32 = 24;
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

fn cross(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

/// p + t*d vs segment a..b: returns (t, u) when they cross.
fn seg_hit(p: Vec2, d: Vec2, a: Vec2, b: Vec2) -> Option<(f32, f32)> {
    let s = b - a;
    let den = cross(d, s);
    if den.abs() < 1e-6 {
        return None;
    }
    let t = cross(a - p, s) / den;
    let u = cross(a - p, d) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some((t, u))
}

fn vnoise(seed: u64, salt: u64, x: f32, z: f32) -> f32 {
    let (fx, fz) = (x.floor(), z.floor());
    let (tx, tz) = (x - fx, z - fz);
    let sm = |t: f32| t * t * (3.0 - 2.0 * t);
    let l = |dx: i64, dz: i64| unit(hash3(seed, salt, fx as i64 + dx, fz as i64 + dz)) as f32;
    let top = l(0, 0) + (l(1, 0) - l(0, 0)) * sm(tx);
    let bot = l(0, 1) + (l(1, 1) - l(0, 1)) * sm(tx);
    top + (bot - top) * sm(tz)
}

/// Nearest water within ~16 units: (distance, direction to it). None when none or clearly inland.
fn water_near(seed: u64, p: Vec2) -> Option<(f32, Vec2)> {
    if height(seed, p.x as f64, p.y as f64) > SEA_LEVEL + 0.16 {
        return None;
    }
    for r in [3.0f32, 6.0, 9.0, 12.0, 16.0] {
        for k in 0..8 {
            let a = k as f32 * TAU / 8.0;
            let d = Vec2::new(a.cos(), a.sin());
            if !is_land(seed, p + d * r) {
                return Some((r, d));
            }
        }
    }
    None
}

/// Terrain-cost grid for arterial routing. cost = INFINITY: impassable.
struct Grid {
    n: usize,
    origin: Vec2,
    cost: Vec<f32>,
}

impl Grid {
    fn build(seed: u64, core: Vec2, half: f32) -> Self {
        let n = 2 * (half / CELL).floor() as usize + 1;
        let origin = core - Vec2::splat((n / 2) as f32 * CELL);
        let at = |i: usize, j: usize| origin + Vec2::new(i as f32, j as f32) * CELL;
        let h: Vec<f32> = (0..n * n).map(|k| height(seed, at(k % n, k / n).x as f64, at(k % n, k / n).y as f64)).collect();
        let water = |i: i64, j: i64| -> bool {
            if i < 0 || j < 0 || i >= n as i64 || j >= n as i64 {
                return true;
            }
            h[j as usize * n + i as usize] < SEA_LEVEL + 0.01
        };
        let mut cost = vec![f32::INFINITY; n * n];
        for j in 0..n {
            for i in 0..n {
                let k = j * n + i;
                if water(i as i64, j as i64) {
                    // narrowest crossing over the four axes, in world units
                    let mut best = f32::INFINITY;
                    for (dx, dz) in [(1i64, 0i64), (0, 1), (1, 1), (1, -1)] {
                        let run = |s: i64| (1..=6).find(|&m| !water(i as i64 + s * m * dx, j as i64 + s * m * dz));
                        if let (Some(a), Some(b)) = (run(1), run(-1)) {
                            let step = CELL * ((dx * dx + dz * dz) as f32).sqrt();
                            best = best.min((a + b - 1) as f32 * step);
                        }
                    }
                    if best <= MAX_BRIDGE {
                        cost[k] = 9.0;
                    }
                } else {
                    let mut slope = 0.0f32;
                    for (dx, dz) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
                        let (x, z) = (i as i64 + dx, j as i64 + dz);
                        if x >= 0 && z >= 0 && x < n as i64 && z < n as i64 {
                            slope = slope.max((h[k] - h[z as usize * n + x as usize]).abs());
                        }
                    }
                    cost[k] = 1.0 + 25.0 * slope + 2.5 * (h[k] - SEA_LEVEL).max(0.0);
                }
            }
        }
        Self { n, origin, cost }
    }
    fn world(&self, k: usize) -> Vec2 {
        self.origin + Vec2::new((k % self.n) as f32, (k / self.n) as f32) * CELL
    }
    fn cell(&self, p: Vec2) -> usize {
        let c = ((p - self.origin) / CELL).round();
        (c.y.clamp(0.0, self.n as f32 - 1.0) as usize) * self.n + c.x.clamp(0.0, self.n as f32 - 1.0) as usize
    }
    /// Least-cost path (cells, src first). `avoid` adds a penalty near that point (bypass roads skirt the core).
    fn path(&self, src: usize, dst: usize, avoid: Option<(Vec2, f32)>, line: Option<(Vec2, Vec2)>) -> Option<Vec<usize>> {
        let n = self.n;
        let mut d = vec![f64::INFINITY; n * n];
        let mut par = vec![usize::MAX; n * n];
        let mut h = BinaryHeap::new();
        d[src] = 0.0;
        h.push(Reverse((0f64.to_bits(), src)));
        while let Some(Reverse((bits, u))) = h.pop() {
            let du = f64::from_bits(bits);
            if du > d[u] {
                continue;
            }
            if u == dst {
                break;
            }
            let (ux, uz) = ((u % n) as i64, (u / n) as i64);
            for dz in -1i64..=1 {
                for dx in -1i64..=1 {
                    if dx == 0 && dz == 0 {
                        continue;
                    }
                    let (x, z) = (ux + dx, uz + dz);
                    if x < 0 || z < 0 || x >= n as i64 || z >= n as i64 {
                        continue;
                    }
                    let v = z as usize * n + x as usize;
                    if !self.cost[v].is_finite() {
                        continue;
                    }
                    let mut c = 0.5 * (self.cost[u] + self.cost[v]);
                    if let Some((p, r)) = avoid {
                        c += 4.0 * (1.0 - self.world(v).distance(p) / r).max(0.0);
                    }
                    if let Some((a, b)) = line {
                        // arterials prefer to run straight: penalise drifting off the core-to-gateway line
                        c += 0.3 * (seg_dist(self.world(v), a, b) / 6.0).powi(2);
                    }
                    let step = if dx != 0 && dz != 0 { 1.4142 } else { 1.0 };
                    let nd = du + (c * step) as f64;
                    if nd < d[v] {
                        d[v] = nd;
                        par[v] = u;
                        h.push(Reverse((nd.to_bits(), v)));
                    }
                }
            }
        }
        if !d[dst].is_finite() {
            return None;
        }
        let mut path = vec![dst];
        while *path.last().unwrap() != src {
            path.push(par[*path.last().unwrap()]);
        }
        path.reverse();
        Some(path)
    }
}

fn dp_keep(pts: &[Vec2], lo: usize, hi: usize, eps: f32, keep: &mut [bool]) {
    let (mut mi, mut md) = (0, 0.0f32);
    for i in lo + 1..hi {
        let d = seg_dist(pts[i], pts[lo], pts[hi]);
        if d > md {
            (mi, md) = (i, d);
        }
    }
    if md > eps {
        keep[mi] = true;
        dp_keep(pts, lo, mi, eps, keep);
        dp_keep(pts, mi, hi, eps, keep);
    }
}

fn simplify(pts: &[Vec2], eps: f32) -> Vec<Vec2> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    (keep[0], keep[pts.len() - 1]) = (true, true);
    dp_keep(pts, 0, pts.len() - 1, eps, &mut keep);
    pts.iter().zip(&keep).filter(|(_, k)| **k).map(|(p, _)| *p).collect()
}

enum Conn {
    Free(u32),
    Hit(u32),
    Blocked,
}

enum Target {
    Node(u32),
    Split(usize, Vec2),
    New(Vec2),
}

struct Gen<'a> {
    seed: u64,
    town: u32,
    core: Vec2,
    half: f32,
    g: &'a mut RoadGraph,
    e0: usize,
    n0: usize,
    rng: SplitMix64,
    a0: f32,
}

impl Gen<'_> {
    fn f(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * unit(self.rng.next_u64()) as f32
    }
    fn pos(&self, n: u32) -> Vec2 {
        self.g.nodes[n as usize]
    }
    fn inside(&self, p: Vec2) -> bool {
        (p - self.core).abs().max_element() <= self.half
    }
    fn dist_core(&self, p: Vec2) -> f32 {
        p.distance(self.core)
    }

    fn edge_exists(&self, a: u32, b: u32) -> bool {
        self.g.edges[self.e0..].iter().any(|e| (e.a, e.b) == (a, b) || (e.a, e.b) == (b, a))
    }

    /// Try to add one edge from `from` toward `q`, handling water, crossings (split / snap) and near-snaps.
    fn connect(&mut self, from: u32, q: Vec2, class: RoadClass, allow_bridge: bool) -> Conn {
        let p = self.pos(from);
        if !self.inside(q) || p.distance(q) < 1.5 {
            return Conn::Blocked;
        }
        let d = q - p;
        // earliest crossing with an existing edge that does not touch `from`
        let mut hit: Option<(f32, usize)> = None;
        for e in self.e0..self.g.edges.len() {
            let ed = &self.g.edges[e];
            if ed.a == from || ed.b == from {
                continue;
            }
            let (a, b) = self.g.pos(e);
            if let Some((t, _)) = seg_hit(p, d, a, b) {
                if t > 0.04 && hit.map_or(true, |(bt, _)| t < bt) {
                    hit = Some((t, e));
                }
            }
        }
        let target = if let Some((t, e)) = hit {
            if self.g.edges[e].bridge {
                return Conn::Blocked;
            }
            let x = p + d * t;
            let (a, b) = (self.g.edges[e].a, self.g.edges[e].b);
            if self.pos(a).distance(x) < SNAP_NODE {
                Target::Node(a)
            } else if self.pos(b).distance(x) < SNAP_NODE {
                Target::Node(b)
            } else {
                Target::Split(e, x)
            }
        } else {
            let near = (self.n0..self.g.nodes.len())
                .filter(|&n| n as u32 != from && self.pos(n as u32).distance(q) < SNAP_NODE)
                .min_by(|&a, &b| self.pos(a as u32).distance(q).total_cmp(&self.pos(b as u32).distance(q)));
            if let Some(n) = near {
                Target::Node(n as u32)
            } else {
                let near_e = (self.e0..self.g.edges.len())
                    .filter(|&e| !self.g.edges[e].bridge && self.g.edges[e].a != from && self.g.edges[e].b != from)
                    .map(|e| {
                        let (a, b) = self.g.pos(e);
                        (e, seg_closest(q, a, b).0)
                    })
                    .filter(|(_, x)| x.distance(q) < SNAP_EDGE)
                    .min_by(|a, b| a.1.distance(q).total_cmp(&b.1.distance(q)));
                match near_e {
                    Some((e, x)) => Target::Split(e, x),
                    None => Target::New(q),
                }
            }
        };
        let tp = match &target {
            Target::Node(n) => self.pos(*n),
            Target::Split(_, x) | Target::New(x) => *x,
        };
        if tp.distance(p) < 1.5 {
            return Conn::Blocked;
        }
        if let Target::Node(n) = target {
            if self.edge_exists(from, n) {
                return Conn::Blocked;
            }
        }
        // water: locals never cross; arterials only bridge narrow water between land ends
        let wr = water_run(self.seed, p, tp);
        if wr > 0.0 && (!allow_bridge || wr > MAX_BRIDGE_ABS || !is_land(self.seed, tp)) {
            return Conn::Blocked;
        }
        // no slivers against edges already leaving `from`
        let dir = (tp - p).normalize_or_zero();
        for e in self.e0..self.g.edges.len() {
            let ed = &self.g.edges[e];
            let o = if ed.a == from { ed.b } else if ed.b == from { ed.a } else { continue };
            if (self.pos(o) - p).normalize_or_zero().dot(dir) > 0.955 {
                return Conn::Blocked;
            }
        }
        let (node, fresh) = match target {
            Target::Node(n) => (n, false),
            Target::New(x) => {
                self.g.nodes.push(x);
                (self.g.nodes.len() as u32 - 1, true)
            }
            Target::Split(e, x) => {
                self.g.nodes.push(x);
                let n = self.g.nodes.len() as u32 - 1;
                let old = self.g.edges[e].clone();
                let (a, b) = (self.pos(old.a), self.pos(old.b));
                self.g.edges[e].b = n;
                self.g.edges[e].len = a.distance(x);
                self.g.edges.push(REdge { a: n, b: old.b, len: x.distance(b), ..old });
                (n, false)
            }
        };
        self.g.edges.push(REdge { a: from, b: node, class, bridge: wr > 0.0, len: p.distance(tp), town: self.town, order: 0 });
        if fresh { Conn::Free(node) } else { Conn::Hit(node) }
    }

    /// Lay an arterial polyline starting at `start`; returns the last node reached.
    fn lay(&mut self, start: u32, pts: &[Vec2]) -> Option<u32> {
        let mut v: Vec<Vec2> = vec![self.pos(start)];
        v.extend(pts.iter().skip(1).copied().filter(|p| is_land(self.seed, *p)));
        let ok = |v: &[Vec2], seed: u64| {
            v.windows(2).all(|w| {
                let wr = water_run(seed, w[0], w[1]);
                wr == 0.0 || (wr <= MAX_BRIDGE_ABS && is_land(seed, w[1]))
            })
        };
        let s = simplify(&v, 2.5);
        let v = if ok(&s, self.seed) { s } else if ok(&v, self.seed) { v } else { return None };
        let mut cur = start;
        for w in v.windows(2) {
            let bridge = water_run(self.seed, w[0], w[1]) > 0.0;
            let n = if bridge { 1 } else { (w[0].distance(w[1]) / ARTERIAL_STEP).ceil().max(1.0) as usize };
            for k in 1..=n {
                match self.connect(cur, w[0].lerp(w[1], k as f32 / n as f32), RoadClass::Arterial, true) {
                    Conn::Free(x) | Conn::Hit(x) => cur = x,
                    Conn::Blocked => return Some(cur),
                }
            }
        }
        Some(cur)
    }

    /// Tensor-field direction (unit, sign arbitrary): grid at the core, curvilinear outward, shoreline near water.
    fn field(&self, p: Vec2) -> Vec2 {
        let d = p - self.core;
        let wg = (-(d.length() / 40.0).powi(2)).exp();
        let tang = d.y.atan2(d.x) + FRAC_PI_2;
        let c = tang + (vnoise(self.seed, 0x51, p.x / 34.0, p.y / 34.0) - 0.5) * 2.6;
        let dbl = |a: f32| Vec2::new((2.0 * a).cos(), (2.0 * a).sin());
        let mut v = dbl(self.a0) * wg + dbl(c) * (1.0 - wg);
        if let Some((dw, dir)) = water_near(self.seed, p) {
            let wc = (1.0 - dw / 18.0).max(0.0).powi(2);
            v = v * (1.0 - wc) + dbl(dir.y.atan2(dir.x) + FRAC_PI_2) * wc;
        }
        let a = v.y.atan2(v.x) / 2.0;
        Vec2::new(a.cos(), a.sin())
    }

    fn spacing(&self, dist: f32) -> f32 {
        16.0 + 12.0 * (dist / self.half).clamp(0.0, 1.0)
    }

    /// Trace one local street (hyperstreamline) from `from`; returns nodes worth branching from.
    fn trace(&mut self, from: u32, dir: Vec2, depth: u8) -> Vec<(u32, Vec2)> {
        let mut out = vec![];
        let mut cur = from;
        let mut heading = dir.normalize_or_zero();
        let frac = (self.dist_core(self.pos(from)) / self.half).clamp(0.0, 1.0);
        let max_len = (44.0 - 26.0 * frac) * if depth >= 2 { 0.9 } else { 1.0 };
        let (mut walked, mut last_seed) = (0.0f32, 0.0f32);
        while self.g.edges.len() - self.e0 < MAX_TOWN_EDGES {
            let p = self.pos(cur);
            let mut fd = self.field(p);
            if fd.dot(heading) < 0.0 {
                fd = -fd;
            }
            let stiff = if depth >= 2 { 0.8 } else { 0.3 }; // cross streets hold their line, ring streets follow the field
            heading = (heading * stiff + fd * (1.0 - stiff)).normalize_or(heading);
            let q = p + heading * (STREET_STEP + self.f(-0.6, 0.6));
            match self.connect(cur, q, RoadClass::Local, false) {
                Conn::Free(n) => {
                    walked += p.distance(self.pos(n));
                    cur = n;
                    let sp = self.spacing(self.dist_core(self.pos(n))) * 0.7;
                    if walked - last_seed >= sp {
                        last_seed = walked;
                        out.push((n, heading));
                    }
                    let fr = (self.dist_core(self.pos(n)) / self.half).clamp(0.0, 1.0);
                    if walked >= max_len || self.f(0.0, 1.0) < 0.01 + 0.05 * fr {
                        break; // dead end: a cul-de-sac in the suburbs
                    }
                }
                _ => break,
            }
        }
        out
    }

    /// Branch streets off `seeds` on both sides, perpendicular to `axis` unless the field is already clear of it.
    fn branch(&mut self, seeds: &[(u32, Vec2)], depth: u8) -> Vec<(u32, Vec2)> {
        let mut next = vec![];
        for &(n, axis) in seeds {
            for s in [1.0f32, -1.0] {
                let perp = Vec2::new(-axis.y, axis.x) * s;
                let mut fd = self.field(self.pos(n));
                if fd.dot(perp) < 0.0 {
                    fd = -fd;
                }
                let dir = if cross(fd, axis).abs() < 0.77 { perp } else { fd };
                next.extend(self.trace(n, dir, depth));
            }
        }
        next
    }
}

fn graph_dist(g: &RoadGraph, e0: usize, n0: usize, src: u32) -> Vec<f32> {
    let mut adj = vec![vec![]; g.nodes.len()];
    for e in &g.edges[e0..] {
        adj[e.a as usize].push((e.b, e.len));
        adj[e.b as usize].push((e.a, e.len));
    }
    let mut d = vec![f32::INFINITY; g.nodes.len()];
    let mut h = BinaryHeap::new();
    d[src as usize] = 0.0;
    h.push(Reverse((0f32.to_bits(), src)));
    while let Some(Reverse((bits, u))) = h.pop() {
        let du = f32::from_bits(bits);
        if du > d[u as usize] {
            continue;
        }
        for &(v, w) in &adj[u as usize] {
            if du + w < d[v as usize] {
                d[v as usize] = du + w;
                h.push(Reverse(((du + w).to_bits(), v)));
            }
        }
    }
    let _ = n0;
    d
}

fn capacity_at(dist: f32, reach: f32) -> f64 {
    PEAK_CAPACITY * (1.0 - dist / (reach + 30.0)).max(0.15).powf(1.3) as f64
}

/// Generate one town into `plan` (graph appended, parcels appended in growth order).
fn gen_town(seed: u64, region: &CityRegion, plan: &mut TownPlan) {
    let town = region.id.0;
    let (ox, oz) = region.center.origin_cell();
    let core = Vec2::new(ox as f32 + 16.0, oz as f32 + 16.0);
    let half = (((region.radius as f32 + 0.5) * 32.0) - 6.0).min(80.0);
    let (e0, n0) = (plan.graph.edges.len(), plan.graph.nodes.len());
    let mut gn = Gen {
        seed,
        town,
        core,
        half,
        g: &mut plan.graph,
        e0,
        n0,
        rng: SplitMix64::new(hash3(seed, 0x70_0A, town as i64, 7)),
        a0: 0.0,
    };
    gn.g.nodes.push(core);
    let core_node = n0 as u32;
    let grid = Grid::build(seed, core, half);

    // seed: a village at a narrow crossing if water is close, otherwise a crossroads
    let mut crossing: Option<(f32, f32, Vec2)> = None; // (width, angle, far point)
    for k in 0..24 {
        let ang = k as f32 * TAU / 24.0;
        let dir = Vec2::new(ang.cos(), ang.sin());
        let mut r = 2.0;
        let mut first_water = None;
        while r < 64.0 {
            let w = !is_land(seed, core + dir * r);
            match (first_water, w) {
                (None, true) => first_water = Some(r),
                (Some(w0), false) => {
                    if w0 <= 36.0 && r - w0 <= MAX_BRIDGE && crossing.map_or(true, |c| r - w0 < c.0) {
                        crossing = Some((r - w0, ang, core + dir * (r + 10.0)));
                    }
                    break;
                }
                _ => {}
            }
            r += 2.0;
        }
    }
    let kind = if crossing.is_some() { SeedKind::RiverCrossing } else { SeedKind::Crossroads };
    gn.a0 = match crossing {
        Some((_, a, _)) => a,
        None => gn.f(0.0, PI),
    };

    // gateways: four, 90 degrees apart, each a least-cost arterial from the core
    let mut ends: Vec<Option<u32>> = vec![];
    let mut targets: Vec<Option<Vec2>> = vec![];
    for k in 0..4 {
        let ang = gn.a0 + k as f32 * FRAC_PI_2 + gn.f(-0.15, 0.15);
        let dir = Vec2::new(ang.cos(), ang.sin());
        let mut dist = gn.f(56.0, half - 4.0);
        let mut t = core + dir * dist;
        if k == 0 {
            if let Some((_, _, far)) = crossing {
                if gn.inside(far) && is_land(seed, far) {
                    t = far;
                }
            }
        }
        while !is_land(seed, t) && dist > 30.0 {
            dist -= 5.0;
            t = core + dir * dist;
        }
        targets.push(is_land(seed, t).then_some(t));
    }
    let c0 = grid.cell(core);
    for t in &targets {
        let end = t.and_then(|t| grid.path(c0, grid.cell(t), None, Some((core, t)))).and_then(|cells| {
            let pts: Vec<Vec2> = cells.iter().map(|&c| grid.world(c)).collect();
            let mut pts = pts;
            pts[0] = core;
            gn.lay(core_node, &pts)
        });
        ends.push(end);
    }
    // bypasses between neighbouring gateways
    for k in 0..4 {
        let j = (k + 1) % 4;
        let (Some(ek), Some(tj), Some(tk)) = (ends[k], targets[j], targets[k]) else { continue };
        if gn.f(0.0, 1.0) > 0.4 || tk.distance(tj) > 1.6 * half {
            continue;
        }
        if let Some(cells) = grid.path(grid.cell(tk), grid.cell(tj), Some((core, half * 0.6)), None) {
            let mut pts: Vec<Vec2> = cells.iter().map(|&c| grid.world(c)).collect();
            pts[0] = gn.pos(ek);
            gn.lay(ek, &pts);
        }
    }

    // local streets: seed on arterial nodes (both sides), then two more generations of branches
    let mut art_nodes: Vec<u32> = (n0 as u32 + 1..gn.g.nodes.len() as u32).collect();
    art_nodes.sort_by(|&a, &b| gn.dist_core(gn.pos(a)).total_cmp(&gn.dist_core(gn.pos(b))).then(a.cmp(&b)));
    let mut seeds: Vec<(u32, Vec2)> = vec![];
    for n in art_nodes {
        let p = gn.pos(n);
        let sp = gn.spacing(gn.dist_core(p));
        if seeds.iter().any(|&(s, _)| gn.pos(s).distance(p) < sp) {
            continue;
        }
        let Some(ed) = gn.g.edges[e0..].iter().find(|e| e.class == RoadClass::Arterial && (e.a == n || e.b == n)) else { continue };
        let o = if ed.a == n { ed.b } else { ed.a };
        let axis = (gn.pos(o) - p).normalize_or_zero();
        if axis != Vec2::ZERO {
            seeds.push((n, axis));
        }
    }
    let gen1 = gn.branch(&seeds, 1);
    let gen2 = gn.branch(&gen1, 2);
    let core_r = half * 0.45;
    let gen3: Vec<(u32, Vec2)> = gen2.into_iter().filter(|&(n, _)| gn.dist_core(gn.pos(n)) < core_r).collect();
    gn.branch(&gen3, 3);

    // build order = graph distance from the core
    let dist = graph_dist(gn.g, e0, n0, core_node);
    for e in &mut gn.g.edges[e0..] {
        let d = dist[e.a as usize].max(dist[e.b as usize]);
        e.order = if d.is_finite() { d.round() as u32 } else { 9999 };
    }

    // parcels both sides of every edge, clear of other roads and water, thinning with distance
    let g = &*gn.g;
    let deg: Vec<usize> = {
        let mut d = vec![0; g.nodes.len()];
        for e in &g.edges[e0..] {
            d[e.a as usize] += 1;
            d[e.b as usize] += 1;
        }
        d
    };
    let mut rng = SplitMix64::new(hash3(seed, 0x70_0B, town as i64, 9));
    let mut f = |lo: f32, hi: f32| lo + (hi - lo) * unit(rng.next_u64()) as f32;
    let mut found: Vec<Parcel> = vec![];
    for e in e0..g.edges.len() {
        let (a, b) = g.pos(e);
        let len = a.distance(b);
        if len < 4.0 {
            continue;
        }
        let dir = (b - a) / len;
        let ang = dir.y.atan2(dir.x);
        let nrm = Vec2::new(-dir.y, dir.x);
        let m = (len / PARCEL_SPACING).round().max(1.0) as usize;
        for i in 0..m {
            let along = a + dir * (len * (i as f32 + 0.5) / m as f32);
            for s in [1.0f32, -1.0] {
                let sz = Vec2::new(f(3.8, 5.8), f(2.8, 4.0));
                let c = along + nrm * s * (ROAD_CLEAR + sz.y * 0.5 + 0.4);
                let dc = c.distance(core);
                let keep = (1.05 - dc / (half * 1.15)).clamp(0.12, 1.0) * if g.edges[e].class == RoadClass::Arterial { 1.15 } else { 0.9 };
                if f(0.0, 1.0) > keep || !(c - core).abs().max_element().le(&(half - 4.0)) || !footprint_on_land(seed, c) {
                    continue;
                }
                let clear = g.edges[e0..]
                    .iter()
                    .enumerate()
                    .filter(|(k, _)| e0 + k != e)
                    .all(|(k, _)| {
                        let (p, q) = g.pos(e0 + k);
                        seg_dist(c, p, q) >= sz.y * 0.5 + 1.8
                    });
                if !clear || found.iter().any(|p| p.center.distance(c) < PARCEL_SPACING - 0.5) {
                    continue;
                }
                // land use follows access
                let arterial = g.edges[e].class == RoadClass::Arterial;
                let near_junction = [g.edges[e].a, g.edges[e].b]
                    .iter()
                    .any(|&n| deg[n as usize] >= 3 && g.nodes[n as usize].distance(c) < 9.0);
                let dw = water_near(seed, c).map_or(f32::INFINITY, |w| w.0);
                let r = f(0.0, 1.0);
                let kind = if dc < 14.0 || (arterial && dc < half * 0.5 && r < 0.55) || (near_junction && dc < half * 0.7 && r < 0.6) {
                    ZoneKind::Commercial
                } else if (dw <= 14.0 && dc > 24.0 && r < 0.55) || (dc > half * 0.7 && arterial && r < 0.4) {
                    ZoneKind::Industrial
                } else {
                    ZoneKind::Residential
                };
                let capacity = capacity_at(dc, 78.0);
                let seeded = g.edges[e].order <= SEED_ORDER;
                let start = if dc < 10.0 { 0.3 } else if seeded && dc < 22.0 { 0.12 } else { 0.0 };
                found.push(Parcel { center: c, town, kind, access: e, angle: ang, size: sz, capacity, start_pop: capacity * start, core: false });
            }
        }
    }
    // growth order: nearest the core first; the core zone is the one at the village centre
    found.sort_by(|a, b| {
        g.edges[a.access].order.cmp(&g.edges[b.access].order).then(a.center.distance(core).total_cmp(&b.center.distance(core)))
    });
    if let Some(i) = (0..found.len()).min_by(|&a, &b| found[a].center.distance(core).total_cmp(&found[b].center.distance(core))) {
        found[i].core = true;
        found[i].kind = ZoneKind::Commercial;
        found[i].start_pop = found[i].capacity * 0.3;
    }
    plan.parcels.extend(found);
    plan.towns.push(TownMeta { town, core, core_node, kind });
}

/// Plan the given city regions (one town each). Parcel index == growth ZoneId.
pub fn plan_towns(seed: u64, regions: &[CityRegion]) -> TownPlan {
    let mut plan = TownPlan::default();
    for r in regions {
        gen_town(seed, r, &mut plan);
    }
    plan
}

#[cfg(test)]
#[path = "roadgen_tests.rs"]
mod tests;
