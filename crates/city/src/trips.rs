//! Car journeys (R28): a trip is sampled from the same demand model as the traffic assignment (home zones by
//! population and car share, destinations by attractiveness and distance, work vs shopping by the hour) and routed
//! over the roads that are built right now. Pure and deterministic given its RNG; the renderer animates them.
use crate::roadgen::{RoadClass, TownPlan, ZoneKind};
use crate::traffic::{shop_profile, work_profile, SPEED_ARTERIAL, SPEED_LOCAL};
use bevy::math::Vec2;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use world_gen::rng::{unit, SplitMix64};

const GAMMA: f64 = 0.05;
/// Straight-line distance to rough minutes when choosing destinations.
const DIST_TO_MIN: f64 = 1.0 / SPEED_LOCAL;

/// One car journey: the polyline from the origin building to the destination building and, per segment, the
/// road edge it runs on (for congestion lookup) and its free-flow speed (world units per game minute).
#[derive(Debug, Clone, PartialEq)]
pub struct Trip {
    pub from: usize,
    pub to: usize,
    pub pts: Vec<Vec2>,
    pub seg_edge: Vec<usize>,
    pub seg_speed: Vec<f32>,
    /// Road nodes passed, origin access node first.
    pub nodes: Vec<u32>,
    /// Road edges driven, in order.
    pub edges: Vec<usize>,
}

pub struct Router {
    adj: Vec<Vec<(u32, u32)>>,
    node_pos: Vec<Vec2>,
    t0: Vec<f32>,
    speed: Vec<f32>,
    ends: Vec<(u32, u32)>,
    access_node: Vec<u32>,
    access_edge: Vec<usize>,
    centre: Vec<Vec2>,
    commercial: Vec<bool>,
}

impl Router {
    pub fn new(plan: &TownPlan) -> Self {
        let g = &plan.graph;
        let mut adj = vec![vec![]; g.nodes.len()];
        for (i, e) in g.edges.iter().enumerate() {
            adj[e.a as usize].push((e.b, i as u32));
            adj[e.b as usize].push((e.a, i as u32));
        }
        let speed: Vec<f32> =
            g.edges.iter().map(|e| (if e.class == RoadClass::Arterial { SPEED_ARTERIAL } else { SPEED_LOCAL }) as f32).collect();
        Self {
            adj,
            node_pos: g.nodes.clone(),
            t0: g.edges.iter().zip(&speed).map(|(e, s)| e.len / s).collect(),
            speed,
            ends: g.edges.iter().map(|e| (e.a, e.b)).collect(),
            access_node: (0..plan.parcels.len()).map(|z| plan.access_node(z)).collect(),
            access_edge: plan.parcels.iter().map(|p| p.access).collect(),
            centre: plan.parcels.iter().map(|p| p.center).collect(),
            commercial: plan.parcels.iter().map(|p| p.kind == ZoneKind::Commercial).collect(),
        }
    }

    /// A zone generates trips only if people live there and its frontage road exists.
    pub fn active(&self, built: &[bool], pop: &[f32], z: usize) -> bool {
        built.get(self.access_edge[z]).copied().unwrap_or(false) && pop.get(z).copied().unwrap_or(0.0) >= 1.0
    }

