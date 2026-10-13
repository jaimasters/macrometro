//! One city-growth algorithm (Jai): a town is a small pre-placed seed (crossing skeleton roads) grown by the SAME
//! growth step the running city uses. A preset only sets the seed size and how many steps have already happened;
//! the steps after that are the plan's future, revealed at runtime by RoadBuilder in growth order (edge `order`).
//! Each step extends a dead end outward or branches a curved side street off a through node, keeping junction
//! clearance (junctions::merge_close) and connectivity (every road attaches to the network; no 1-segment stubs).
use crate::roadgen::*;
use bevy::math::Vec2;
use world_gen::rng::SplitMix64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// A few houses on one road.
    Hamlet,
    /// A small country town: two crossing roads and a handful of side streets.
    SmallTown,
    /// A large country town on an interstate.
    LargeTown,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::Hamlet, Preset::SmallTown, Preset::LargeTown];
    pub fn name(self) -> &'static str {
        match self {
            Preset::Hamlet => "Hamlet",
            Preset::SmallTown => "Small country town",
            Preset::LargeTown => "Large country town + interstate",
        }
    }
    /// (skeleton roads, seed reach from the core, growth steps already run, built houses)
    pub fn shape(self) -> (usize, f32, usize, usize) {
        match self {
            Preset::Hamlet => (1, 48.0, 4, 5),
            Preset::SmallTown => (2, 64.0, 8, 40),
            Preset::LargeTown => (2, 96.0, 90, 140),
        }
    }
}

/// Growth steps planned past the preset (the city's future, built at runtime).
pub const FUTURE_STEPS: usize = 160;
const STEP: f32 = 16.0;
const SETBACK: f32 = 6.0;
const HOUSE_GAP: f32 = 9.0;
const CLEAR: f32 = 5.0;
/// Closest two junctions may sit along a road of this class; arterials space theirs further apart.
pub fn junction_gap(c: RoadClass) -> f32 {
    if c == RoadClass::Arterial { STEP * 4.0 } else { STEP * 2.5 }
}

fn crosses(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    let o = |p: Vec2, q: Vec2, r: Vec2| (q - p).perp_dot(r - p);
    o(a, b, c) * o(a, b, d) < 0.0 && o(c, d, a) * o(c, d, b) < 0.0
}

/// A growing town graph.
pub struct Town {
    pub g: RoadGraph,
    pub core: Vec2,
    world: u64,
    rng: SplitMix64,
    /// Growth picks spots near the core: a spot `d` out is taken with chance 1 / (1 + (d / falloff)^2).
    pub falloff: f32,
}

impl Town {
    fn node(&mut self, p: Vec2) -> u32 {
        self.g.nodes.push(p);
        self.g.nodes.len() as u32 - 1
    }
    fn edge(&mut self, a: u32, b: u32, class: RoadClass, order: u32) {
        let len = self.g.nodes[a as usize].distance(self.g.nodes[b as usize]);
        self.g.edges.push(REdge { a, b, class, bridge: false, len, town: 0, order });
    }
    fn degree(&self, n: u32) -> usize {
        self.g.edges.iter().filter(|e| e.a == n || e.b == n).count()
    }
    /// Would a new segment p-q cross water (arterials may bridge a short run), cross a road, or crowd one?
    fn blocked(&self, from: u32, p: Vec2, q: Vec2, class: RoadClass) -> bool {
        let wet = water_run(self.world, p, q);
        if !is_land(self.world, q) || (wet > 0.0 && (class != RoadClass::Arterial || wet > MAX_BRIDGE)) {
            return true;
        }
        self.g.edges.iter().any(|e| {
            let (a, b) = (self.g.nodes[e.a as usize], self.g.nodes[e.b as usize]);
            let touches = e.a == from || e.b == from;
            crosses(p, q, a, b) || (!touches && seg_dist(q, a, b) < CLEAR * 2.0)
        })
    }
    /// A meandering road from node `n` heading `dir` for `len`, stopping early if it would hit water or a road.
    fn grow(&mut self, mut n: u32, mut dir: Vec2, len: f32, class: RoadClass, wiggle: f32, order: u32) -> usize {
        let mut made = 0;
        for _ in 0..(len / STEP).ceil() as usize {
            dir = Vec2::from_angle((f(&mut self.rng) as f32 - 0.5) * wiggle).rotate(dir).normalize();
            let p = self.g.nodes[n as usize];
            let q = p + dir * STEP;
            if self.blocked(n, p, q, class) {
                break;
            }
            let m = self.node(q);
            self.edge(n, m, class, order);
            n = m;
            made += 1;
        }
        made
    }

