//! Traffic and mode choice over the generated road graph.
//! Per assignment: gravity trips (work + shop, time-of-day peaks) over congested car time; logit mode choice
//! against door-to-door transit time; car flow routed on built roads (BPR delay on smoothed edge flow).
//! Deterministic: fixed iteration orders, no hashing of floats into ordering. Units: minutes, world units, veh/h.
use crate::roadgen::{RoadClass, TownPlan, ZoneKind};
use growth::{CostMatrix, Zones};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// One game day in sim ticks (10 ticks/s => a 4 minute day).
pub const DAY_TICKS: u64 = 2400;
/// Ticks between assignments (2 s).
pub const TRAFFIC_EVERY: u64 = 20;
pub const SPEED_LOCAL: f64 = 4.0;
pub const SPEED_ARTERIAL: f64 = 7.0;
pub const CAP_LOCAL: f64 = 300.0;
pub const CAP_ARTERIAL: f64 = 900.0;
/// Gridlock floor: congestion never slows an edge by more than this factor.
pub const MAX_SLOWDOWN: f64 = 6.0;
/// A crossing shut all the time still passes a tenth of its capacity.
const MAX_CLOSURE: f64 = 0.9;
const BPR_ALPHA: f64 = 1.0;
const BPR_BETA: f64 = 4.0;
const PARK_MIN: f64 = 3.0;
/// Cars feel nicer than their clock time: transit must beat car time by this many minutes to be preferred.
pub const CAR_BIAS: f64 = 4.0;
pub const MODE_BETA: f64 = 0.15;
const GAMMA: f64 = 0.05;
/// Flow smoothing toward each new assignment (method of successive averages).
const RHO: f64 = 0.4;
const WORK_PER_DAY: f64 = 1.2;
const SHOP_PER_DAY: f64 = 1.6;
const ACCESS_HORIZON: f64 = 30.0;

pub fn hour_of(tick: u64) -> f64 {
    (tick % DAY_TICKS) as f64 / DAY_TICKS as f64 * 24.0
}

fn bell(h: f64, mu: f64, sigma: f64) -> f64 {
    (-((h - mu) / sigma).powi(2) / 2.0).exp()
}

/// Commute peaks about 08:00 and 17:20; mean ~0.8 over the day.
pub fn work_profile(h: f64) -> f64 {
    0.2 + 2.0 * (bell(h, 8.0, 1.4) + bell(h, 17.3, 1.6))
}

/// Shopping/leisure peaks at lunch and early evening.
pub fn shop_profile(h: f64) -> f64 {
    0.25 + 1.5 * bell(h, 12.5, 1.8) + 1.8 * bell(h, 18.0, 2.0)
}