    /// Sample (home zone, destination zone) for the given hour. `share[z]` = zone z's transit share: those trips
    /// are not cars. None if the city has fewer than two active zones.
    pub fn sample(&self, built: &[bool], pop: &[f32], share: &[f32], hour: f64, rng: &mut SplitMix64) -> Option<(usize, usize)> {
        let n = self.centre.len();
        let pick = |w: &[f64], rng: &mut SplitMix64| -> Option<usize> {
            let total: f64 = w.iter().sum();
            if total <= 0.0 {
                return None;
            }
            let mut x = unit(rng.next_u64()) * total;
            w.iter().position(|v| {
                x -= v;
                x < 0.0
            })
        };
        let origin_w: Vec<f64> = (0..n)
            .map(|z| if self.active(built, pop, z) { pop[z] as f64 * (1.0 - share.get(z).copied().unwrap_or(0.0) as f64) } else { 0.0 })
            .collect();
        let from = pick(&origin_w, rng)?;
        let (work, shop) = (work_profile(hour), shop_profile(hour));
        let shopping = unit(rng.next_u64()) * (work + shop) >= work;
        let dest_w: Vec<f64> = (0..n)
            .map(|j| {
                if j == from || !self.active(built, pop, j) {
                    return 0.0;
                }
                let t = self.centre[from].distance(self.centre[j]) as f64 * DIST_TO_MIN;
                let attract = pop[j] as f64 * if shopping && self.commercial[j] { 3.0 } else if shopping { 1.0 } else { 0.4 + 2.0 * self.commercial[j] as u8 as f64 };
                attract * (-GAMMA * t).exp()
            })
            .collect();
        Some((from, pick(&dest_w, rng)?))
    }

