//! R42/R43 organic towns: long curving skeleton roads to a fixed reach, curved side streets that branch off and stop
//! short of other roads (junctions only where roads really meet), and houses set back along the frontage, ordered
//! out from the core. Curves are chains of short edges (degree-2 bends), so the RoadGraph shape is unchanged.
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
    /// (skeleton roads, reach from the core, side streets, built houses)
    fn shape(self) -> (usize, f32, usize, usize) {
        match self {
            Preset::Hamlet => (1, 220.0, 0, 5),
            Preset::SmallTown => (2, 320.0, 8, 40),
            Preset::LargeTown => (2, 520.0, 26, 140),
        }
    }
}

const STEP: f32 = 16.0;
const SETBACK: f32 = 6.0;
const HOUSE_GAP: f32 = 9.0;
const CLEAR: f32 = 5.0;

fn crosses(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    let o = |p: Vec2, q: Vec2, r: Vec2| (q - p).perp_dot(r - p);
    o(a, b, c) * o(a, b, d) < 0.0 && o(c, d, a) * o(c, d, b) < 0.0
}

struct G {
    g: RoadGraph,
}

impl G {
    fn node(&mut self, p: Vec2) -> u32 {
        self.g.nodes.push(p);
        self.g.nodes.len() as u32 - 1
    }
    fn edge(&mut self, a: u32, b: u32, class: RoadClass, order: u32) {
        let len = self.g.nodes[a as usize].distance(self.g.nodes[b as usize]);
        self.g.edges.push(REdge { a, b, class, bridge: false, len, town: 0, order });
    }
    /// Would a new segment p-q cross or crowd an existing road (other than at node `from`)?
    fn blocked(&self, from: u32, p: Vec2, q: Vec2) -> bool {
        self.g.edges.iter().any(|e| {
            let (a, b) = (self.g.nodes[e.a as usize], self.g.nodes[e.b as usize]);
            let touches = e.a == from || e.b == from;
            crosses(p, q, a, b) || (!touches && seg_dist(q, a, b) < CLEAR * 2.0)
        })
    }
    /// A meandering road from node `n` heading `dir` for `len`, stopping early if it would hit another road.
    fn grow(&mut self, rng: &mut SplitMix64, mut n: u32, mut dir: Vec2, len: f32, class: RoadClass, wiggle: f32, order: &mut u32) -> usize {
        let mut made = 0;
        for _ in 0..(len / STEP).ceil() as usize {
            dir = Vec2::from_angle((f(rng) as f32 - 0.5) * wiggle).rotate(dir).normalize();
            let p = self.g.nodes[n as usize];
            let q = p + dir * STEP;
            if self.blocked(n, p, q) {
                break;
            }
            let m = self.node(q);
            *order += 1;
            self.edge(n, m, class, *order);
            n = m;
            made += 1;
        }
        made
    }
}