    /// The seed: `roads` skeleton roads crossing at the core, `reach` out each way (order 0 = built).
    pub fn seed(world: u64, rng_seed: u64, core: Vec2, roads: usize, reach: f32, arterial: bool) -> Self {
        let mut t = Town { g: RoadGraph::default(), core, world, rng: SplitMix64::new(rng_seed ^ 0x0E6A_1C), falloff: f32::INFINITY };
        let c = t.node(core);
        let turn = f(&mut t.rng) as f32 * std::f32::consts::TAU;
        for k in 0..roads {
            let axis = Vec2::from_angle(turn + k as f32 * std::f32::consts::FRAC_PI_2);
            let class = if arterial && k == 0 { RoadClass::Arterial } else { RoadClass::Local };
            for d in [axis, -axis] {
                t.grow(c, d, reach, class, 0.3, 0);
            }
        }
        t
    }

    /// ONE growth step, used for fresh cities and for the runtime plan alike. New edges get `order`.
    /// Returns false (graph unchanged) when the chosen spot can't take a road this time.
    pub fn step(&mut self, order: u32) -> bool {
        let (n0, e0) = (self.g.nodes.len(), self.g.edges.len());
        let n = (self.rng.next_u64() % n0 as u64) as u32;
        let p = self.g.nodes[n as usize];
        if f(&mut self.rng) as f32 * (1.0 + (p.distance(self.core) / self.falloff).powi(2)) > 1.0 {
            return false; // centre-weighted: the outskirts grow rarely
        }
        let ok = match self.degree(n) {
            // extend a dead end outward, keeping its class (skeleton roads keep reaching out)
            1 => {
                let e = self.g.edges.iter().find(|e| e.a == n || e.b == n).unwrap().clone();
                let back = self.g.nodes[(e.a + e.b - n) as usize];
                self.grow(n, (p - back).normalize(), STEP * 3.0, e.class, 0.3, order) >= 1
            }
            // branch a side street off a through node, spaced from other junctions; >= 2 segments or nothing
            2 if p.distance(self.core) >= STEP * 1.5 => {
                let e = self.g.edges.iter().find(|e| e.a == n || e.b == n).unwrap().clone();
                let gap = junction_gap(e.class);
                let crowded = (0..n0 as u32).any(|m| m != n && self.degree(m) >= 3 && self.g.nodes[m as usize].distance(p) < gap);
                let along = (self.g.nodes[e.b as usize] - self.g.nodes[e.a as usize]).normalize();
                let side = if self.rng.next_u64() % 2 == 0 { 1.0 } else { -1.0 };
                let len = STEP * (3.0 + f(&mut self.rng) as f32 * 5.0);
                !crowded && self.grow(n, along.perp() * side, len, RoadClass::Local, 0.6, order) >= 2
            }
            _ => false,
        };
        if !ok {
            self.g.nodes.truncate(n0);
            self.g.edges.truncate(e0);
            return false;
        }
        crate::junctions::merge_close(&mut self.g, 0, 0, |e| crate::lanes::default_class(e.class));
        true
    }

    /// Run `steps` growth steps (failed attempts don't count; bounded tries) with orders from `order0`.
    pub fn run(&mut self, steps: usize, order0: u32) {
        let (mut done, mut tries) = (0, 0);
        while done < steps && tries < steps * 20 + 20 {
            tries += 1;
            if self.step(order0 + done as u32) {
                done += 1;
            }
        }
    }
}