    /// Fastest route between two zones' buildings over built roads; `slow[e]` = congestion slowdown (>= 1, empty = free).
    pub fn route(&self, built: &[bool], slow: &[f32], from: usize, to: usize) -> Option<Trip> {
        let (src, dst) = (self.access_node[from], self.access_node[to]);
        let mut dist = vec![f32::INFINITY; self.adj.len()];
        let mut pred = vec![u32::MAX; self.adj.len()];
        let mut h = BinaryHeap::new();
        dist[src as usize] = 0.0;
        h.push(Reverse((0f32.to_bits(), src)));
        while let Some(Reverse((bits, u))) = h.pop() {
            let du = f32::from_bits(bits);
            if du > dist[u as usize] {
                continue;
            }
            if u == dst {
                break;
            }
            for &(v, e) in &self.adj[u as usize] {
                if !built.get(e as usize).copied().unwrap_or(false) {
                    continue;
                }
                let nd = du + self.t0[e as usize] * slow.get(e as usize).copied().unwrap_or(1.0).max(1.0);
                if nd < dist[v as usize] {
                    dist[v as usize] = nd;
                    pred[v as usize] = e;
                    h.push(Reverse((nd.to_bits(), v)));
                }
            }
        }
        if !dist[dst as usize].is_finite() {
            return None;
        }
        let (mut nodes, mut edges) = (vec![dst], vec![]);
        let mut cur = dst;
        while cur != src {
            let e = pred[cur as usize] as usize;
            edges.push(e);
            cur = if self.ends[e].0 == cur { self.ends[e].1 } else { self.ends[e].0 };
            nodes.push(cur);
        }
        nodes.reverse();
        edges.reverse();
        let mut pts = vec![self.centre[from]];
        let (mut seg_edge, mut seg_speed) = (vec![self.access_edge[from]], vec![SPEED_LOCAL as f32]);
        pts.extend(nodes.iter().map(|&n| self.node_pos[n as usize]));
        for &e in &edges {
            seg_edge.push(e);
            seg_speed.push(self.speed[e]);
        }
        pts.push(self.centre[to]);
        seg_edge.push(self.access_edge[to]);
        seg_speed.push(SPEED_LOCAL as f32);
        Some(Trip { from, to, pts, seg_edge, seg_speed, nodes, edges })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roadgen::plan_towns;
    use world_gen::generate_cities;

    fn setup() -> (TownPlan, Router, Vec<bool>, Vec<f32>) {
        let plan = plan_towns(7, &generate_cities(7, 3)[..2]);
        let r = Router::new(&plan);
        let built = vec![true; plan.graph.edges.len()];
        let pop = plan.parcels.iter().map(|p| (p.capacity * 0.5) as f32).collect();
        (plan, r, built, pop)
    }

    #[test]
    fn a_journey_runs_from_its_origin_to_its_destination_over_connected_built_roads() {
        let (plan, r, built, pop) = setup();
        let share = vec![0.0; pop.len()];
        let mut rng = SplitMix64::new(11);
        let mut routed = 0;
        for _ in 0..300 {
            let (a, b) = r.sample(&built, &pop, &share, 8.0, &mut rng).unwrap();
            let Some(t) = r.route(&built, &[], a, b) else { continue };
            routed += 1;
            assert_eq!((t.from, t.to), (a, b));
            assert_eq!(t.pts[0], plan.parcels[a].center);
            assert_eq!(*t.pts.last().unwrap(), plan.parcels[b].center);
            assert_eq!(t.nodes[0], plan.access_node(a));
            assert_eq!(*t.nodes.last().unwrap(), plan.access_node(b));
            // every driven edge is built and joins consecutive nodes
            for (w, &e) in t.nodes.windows(2).zip(&t.edges) {
                let ed = &plan.graph.edges[e];
                assert!(built[e] && ((ed.a, ed.b) == (w[0], w[1]) || (ed.a, ed.b) == (w[1], w[0])), "edge {e} does not join {w:?}");
            }
            assert_eq!(t.pts.len(), t.nodes.len() + 2);
            assert_eq!(t.seg_edge.len(), t.pts.len() - 1);
        }
        assert!(routed > 250, "most trips between active zones are routable: {routed}");
    }

    #[test]
    fn unbuilt_roads_are_not_driven_and_empty_zones_start_no_trips() {
        let (plan, r, mut built, mut pop) = setup();
        let share = vec![0.0; pop.len()];
        let mut rng = SplitMix64::new(3);
        let (a, b) = r.sample(&built, &pop, &share, 12.0, &mut rng).unwrap();
        let t = r.route(&built, &[], a, b);
        if let Some(t) = t {
            built[t.edges[t.edges.len() / 2]] = false;
            if let Some(t2) = r.route(&built, &[], a, b) {
                assert!(t2.edges.iter().all(|&e| built[e]));
            }
        }
        let empty = 0;
        pop[empty] = 0.0;
        for _ in 0..200 {
            let (o, d) = r.sample(&vec![true; plan.graph.edges.len()], &pop, &share, 8.0, &mut rng).unwrap();
            assert!(o != empty && d != empty);
        }
        assert!(r.sample(&vec![false; plan.graph.edges.len()], &pop, &share, 8.0, &mut rng).is_none());
    }

    #[test]
    fn bigger_zones_start_more_trips_and_transit_riders_do_not_drive() {
        let (_, r, built, mut pop) = setup();
        let n = pop.len();
        let mut rng = SplitMix64::new(5);
        pop[1] *= 8.0;
        let mut starts = vec![0u32; n];
        let share = vec![0.0; n];
        for _ in 0..4000 {
            starts[r.sample(&built, &pop, &share, 8.0, &mut rng).unwrap().0] += 1;
        }
        assert!(starts[1] > 3 * starts[2].max(1), "{} vs {}", starts[1], starts[2]);
        let mut all_transit = vec![0.0; n];
        all_transit[1] = 1.0;
        let mut again = 0;
        for _ in 0..1000 {
            again += (r.sample(&built, &pop, &all_transit, 8.0, &mut rng).unwrap().0 == 1) as u32;
        }
        assert_eq!(again, 0, "a zone whose trips all go by transit starts no car trips");
    }

    #[test]
    fn congestion_makes_the_router_choose_another_way_or_take_longer() {
        let (plan, r, built, pop) = setup();
        let share = vec![0.0; pop.len()];
        let mut rng = SplitMix64::new(9);
        let (a, b) = r.sample(&built, &pop, &share, 8.0, &mut rng).unwrap();
        let t = r.route(&built, &[], a, b).unwrap();
        let mut slow = vec![1.0f32; plan.graph.edges.len()];
        for &e in &t.edges {
            slow[e] = 6.0;
        }
        let t2 = r.route(&built, &slow, a, b).unwrap();
        let cost = |t: &Trip| t.edges.iter().map(|&e| r.t0[e] * slow[e]).sum::<f32>();
        assert!(cost(&t2) <= cost(&t) + 1e-4);
    }
}