/// A fresh town for a reset: skeleton roads cross at `core` and run `reach` out, then side streets and houses.
pub fn plan_preset(seed: u64, core: Vec2, preset: Preset) -> TownPlan {
    let (roads, reach, streets, houses) = preset.shape();
    let mut rng = SplitMix64::new(seed ^ 0x0E6A_1C);
    let mut g = G { g: RoadGraph::default() };
    let c = g.node(core);
    let turn = f(&mut rng) as f32 * std::f32::consts::TAU;
    // skeleton: both directions from the core, so a two-road town crosses in the middle (one real junction)
    for k in 0..roads {
        let axis = Vec2::from_angle(turn + k as f32 * std::f32::consts::FRAC_PI_2);
        for d in [axis, -axis] {
            let mut o = 0;
            let mut n = c;
            let mut i = 0;
            while g.g.nodes[n as usize].distance(core) < reach && i < 200 {
                i += 1;
                // gentle bends, steered back toward the axis so the road keeps its reach
                let p = g.g.nodes[n as usize];
                let off = (p - core).perp_dot(d);
                let bend = (f(&mut rng) as f32 - 0.5) * 0.5 + off * 0.01;
                let q = p + Vec2::from_angle(bend).rotate(d) * STEP;
                let m = g.node(q);
                let class = if preset == Preset::LargeTown && k == 0 { RoadClass::Arterial } else { RoadClass::Local };
                g.edge(n, m, class, o.max(i as u32));
                o += 1;
                n = m;
            }
        }
    }
    // side streets branch off existing nodes (never mid-edge), curve, and stop short of other roads
    let mut tries = 0;
    let mut made = 0;
    while made < streets && tries < streets * 20 {
        tries += 1;
        let n = (rng.next_u64() % g.g.nodes.len() as u64) as u32;
        let p = g.g.nodes[n as usize];
        if p.distance(core) > reach * 0.6 || p.distance(core) < STEP * 1.5 {
            continue;
        }
        let deg = g.g.edges.iter().filter(|e| e.a == n || e.b == n).count();
        if deg != 2 {
            continue;
        }
        let e = g.g.edges.iter().find(|e| e.a == n || e.b == n).unwrap();
        let along = (g.g.nodes[e.b as usize] - g.g.nodes[e.a as usize]).normalize();
        let side = if rng.next_u64() % 2 == 0 { 1.0 } else { -1.0 };
        let len = STEP * (3.0 + (f(&mut rng) as f32) * 5.0);
        let mut o = 100 + made as u32 * 10;
        if g.grow(&mut rng, n, along.perp() * side, len, RoadClass::Local, 0.6, &mut o) >= 2 {
            made += 1;
        }
    }
    // houses: both sides of every edge, set back, in order out from the core; skip anything crowding a road/house
    let mut slots: Vec<(f32, Vec2, usize, f32)> = vec![];
    for (i, e) in g.g.edges.iter().enumerate() {
        let (a, b) = (g.g.nodes[e.a as usize], g.g.nodes[e.b as usize]);
        let d = (b - a).normalize();
        let mut t = HOUSE_GAP / 2.0;
        while t < e.len {
            for s in [1.0, -1.0] {
                let p = a + d * t + d.perp() * s * SETBACK;
                slots.push((p.distance(core), p, i, (-d.perp() * s).to_angle()));
            }
            t += HOUSE_GAP;
        }
    }
    slots.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut parcels: Vec<Parcel> = vec![];
    for (dist, p, access, angle) in slots {
        let road_ok = g.g.edges.iter().all(|e| seg_dist(p, g.g.nodes[e.a as usize], g.g.nodes[e.b as usize]) >= SETBACK - 0.5);
        if !road_ok || parcels.iter().any(|q| q.center.distance(p) < HOUSE_GAP * 0.9) || !footprint_on_land(seed, p) {
            continue;
        }
        let k = parcels.len();
        parcels.push(Parcel {
            center: p,
            town: 0,
            kind: if k < 3 && preset != Preset::Hamlet { ZoneKind::Commercial } else { ZoneKind::Residential },
            access,
            angle,
            size: Vec2::new(5.0, 5.0),
            capacity: (PEAK_CAPACITY * 0.05 / (1.0 + dist as f64 / 60.0)).max(20.0),
            start_pop: if k < houses { 40.0 } else { 0.0 },
            core: k == 0,
        });
    }
    for e in &mut g.g.edges {
        let (a, b) = (g.g.nodes[e.a as usize], g.g.nodes[e.b as usize]);
        e.bridge = water_run(seed, a, b) > 0.0;
    }
    TownPlan { graph: g.g, parcels, towns: vec![TownMeta { town: 0, core, core_node: c, kind: SeedKind::Crossroads }] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_connected_reachable_and_every_house_has_access() {
        for seed in [1, 7, 42] {
            for p in Preset::ALL {
                let plan = plan_preset(seed, Vec2::new(5000.0, 5000.0), p);
                let g = &plan.graph;
                // connected
                let inc = g.incident();
                let mut seen = vec![false; g.nodes.len()];
                let mut st = vec![0usize];
                while let Some(n) = st.pop() {
                    if std::mem::replace(&mut seen[n], true) {
                        continue;
                    }
                    for &e in &inc[n] {
                        st.push((g.edges[e].a + g.edges[e].b) as usize - n);
                    }
                }
                assert!(seen.iter().all(|&s| s), "{p:?} seed {seed}: disconnected");
                // skeleton reach
                let far = g.nodes.iter().map(|q| q.distance(plan.towns[0].core)).fold(0.0, f32::max);
                let reach = p.shape().1;
                assert!(far >= reach * 0.9, "{p:?} seed {seed}: reach {far} < {reach}");
                // houses front a real road, close to it
                assert!(!plan.parcels.is_empty());
                for z in &plan.parcels {
                    let (a, b) = g.pos(z.access);
                    assert!(seg_dist(z.center, a, b) <= SETBACK + 0.5, "{p:?}: house off its road");
                }
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
