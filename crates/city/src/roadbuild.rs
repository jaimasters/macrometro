//! City road growth (D-7). The city reserves budget and builds the planned road graph outward as the town develops:
//! one organic plan per 1/(GROWTH_RATE*ROADS_PER_RATE) ticks, plus growth::RoadAgency plans for underserved built zones
//! (poor transit -> the city builds more road near them). Every road is planned (ghost) LEAD ticks before it is built.
use crate::roadgen::{RoadClass, TownPlan, SEED_ORDER};
use bevy::math::Vec2;
use growth::{AgencyParams, RoadAgency, Zones};

/// A zone with at least this population counts as built (drawn, and its street may be extended past).
pub const BUILT_POP: f64 = 30.0;
/// Road plans emitted per tick = rate * ROADS_PER_RATE. With the default GROWTH_RATE (0.0015) and no transit
/// (x0.4) that is one road about every 12 s.
pub const ROADS_PER_RATE: f64 = 14.0;
/// Ticks between a road being planned (ghost) and built.
pub const LEAD_TICKS: u64 = 100;
const MAX_PENDING: usize = 3;
const STALL_TICKS: u32 = 300;
/// Construction cost per world unit of city road (feeds the road reserve figure).
pub const ROAD_COST_PER_LEN: f64 = 1.5;

#[derive(Clone)]
pub struct RoadBuilder {
    pub built: Vec<bool>,
    /// (edge, build tick), in plan order.
    pub pending: Vec<(usize, u64)>,
    agency: RoadAgency,
    accum: f64,
    stall: u32,
    node_built: Vec<bool>,
}

impl RoadBuilder {
    pub fn new(plan: &TownPlan, agency: AgencyParams) -> Self {
        let g = &plan.graph;
        let built: Vec<bool> = g.edges.iter().map(|e| e.order <= SEED_ORDER).collect();
        let mut built = built;
        for p in plan.parcels.iter().filter(|p| p.start_pop > 0.0) {
            built[p.access] = true; // a zone that starts populated starts with its road
        }
        let mut node_built = vec![false; g.nodes.len()];
        for t in &plan.towns {
            node_built[t.core_node as usize] = true;
        }
        for (e, b) in g.edges.iter().zip(&built) {
            if *b {
                node_built[e.a as usize] = true;
                node_built[e.b as usize] = true;
            }
        }
        Self { built, pending: vec![], agency: RoadAgency::new(agency), accum: 0.0, stall: 0, node_built }
    }

    pub fn is_built(&self, edge: usize) -> bool {
        self.built[edge]
    }

    pub fn built_count(&self) -> usize {
        self.built.iter().filter(|b| **b).count()
    }

    /// Sum of pending road costs (the road reserve the city is holding back).
    pub fn reserve(&self, plan: &TownPlan) -> f64 {
        self.pending.iter().map(|&(e, _)| plan.graph.edges[e].len as f64 * ROAD_COST_PER_LEN).sum()
    }

    /// Is the neighbourhood behind node `u` filled in? At least half of the parcels with built road within 20
    /// units of it have grown to BUILT_POP (vacuously true with none).
    fn ready(&self, plan: &TownPlan, zones: &Zones, u: u32) -> bool {
        let p = plan.graph.nodes[u as usize];
        let (mut n, mut dev) = (0usize, 0usize);
        for (z, pc) in plan.parcels.iter().enumerate() {
            if self.built[pc.access] && pc.center.distance(p) <= 20.0 {
                n += 1;
                dev += (zones.0[z].pop >= BUILT_POP) as usize;
            }
        }
        dev * 2 >= n
    }

    /// Parcels still without road that this edge would serve (fronting it or near its far end).
    fn pull(&self, plan: &TownPlan, e: usize, far: Vec2) -> usize {
        plan.parcels.iter().filter(|p| !self.built[p.access] && (p.access == e || p.center.distance(far) <= 22.0)).count()
    }

    /// Best unbuilt, unplanned frontier edge. `anywhere` skips the "street behind is filled in" rule;
    /// `near` picks the frontier edge closest to a point (for underserved-zone plans) within 35 units.
    fn pick(&self, plan: &TownPlan, zones: &Zones, anywhere: bool, near: Option<Vec2>) -> Option<usize> {
        let g = &plan.graph;
        let mut best: Option<(f64, usize)> = None;
        for (e, ed) in g.edges.iter().enumerate() {
            if self.built[e] || self.pending.iter().any(|p| p.0 == e) {
                continue;
            }
            let (u, w) = if self.node_built[ed.a as usize] {
                (ed.a, ed.b)
            } else if self.node_built[ed.b as usize] {
                (ed.b, ed.a)
            } else {
                continue;
            };
            let score = match near {
                Some(p) => {
                    let (a, b) = g.pos(e);
                    let d = ((a + b) * 0.5).distance(p);
                    if d > 35.0 {
                        continue;
                    }
                    -(d as f64)
                }
                None => {
                    let pull = self.pull(plan, e, g.nodes[w as usize]);
                    if !anywhere && (pull == 0 || !self.ready(plan, zones, u)) {
                        continue;
                    }
                    pull as f64 + if ed.class == RoadClass::Arterial { 3.0 } else { 0.0 } - 0.02 * ed.order as f64
                }
            };
            if best.map_or(true, |(s, _)| score > s) {
                best = Some((score, e));
            }
        }
        best.map(|b| b.1)
    }