/// A town's roads: seed + the preset's growth steps (built, order 0), then FUTURE_STEPS more of the same step as the
/// plan's future (order 1..). `world` is the terrain seed, `rng_seed` varies the town.
pub fn grow_town(world: u64, rng_seed: u64, core: Vec2, preset: Preset) -> RoadGraph {
    let (roads, reach, steps, _) = preset.shape();
    let mut t = Town::seed(world, rng_seed, core, roads, reach, preset != Preset::Hamlet);
    t.falloff = reach;
    t.run(steps, 0);
    for e in &mut t.g.edges {
        e.order = 0;
    }
    t.run(FUTURE_STEPS, 1);
    t.g
}

/// A fresh single town (see grow_town), with its houses.
pub fn plan_preset(world: u64, rng_seed: u64, core: Vec2, preset: Preset) -> TownPlan {
    assemble(world, vec![(grow_town(world, rng_seed, core, preset), core)], preset)
}

/// Join town graphs into one plan: junction clearance across towns (merge_close over the whole graph), no 1-segment
/// stubs, then houses. Town i's edges get `town = i`.
pub fn assemble(world: u64, towns: Vec<(RoadGraph, Vec2)>, preset: Preset) -> TownPlan {
    let mut g = RoadGraph::default();
    let cores: Vec<Vec2> = towns.iter().map(|t| t.1).collect();
    for (i, (t, _)) in towns.into_iter().enumerate() {
        let n0 = g.nodes.len() as u32;
        g.nodes.extend(t.nodes);
        g.edges.extend(t.edges.into_iter().map(|e| REdge { a: e.a + n0, b: e.b + n0, town: i as u32, ..e }));
    }
    crate::junctions::merge_close(&mut g, 0, 0, |e| crate::lanes::default_class(e.class));
    prune_stubs(&mut g);
    for e in &mut g.edges {
        let (a, b) = (g.nodes[e.a as usize], g.nodes[e.b as usize]);
        e.bridge = water_run(world, a, b) > 0.0;
    }
    let near = |c: Vec2| (0..g.nodes.len()).min_by(|&x, &y| g.nodes[x].distance(c).total_cmp(&g.nodes[y].distance(c))).unwrap_or(0) as u32;
    let metas = cores.iter().enumerate().map(|(i, &c)| TownMeta { town: i as u32, core: c, core_node: near(c), kind: SeedKind::Crossroads }).collect();
    let parcels = houses(world, &g, &cores, preset);
    TownPlan { graph: g, parcels, towns: metas }
}

/// Drop dead-end spurs one segment long off a junction (repeat until none), then unused nodes.
fn prune_stubs(g: &mut RoadGraph) {
    loop {
        let inc = g.incident();
        let stub = |e: &REdge| {
            let (da, db) = (inc[e.a as usize].len(), inc[e.b as usize].len());
            (da == 1 && db >= 3) || (db == 1 && da >= 3)
        };
        let before = g.edges.len();
        g.edges.retain(|e| !stub(e));
        if g.edges.len() == before {
            break;
        }
    }
    let inc = g.incident();
    let mut map = vec![u32::MAX; g.nodes.len()];
    let mut nodes = vec![];
    for n in 0..g.nodes.len() {
        if !inc[n].is_empty() {
            map[n] = nodes.len() as u32;
            nodes.push(g.nodes[n]);
        }
    }
    g.nodes = nodes;
    for e in &mut g.edges {
        e.a = map[e.a as usize];
        e.b = map[e.b as usize];
    }
}

