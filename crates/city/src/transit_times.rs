//! Door-to-door transit travel time between zones over the player's rail network:
//! access (walk/feeder) + wait + ride (Dijkstra over line-served rail edges) + egress. Minutes.
use bevy::math::Vec2;
use growth::CostMatrix;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use transit::{EdgeKind, Network, NodeId};

/// Rail speed units -> world units per minute (LightRail 35 -> ~9.8 u/min).
pub const RAIL_SCALE: f64 = 0.28;
pub const WAIT_MIN: f64 = 4.0;
pub const ACCESS_SPEED: f64 = 1.6;
pub const MAX_ACCESS: f32 = 20.0;

#[derive(Debug, Clone, Copy)]
pub struct StationPos {
    pub node: NodeId,
    pub p: Vec2,
}

/// (zone-to-zone minutes, INFINITY = no transit trip; line capacity per hour for the crowding score).
pub fn transit_times(net: &Network, zones: &[Vec2], stations: &[StationPos]) -> (CostMatrix, f64) {
    let n = zones.len();
    let capacity: f64 = net.lines.iter().map(|l| transit::effective_capacity(l.class, l.penalty(net))).sum();
    let mut m = CostMatrix::new(n);
    let served: Vec<&StationPos> = stations.iter().filter(|s| net.lines.iter().any(|l| l.stops.contains(&s.node))).collect();
    if served.len() < 2 {
        return (m, capacity);
    }
    // rail adjacency by line-served edges
    let mut adj: Vec<Vec<(u32, f64)>> = vec![vec![]; net.nodes as usize];
    for (i, e) in net.edges.iter().enumerate() {
        if e.kind != EdgeKind::Rail {
            continue;
        }
        let t = net.lines.iter().filter(|l| l.segments.contains(&i)).map(|l| e.len / (l.speed(net) * RAIL_SCALE)).min_by(|a, b| a.total_cmp(b));
        if let Some(t) = t {
            adj[e.a.0 as usize].push((e.b.0, t));
            adj[e.b.0 as usize].push((e.a.0, t));
        }
    }
    let ride: Vec<Vec<f64>> = served.iter().map(|s| dijkstra(&adj, s.node.0)).collect();
    // nearest stations per zone
    let access: Vec<Vec<(usize, f64)>> = zones
        .iter()
        .map(|z| {
            let mut v: Vec<(usize, f64)> = served
                .iter()
                .enumerate()
                .filter(|(_, s)| s.p.distance(*z) <= MAX_ACCESS)
                .map(|(k, s)| (k, s.p.distance(*z) as f64 / ACCESS_SPEED))
                .collect();
            v.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
            v.truncate(3);
            v
        })
        .collect();
    for i in 0..n {
        for j in 0..n {
            if i == j {
                continue;
            }
            let mut best = f64::INFINITY;
            for &(s, a) in &access[i] {
                for &(t, b) in &access[j] {
                    let r = ride[s][served[t].node.0 as usize];
                    if s != t && r.is_finite() {
                        best = best.min(a + WAIT_MIN + r + b);
                    }
                }
            }
            if best.is_finite() {
                m.set(i, j, best);
            }
        }
    }
    (m, capacity)
}

fn dijkstra(adj: &[Vec<(u32, f64)>], src: u32) -> Vec<f64> {
    let mut d = vec![f64::INFINITY; adj.len()];
    let mut h = BinaryHeap::new();
    d[src as usize] = 0.0;
    h.push(Reverse((0f64.to_bits(), src)));
    while let Some(Reverse((bits, u))) = h.pop() {
        let du = f64::from_bits(bits);
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
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::AgencyId;
    use transit::{RailClass, TransitAction, TransitCommand};

    #[test]
    fn rail_trip_is_access_wait_ride_egress_and_far_zones_have_none() {
        let mut net = Network::default();
        let mut go = |a: TransitAction| net.apply(&TransitCommand { agency: AgencyId(1), action: a }).unwrap();
        go(TransitAction::AddNode);
        go(TransitAction::AddNode);
        go(TransitAction::AddEdge { a: NodeId(0), b: NodeId(1), len: 60.0, kind: EdgeKind::Rail });
        go(TransitAction::AddLine { class: RailClass::LightRail, stops: vec![NodeId(0), NodeId(1)], alignment_mismatch: 0.0 });
        let stations = [StationPos { node: NodeId(0), p: Vec2::ZERO }, StationPos { node: NodeId(1), p: Vec2::new(60.0, 0.0) }];
        let zones = [Vec2::new(2.0, 0.0), Vec2::new(58.0, 0.0), Vec2::new(200.0, 200.0)];
        let (m, cap) = transit_times(&net, &zones, &stations);
        let speed = net.lines[0].speed(&net) * RAIL_SCALE;
        let want = 2.0 / ACCESS_SPEED * 2.0 + WAIT_MIN + 60.0 / speed;
        assert!((m.get(0, 1) - want).abs() < 1e-9, "{} vs {want}", m.get(0, 1));
        assert!(m.get(0, 2).is_infinite() && m.get(2, 1).is_infinite());
        assert!(cap > 0.0);
        // no line, no transit
        let (none, cap0) = transit_times(&Network::default(), &zones, &stations);
        assert!(none.get(0, 1).is_infinite() && cap0 == 0.0);
    }
}