    /// One tick. `rate` = plans per tick (already gated by transit pace), `shares[i]` = zone i's transit share.
    /// Returns the edges that finished building this tick.
    pub fn step(&mut self, tick: u64, plan: &TownPlan, zones: &Zones, shares: &[f64], rate: f64) -> Vec<usize> {
        let mut newly = vec![];
        let (due, keep): (Vec<(usize, u64)>, Vec<(usize, u64)>) = self.pending.iter().copied().partition(|p| p.1 <= tick);
        self.pending = keep;
        for (e, _) in due {
            if !self.built[e] {
                self.built[e] = true;
                self.node_built[plan.graph.edges[e].a as usize] = true;
                self.node_built[plan.graph.edges[e].b as usize] = true;
                newly.push(e);
            }
        }
        // organic extension behind developed streets
        self.accum = (self.accum + rate).min(1.5);
        if self.accum >= 1.0 && self.pending.len() < MAX_PENDING {
            let pick = self.pick(plan, zones, false, None).or_else(|| {
                (self.pending.is_empty() && self.stall >= STALL_TICKS).then(|| self.pick(plan, zones, true, None)).flatten()
            });
            match pick {
                Some(e) => {
                    self.accum -= 1.0;
                    self.stall = 0;
                    self.pending.push((e, tick + LEAD_TICKS));
                }
                None => self.stall += 1,
            }
        }
        // D-7: built, well-filled zones the city thinks are underserved pull a nearby road ahead of the queue
        let open = self.pending.len() < MAX_PENDING;
        let adj: Vec<f64> = zones
            .0
            .iter()
            .enumerate()
            .map(|(i, z)| if open && z.pop >= BUILT_POP && z.pop / z.capacity.max(1.0) >= 0.3 { shares[i] } else { 1.0 })
            .collect();
        for p in self.agency.observe(tick, zones, &adj).planned {
            if self.pending.len() >= MAX_PENDING {
                break;
            }
            let c = plan.parcels[p.zone.0 as usize].center;
            if let Some(e) = self.pick(plan, zones, true, Some(c)) {
                self.pending.push((e, p.build_tick));
            }
        }
        newly
    }

    pub fn hash(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for (i, b) in self.built.iter().enumerate() {
            if *b {
                h = (h ^ i as u64).wrapping_mul(0x100000001b3);
            }
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roadgen::plan_towns;
    use growth::{Zone, ZoneId};
    use world_gen::generate_cities;

    fn zones(plan: &TownPlan, f: f64) -> Zones {
        Zones::new(
            plan.parcels
                .iter()
                .enumerate()
                .map(|(i, p)| Zone { id: ZoneId(i as u32), pop: p.capacity * f, jobs: p.capacity * f * 0.4, capacity: p.capacity, crowding: 0.0 })
                .collect(),
        )
    }

    fn params() -> AgencyParams {
        AgencyParams { s_min: 0.2, t_ticks: 100, lead_ticks: 100, cost_per_pop: 1.0 }
    }

    #[test]
    fn builds_outward_connected_at_the_set_rate() {
        let plan = plan_towns(7, &generate_cities(7, 3)[..2]);
        let z = zones(&plan, 0.5); // everything developed: only the rate limits growth
        let mut b = RoadBuilder::new(&plan, params());
        let start = b.built_count();
        let shares = vec![1.0; z.0.len()];
        for t in 0..1200 {
            b.step(t, &plan, &z, &shares, 1.0 / 120.0);
        }
        let gained = b.built_count() - start;
        assert!((6..=14).contains(&gained), "built {gained} in 120 s at 1 per 12 s");
        // every built edge touches the built network (grown outward, never in isolation)
        let mut reach = vec![false; plan.graph.nodes.len()];
        for t in &plan.towns {
            reach[t.core_node as usize] = true;
        }
        for _ in 0..plan.graph.edges.len() {
            for (i, e) in plan.graph.edges.iter().enumerate() {
                if b.built[i] && (reach[e.a as usize] || reach[e.b as usize]) {
                    reach[e.a as usize] = true;
                    reach[e.b as usize] = true;
                }
            }
        }
        assert!(plan.graph.edges.iter().enumerate().all(|(i, e)| !b.built[i] || reach[e.a as usize]), "isolated built road");
    }

    #[test]
    fn undeveloped_streets_stall_extension_then_failsafe_resumes() {
        let plan = plan_towns(7, &generate_cities(7, 3)[..2]);
        let grow = |fill: f64, ticks: u64| {
            let z = zones(&plan, fill);
            let mut b = RoadBuilder::new(&plan, params());
            let start = b.built_count();
            let shares = vec![1.0; z.0.len()];
            for t in 0..ticks {
                b.step(t, &plan, &z, &shares, 1.0 / 20.0);
            }
            b.built_count() - start
        };
        // streets behind the frontier undeveloped: only bare connectors extend, far fewer than when the town fills in
        let (stalled, filled) = (grow(0.0, 1000), grow(0.5, 1000));
        assert!(stalled + 4 < filled, "undeveloped {stalled} vs developed {filled}");
        assert!(grow(0.0, 5000) > stalled, "failsafe never resumed growth");
    }

    #[test]
    fn underserved_zones_pull_roads_and_runs_are_deterministic() {
        let plan = plan_towns(7, &generate_cities(7, 3)[..2]);
        let z = zones(&plan, 0.5);
        let run = |share: f64| {
            let mut b = RoadBuilder::new(&plan, params());
            let shares = vec![share; z.0.len()];
            for t in 0..600 {
                b.step(t, &plan, &z, &shares, 0.0);
            }
            (b.built_count(), b.hash())
        };
        let poor = run(0.0);
        assert!(poor.0 > RoadBuilder::new(&plan, params()).built_count(), "no underserved plans built");
        assert_eq!(poor, run(0.0));
        assert_eq!(run(1.0).0, RoadBuilder::new(&plan, params()).built_count(), "well-served zones should not pull roads");
    }
}