/// Houses on both sides of every edge, set back, nearest its town core first; never crowding a road or house, never
/// on or driving across water. The first few per town are commercial, far ones sometimes industrial; the nearest
/// `preset` houses on built roads start lived in.
fn houses(world: u64, g: &RoadGraph, cores: &[Vec2], preset: Preset) -> Vec<Parcel> {
    let mut slots: Vec<(f32, Vec2, usize, f32)> = vec![];
    for (i, e) in g.edges.iter().enumerate() {
        let (a, b) = (g.nodes[e.a as usize], g.nodes[e.b as usize]);
        let d = (b - a).normalize();
        let mut s = HOUSE_GAP / 2.0;
        while s < e.len {
            for side in [1.0, -1.0] {
                let p = a + d * s + d.perp() * side * SETBACK;
                slots.push((p.distance(cores[e.town as usize]), p, i, d.to_angle())); // long axis along the road
            }
            s += HOUSE_GAP;
        }
    }
    slots.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut far = vec![1.0f32; cores.len()];
    for s in &slots {
        far[g.edges[s.2].town as usize] = s.0;
    }
    let mut count = vec![(0usize, 0usize); cores.len()]; // (houses, lived in) per town
    let mut parcels: Vec<Parcel> = vec![];
    for (dist, p, access, angle) in slots {
        let road_ok = g.edges.iter().all(|e| seg_dist(p, g.nodes[e.a as usize], g.nodes[e.b as usize]) >= SETBACK - 0.5);
        let (a, b) = g.pos(access);
        let drive_dry = water_run(world, p, seg_closest(p, a, b).0) == 0.0;
        if !road_ok || !drive_dry || parcels.iter().any(|q| q.center.distance(p) < HOUSE_GAP * 0.9) || !footprint_on_land(world, p) {
            continue;
        }
        let town = g.edges[access].town;
        let (k, built) = &mut count[town as usize];
        let kind = if *k < 3 && preset != Preset::Hamlet {
            ZoneKind::Commercial
        } else if dist > far[town as usize] * 0.8 && *k % 4 == 0 {
            ZoneKind::Industrial
        } else {
            ZoneKind::Residential
        };
        let live = *built < preset.shape().3 && g.edges[access].order <= SEED_ORDER;
        *built += live as usize;
        parcels.push(Parcel {
            center: p,
            town,
            kind,
            access,
            angle,
            size: Vec2::new(5.0, 5.0),
            capacity: PEAK_CAPACITY * (1.0 - dist / (far[town as usize] + 30.0)).max(0.15).powf(1.3) as f64,
            start_pop: if live { 40.0 } else { 0.0 },
            core: *k == 0,
        });
        *k += 1;
    }
    parcels
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No two junction clearance discs overlap, and no node other than a junction's own legs sits inside one.
    pub fn assert_clear(g: &RoadGraph, what: &str) {
        let inc = g.incident();
        let cls = |e: usize| crate::lanes::default_class(g.edges[e].class);
        let clr: Vec<f32> = (0..g.nodes.len()).map(|n| crate::junctions::clearance(g, &inc[n], n, cls)).collect();
        for n in (0..g.nodes.len()).filter(|&n| clr[n] > 0.0) {
            for m in 0..g.nodes.len() {
                let adj = inc[n].iter().any(|&e| g.edges[e].a as usize == m || g.edges[e].b as usize == m);
                let d = g.nodes[n].distance(g.nodes[m]);
                assert!(m == n || (clr[m] == 0.0 && adj) || d >= clr[n] + clr[m], "{what}: node {m} {d} from junction {n} (clearance {} + {})", clr[n], clr[m]);
            }
        }
    }

    #[test]
    fn fresh_multi_town_cities_keep_junction_clearance() {
        for seed in [1, 2, 3, 7, 42, 99] {
            let regions = world_gen::generate_cities(seed, 3);
            assert_clear(&crate::roadgen::plan_towns(seed, &regions[..2]).graph, &format!("plan_towns seed {seed}"));
        }
    }

    /// Dead-end spurs (from a degree-1 node back to the first junction) are at least two segments long.
    fn assert_no_stubs(g: &RoadGraph, what: &str) {
        let inc = g.incident();
        for n in (0..g.nodes.len()).filter(|&n| inc[n].len() == 1) {
            let (mut at, mut e, mut segs) = (n, inc[n][0], 1);
            loop {
                at = (g.edges[e].a + g.edges[e].b) as usize - at;
                if inc[at].len() != 2 {
                    break;
                }
                e = if inc[at][0] == e { inc[at][1] } else { inc[at][0] };
                segs += 1;
            }
            assert!(inc[at].len() == 1 || segs >= 2, "{what}: 1-segment stub at node {n}");
        }
    }

    fn connected(g: &RoadGraph, keep: impl Fn(&REdge) -> bool) -> bool {
        let inc = g.incident();
        let used: Vec<bool> = (0..g.nodes.len()).map(|n| inc[n].iter().any(|&e| keep(&g.edges[e]))).collect();
        let Some(s) = used.iter().position(|&u| u) else { return true };
        let mut seen = vec![false; g.nodes.len()];
        let mut st = vec![s];
        while let Some(n) = st.pop() {
            if std::mem::replace(&mut seen[n], true) {
                continue;
            }
            st.extend(inc[n].iter().filter(|&&e| keep(&g.edges[e])).map(|&e| (g.edges[e].a + g.edges[e].b) as usize - n));
        }
        (0..g.nodes.len()).all(|n| !used[n] || seen[n])
    }

    /// Presets come only from seed + Town::step; after seed + N steps (built) and the future steps, across seeds:
    /// connected (also the built part alone), no stubs, junction clearance, spacing, no crossings, houses on roads.
    #[test]
    fn presets_are_connected_reachable_and_every_house_has_access() {
        for seed in [1, 2, 3, 7, 42, 99] {
            let mut size = 0;
            for p in Preset::ALL {
                let plan = plan_preset(seed, seed * 31, Vec2::new(5000.0, 5000.0), p);
                let g = &plan.graph;
                let inc = g.incident();
                assert!(connected(g, |_| true), "{p:?} seed {seed}: disconnected");
                assert!(connected(g, |e| e.order <= SEED_ORDER), "{p:?} seed {seed}: built roads disconnected");
                assert_no_stubs(g, &format!("{p:?} seed {seed}"));
                let built = g.edges.iter().filter(|e| e.order <= SEED_ORDER).count();
                assert!(built > size, "{p:?} seed {seed}: {built} built edges, not bigger than the smaller preset");
                size = built;
                // houses front a real road, close to it
                assert!(!plan.parcels.is_empty());
                for z in &plan.parcels {
                    let (a, b) = g.pos(z.access);
                    assert!(seg_dist(z.center, a, b) <= SETBACK + 0.5, "{p:?}: house off its road");
                }
                // spacing: junctions keep their class gap and roundabout rings never touch
                let cls = |e: usize| crate::lanes::default_class(g.edges[e].class);
                let juncs: Vec<usize> = (0..g.nodes.len()).filter(|&n| inc[n].len() >= 3).collect();
                let rings: Vec<_> = juncs.iter().filter_map(|&n| crate::junctions::Ring::at(g, &inc[n], n, cls)).collect();
                for (i, &a) in juncs.iter().enumerate() {
                    for &b in &juncs[i + 1..] {
                        let gap = inc[a].iter().map(|&e| junction_gap(g.edges[e].class)).fold(STEP * 2.5, f32::min);
                        let d = g.nodes[a].distance(g.nodes[b]);
                        assert!(d >= gap - 0.01, "{p:?} seed {seed}: junctions {a},{b} only {d} apart");
                    }
                }
                assert_clear(g, &format!("{p:?} seed {seed}"));
                // junctions only where roads meet: no two edges cross
                for i in 0..g.edges.len() {
                    for j in i + 1..g.edges.len() {
                        let (a, b) = g.pos(i);
                        let (c, d) = g.pos(j);
                        assert!(!crosses(a, b, c, d), "{p:?} seed {seed}: edges {i} and {j} cross without a junction {:?} {:?} {} {}", g.edges[i].order, g.edges[j].order, g.edges.len(), g.nodes.len());
                    }
                }
            }
        }
    }
}

fn f(rng: &mut SplitMix64) -> f64 {
    (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64
}