/// Probability a trip goes by transit given door-to-door minutes by each mode.
pub fn p_transit(t_car: f64, t_transit: f64) -> f64 {
    1.0 / (1.0 + (MODE_BETA * (t_transit - t_car - CAR_BIAS)).exp())
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrafficOut {
    /// Transit share of each zone's trips (0 for zones without trips).
    pub zone_share: Vec<f64>,
    pub car_trips: f64,
    pub transit_trips: f64,
    /// Vehicle-distance (veh-units/h), a proxy for emissions.
    pub car_vkt: f64,
    pub mean_trip_time: f64,
    /// Pop-weighted share of jobs reachable within 30 min by the better mode.
    pub accessibility: f64,
    /// 1 - spread of transit share across developed zones.
    pub equity: f64,
}

impl TrafficOut {
    pub fn transit_share(&self) -> f64 {
        let t = self.car_trips + self.transit_trips;
        if t > 0.0 { self.transit_trips / t } else { 0.0 }
    }
}

#[derive(Clone)]
pub struct Traffic {
    adj: Vec<Vec<(u32, u32)>>,
    ends: Vec<(u32, u32)>,
    len: Vec<f64>,
    t0: Vec<f64>,
    cap: Vec<f64>,
    /// Smoothed car flow per edge (veh/h, both directions).
    pub flow: Vec<f64>,
    /// Fraction of time each edge is closed by a train on a level crossing (0..1).
    closed: Vec<f32>,
    zone_node: Vec<u32>,
    zone_acc: Vec<f64>,
    access: Vec<usize>,
    commercial: Vec<bool>,
    pub out: TrafficOut,
}

impl Traffic {
    pub fn new(plan: &TownPlan) -> Self {
        let g = &plan.graph;
        let mut adj = vec![vec![]; g.nodes.len()];
        for (i, e) in g.edges.iter().enumerate() {
            adj[e.a as usize].push((e.b, i as u32));
            adj[e.b as usize].push((e.a, i as u32));
        }
        let speed = |c: RoadClass| if c == RoadClass::Arterial { SPEED_ARTERIAL } else { SPEED_LOCAL };
        let (mut zone_node, mut zone_acc) = (vec![], vec![]);
        for (z, p) in plan.parcels.iter().enumerate() {
            let n = plan.access_node(z);
            zone_node.push(n);
            zone_acc.push(g.nodes[n as usize].distance(p.center) as f64 / SPEED_LOCAL);
        }
        Self {
            adj,
            ends: g.edges.iter().map(|e| (e.a, e.b)).collect(),
            len: g.edges.iter().map(|e| e.len as f64).collect(),
            t0: g.edges.iter().map(|e| e.len as f64 / speed(e.class)).collect(),
            cap: g.edges.iter().map(|e| if e.class == RoadClass::Arterial { CAP_ARTERIAL } else { CAP_LOCAL }).collect(),
            flow: vec![0.0; g.edges.len()],
            closed: vec![0.0; g.edges.len()],
            zone_node,
            zone_acc,
            access: plan.parcels.iter().map(|p| p.access).collect(),
            commercial: plan.parcels.iter().map(|p| p.kind == ZoneKind::Commercial).collect(),
            out: TrafficOut { zone_share: vec![0.0; plan.parcels.len()], ..Default::default() },
        }
    }

    /// R24: set edge `e` to a lane class (capacity and free-flow time follow it).
    pub fn set_lane_class(&mut self, e: usize, c: crate::lanes::LaneClass) {
        self.cap[e] = c.capacity();
        self.t0[e] = self.len[e] / c.speed();
    }

    /// Level crossings: `closed[e]` = share of time edge `e` is shut while a train passes (missing = open).
    pub fn set_closure(&mut self, closed: &[f32]) {
        for (c, i) in self.closed.iter_mut().zip(0..) {
            *c = closed.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        }
    }

    /// Capacity of edge `e` after crossing closures: a crossing that is shut half the time passes half the cars.
    fn capacity(&self, e: usize) -> f64 {
        self.cap[e] * (1.0 - MAX_CLOSURE * self.closed[e] as f64)
    }

    /// Congested travel time over edge `e` (minutes): free flow * (1 + a (q/c)^b), capped at MAX_SLOWDOWN; cars also
    /// wait out the closed share of the time at a level crossing.
    pub fn edge_time(&self, e: usize) -> f64 {
        let wait = 1.0 / (1.0 - MAX_CLOSURE * self.closed[e] as f64);
        self.t0[e] * ((1.0 + BPR_ALPHA * (self.flow[e] / self.capacity(e)).powf(BPR_BETA)).min(MAX_SLOWDOWN)) * wait.min(MAX_SLOWDOWN)
    }

    /// Volume over capacity of edge `e`.
    pub fn vc(&self, e: usize) -> f64 {
        self.flow[e] / self.capacity(e)
    }

    /// Congestion slowdown factor (>= 1): congested time over free-flow time.
    pub fn slowdown(&self, e: usize) -> f64 {
        self.edge_time(e) / self.t0[e]
    }

    pub fn free_time(&self, e: usize) -> f64 {
        self.t0[e]
    }

    fn dijkstra(&self, built: &[bool], src: u32, dist: &mut [f64], pred: &mut [u32]) {
        dist.fill(f64::INFINITY);
        pred.fill(u32::MAX);
        let mut h = BinaryHeap::new();
        dist[src as usize] = 0.0;
        h.push(Reverse((0f64.to_bits(), src)));
        while let Some(Reverse((bits, u))) = h.pop() {
            let du = f64::from_bits(bits);
            if du > dist[u as usize] {
                continue;
            }
            for &(v, e) in &self.adj[u as usize] {
                if !built[e as usize] {
                    continue;
                }
                let nd = du + self.edge_time(e as usize);
                if nd < dist[v as usize] {
                    dist[v as usize] = nd;
                    pred[v as usize] = e;
                    h.push(Reverse((nd.to_bits(), v)));
                }
            }
        }
    }

    /// Door-to-door car minutes between two zones over the current congested network (INFINITY if unreachable).
    pub fn car_time(&self, built: &[bool], a: usize, b: usize) -> f64 {
        let mut dist = vec![f64::INFINITY; self.adj.len()];
        let mut pred = vec![u32::MAX; self.adj.len()];
        self.dijkstra(built, self.zone_node[a], &mut dist, &mut pred);
        self.zone_acc[a] + dist[self.zone_node[b] as usize] + self.zone_acc[b] + PARK_MIN
    }

    /// Recompute demand, mode split and edge flows for the given hour. `transit` = door-to-door transit minutes
    /// (None = no transit anywhere).
    pub fn assign(&mut self, built: &[bool], zones: &Zones, transit: Option<&CostMatrix>, hour: f64) {
        let n = self.zone_node.len();
        let active: Vec<bool> = (0..n).map(|i| built[self.access[i]] && zones.0[i].pop >= 1.0).collect();
        let (mut dist, mut pred) = (vec![0.0; self.adj.len()], vec![0u32; self.adj.len()]);
        let mut raw = vec![0.0; self.flow.len()];
        let mut out = TrafficOut { zone_share: vec![0.0; n], ..Default::default() };
        let (mut time_sum, mut acc_num, mut acc_den) = (0.0, 0.0, 0.0);
        let (wp, sp) = (WORK_PER_DAY / 24.0 * work_profile(hour), SHOP_PER_DAY / 24.0 * shop_profile(hour));
        let (mut tcar, mut ww, mut ws) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
        let jobs_total: f64 = (0..n).filter(|&j| active[j]).map(|j| zones.0[j].jobs).sum();
        for i in (0..n).filter(|&i| active[i]) {
            self.dijkstra(built, self.zone_node[i], &mut dist, &mut pred);
            let (mut sw, mut ss) = (0.0, 0.0);
            for j in 0..n {
                tcar[j] = f64::INFINITY;
                ww[j] = 0.0;
                ws[j] = 0.0;
                if j == i || !active[j] || !dist[self.zone_node[j] as usize].is_finite() {
                    continue;
                }
                tcar[j] = self.zone_acc[i] + dist[self.zone_node[j] as usize] + self.zone_acc[j] + PARK_MIN;
                let a = (-GAMMA * tcar[j]).exp() * zones.0[j].jobs;
                ww[j] = a;
                ws[j] = a * if self.commercial[j] { 3.0 } else { 1.0 };
                sw += ww[j];
                ss += ws[j];
            }
            let (prod_w, prod_s) = (zones.0[i].pop * wp, zones.0[i].pop * sp);
            let (mut dem_i, mut tran_i, mut reach) = (0.0, 0.0, 0.0);
            for j in 0..n {
                if !tcar[j].is_finite() {
                    continue;
                }
                let d = if sw > 0.0 { prod_w * ww[j] / sw } else { 0.0 } + if ss > 0.0 { prod_s * ws[j] / ss } else { 0.0 };
                let tt = transit.map(|m| m.get(i, j)).filter(|t| t.is_finite());
                let p = tt.map_or(0.0, |t| p_transit(tcar[j], t));
                let (car, tran) = (d * (1.0 - p), d * p);
                if car > 0.0 {
                    let mut node = self.zone_node[j];
                    while node != self.zone_node[i] {
                        let e = pred[node as usize] as usize;
                        raw[e] += car;
                        node = if self.ends[e].0 == node { self.ends[e].1 } else { self.ends[e].0 };
                    }
                }
                out.car_trips += car;
                out.transit_trips += tran;
                dem_i += d;
                tran_i += tran;
                time_sum += d * (p * tt.unwrap_or(0.0) + (1.0 - p) * tcar[j]);
                if tcar[j].min(tt.unwrap_or(f64::INFINITY)) <= ACCESS_HORIZON {
                    reach += zones.0[j].jobs;
                }
            }
            if dem_i > 0.0 {
                out.zone_share[i] = tran_i / dem_i;
            }
            acc_num += zones.0[i].pop * if jobs_total > 0.0 { reach / jobs_total } else { 0.0 };
            acc_den += zones.0[i].pop;
        }
        for e in 0..self.flow.len() {
            self.flow[e] += RHO * (raw[e] - self.flow[e]);
            out.car_vkt += raw[e] * self.len[e];
        }
        let trips = out.car_trips + out.transit_trips;
        out.mean_trip_time = if trips > 0.0 { time_sum / trips } else { 0.0 };
        out.accessibility = if acc_den > 0.0 { acc_num / acc_den } else { 0.0 };
        let shares: Vec<f64> = (0..n).filter(|&i| active[i]).map(|i| out.zone_share[i]).collect();
        out.equity = if shares.len() > 1 {
            let mean = shares.iter().sum::<f64>() / shares.len() as f64;
            let sd = (shares.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / shares.len() as f64).sqrt();
            (1.0 - 2.0 * sd).clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.out = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roadgen::plan_towns;
    use growth::{Zone, ZoneId};
    use world_gen::generate_cities;

    fn setup(fill: f64) -> (TownPlan, Zones, Vec<bool>) {
        let plan = plan_towns(7, &generate_cities(7, 3)[..2]);
        let zones = Zones::new(
            plan.parcels
                .iter()
                .enumerate()
                .map(|(i, p)| Zone { id: ZoneId(i as u32), pop: p.capacity * fill, jobs: p.capacity * fill * 0.4, capacity: p.capacity, crowding: 0.0 })
                .collect(),
        );
        let built = vec![true; plan.graph.edges.len()];
        (plan, zones, built)
    }

    fn settle(t: &mut Traffic, built: &[bool], z: &Zones, tr: Option<&CostMatrix>) {
        for _ in 0..12 {
            t.assign(built, z, tr, 8.0);
        }
    }

    #[test]
    fn congestion_raises_car_travel_time() {
        let (plan, z_light, built) = setup(0.02);
        let (_, z_heavy, _) = setup(0.8);
        let (mut light, mut heavy) = (Traffic::new(&plan), Traffic::new(&plan));
        settle(&mut light, &built, &z_light, None);
        settle(&mut heavy, &built, &z_heavy, None);
        assert!(heavy.out.mean_trip_time > light.out.mean_trip_time * 1.15, "{} vs {}", heavy.out.mean_trip_time, light.out.mean_trip_time);
        let busiest = (0..heavy.flow.len()).max_by(|&a, &b| heavy.vc(a).total_cmp(&heavy.vc(b))).unwrap();
        assert!(heavy.vc(busiest) > 0.5, "peak v/c only {}", heavy.vc(busiest));
        assert!(heavy.edge_time(busiest) > heavy.free_time(busiest) * 1.05);
        // the same pair of zones takes longer on the loaded network
        let (a, b) = (0, plan.parcels.iter().rposition(|p| p.town == 0).unwrap());
        assert!(heavy.car_time(&built, a, b) > light.car_time(&built, a, b));
        // rush hour is worse than 03:00
        let mut night = Traffic::new(&plan);
        for _ in 0..12 {
            night.assign(&built, &z_heavy, None, 3.0);
        }
        assert!(night.out.mean_trip_time < heavy.out.mean_trip_time);
    }

    #[test]
    fn widening_a_road_cuts_its_travel_time_and_congestion() {
        let (plan, z, built) = setup(0.6);
        let mut t = Traffic::new(&plan);
        settle(&mut t, &built, &z, None);
        let busiest = (0..t.flow.len()).max_by(|&a, &b| t.flow[a].total_cmp(&t.flow[b])).unwrap();
        let (vc0, t0) = (t.vc(busiest), t.edge_time(busiest));
        t.set_lane_class(busiest, crate::lanes::LaneClass::SixLane);
        assert!(t.vc(busiest) < vc0 && t.edge_time(busiest) < t0);
    }

    #[test]
    fn a_level_crossing_closed_by_trains_slows_the_road_and_queues_cars() {
        let (plan, z, built) = setup(0.6);
        let (mut open, mut shut) = (Traffic::new(&plan), Traffic::new(&plan));
        settle(&mut open, &built, &z, None);
        let busiest = (0..open.flow.len()).max_by(|&a, &b| open.flow[a].total_cmp(&open.flow[b])).unwrap();
        let mut closed = vec![0.0; plan.graph.edges.len()];
        closed[busiest] = 0.5;
        shut.set_closure(&closed);
        // same load on the shut road: slower and more congested (assignment may later reroute cars away)
        shut.flow = open.flow.clone();
        assert!(shut.edge_time(busiest) > open.edge_time(busiest) * 1.5, "{} vs {}", shut.edge_time(busiest), open.edge_time(busiest));
        assert!(shut.slowdown(busiest) > open.slowdown(busiest));
        assert!(shut.vc(busiest) > open.vc(busiest));
        // once cars re-assign they shed the shut road
        settle(&mut shut, &built, &z, None);
        assert!(shut.flow[busiest] < open.flow[busiest], "{} vs {}", shut.flow[busiest], open.flow[busiest]);
        // an edge with no crossing is untouched by the closure of another
        let quiet = (0..open.flow.len()).find(|&e| e != busiest && shut.closed[e] == 0.0).unwrap();
        assert_eq!(shut.closed[quiet], 0.0);
        // a bridge never closes anything: no closure => identical to open
        let mut bridged = Traffic::new(&plan);
        bridged.set_closure(&[]);
        settle(&mut bridged, &built, &z, None);
        assert_eq!(bridged.flow, open.flow);
    }

    fn fast_transit(n: usize, rows: &[usize], minutes: f64) -> CostMatrix {
        let mut m = CostMatrix::new(n);
        for &i in rows {
            for j in 0..n {
                if i != j {
                    m.set(i, j, minutes);
                    m.set(j, i, minutes);
                }
            }
        }
        m
    }

    #[test]
    fn better_transit_lowers_car_share() {
        let (plan, z, built) = setup(0.5);
        let n = z.0.len();
        let all: Vec<usize> = (0..n).collect();
        let mut shares = vec![];
        for tr in [None, Some(fast_transit(n, &all, 60.0)), Some(fast_transit(n, &all, 25.0)), Some(fast_transit(n, &all, 8.0))] {
            let mut t = Traffic::new(&plan);
            settle(&mut t, &built, &z, tr.as_ref());
            shares.push(t.out.transit_share());
        }
        assert_eq!(shares[0], 0.0);
        assert!(shares.windows(2).all(|w| w[1] > w[0]), "transit share should rise with service: {shares:?}");
    }

    #[test]
    fn a_line_serving_a_busy_origin_takes_cars_off_the_road() {
        let (plan, z, built) = setup(0.5);
        let n = z.0.len();
        let busy = (0..n).max_by(|&a, &b| z.0[a].pop.total_cmp(&z.0[b].pop).then(b.cmp(&a))).unwrap();
        let mut base = Traffic::new(&plan);
        settle(&mut base, &built, &z, None);
        let tr = fast_transit(n, &[busy], 6.0);
        let mut served = Traffic::new(&plan);
        settle(&mut served, &built, &z, Some(&tr));
        assert!(served.out.zone_share[busy] > 0.5, "busy zone share {}", served.out.zone_share[busy]);
        assert!(served.out.car_trips < base.out.car_trips * 0.98, "{} !< {}", served.out.car_trips, base.out.car_trips);
        assert!(served.out.car_vkt < base.out.car_vkt, "car-km should fall");
        // roads near the busy zone empty out
        let e = plan.parcels[busy].access;
        assert!(served.flow[e] < base.flow[e], "frontage flow {} !< {}", served.flow[e], base.flow[e]);
    }

    #[test]
    fn unbuilt_roads_carry_nothing_and_runs_are_identical() {
        let (plan, z, _) = setup(0.5);
        let mut built = vec![false; plan.graph.edges.len()];
        for (i, e) in plan.graph.edges.iter().enumerate() {
            built[i] = e.order <= crate::roadgen::SEED_ORDER;
        }
        let run = || {
            let mut t = Traffic::new(&plan);
            settle(&mut t, &built, &z, None);
            (t.flow.clone(), t.out.clone())
        };
        let (f, o) = run();
        assert_eq!((f.clone(), o), run());
        assert!(f.iter().enumerate().all(|(i, q)| built[i] || *q == 0.0));
        assert!(f.iter().any(|q| *q > 0.0));
    }
}
