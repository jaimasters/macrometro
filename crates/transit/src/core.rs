//! Pure-Rust transit core: rail+road graph, lines, RailClass mismatch penalty (D-9), zone cost matrix. No bevy.
//! Spec: docs/features/transit/SPEC.md; arch: aggregation.md.
use growth::CostMatrix;
use sim_core::AgencyId;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Rail specialisation a line converges to. All rail is one interchangeable system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RailClass { Tram, LightRail, HeavyRail, HighSpeed, DoubleDecker }

/// (cruise speed, ideal stop spacing, capacity) in graph units (len / time, trips/period). Provisional tuning.
pub fn class_profile(c: RailClass) -> (f64, f64, f64) {
    match c {
        RailClass::Tram => (20.0, 0.4, 150.0),
        RailClass::LightRail => (35.0, 0.8, 300.0),
        RailClass::HeavyRail => (60.0, 2.0, 800.0),
        RailClass::HighSpeed => (150.0, 20.0, 600.0),
        RailClass::DoubleDecker => (45.0, 1.2, 1000.0),
    }
}

/// D-9 provisional: worst-case mismatch costs this fraction of effective speed/capacity.
pub const MAX_MISMATCH_PENALTY: f64 = 0.30;

/// Mismatch penalty in 0..=MAX_MISMATCH_PENALTY. `spacing` = mean stop spacing; `alignment_mismatch` 0 (aligned with the
/// demand corridor) ..= 1 (fully misaligned). Spacing mismatch = |ln(spacing/ideal)| saturating at a 4x ratio.
pub fn mismatch_penalty(class: RailClass, spacing: f64, alignment_mismatch: f64) -> f64 {
    let ideal = class_profile(class).1;
    let sm = if spacing > 0.0 { ((spacing / ideal).ln().abs() / 4f64.ln()).min(1.0) } else { 1.0 };
    MAX_MISMATCH_PENALTY * 0.5 * (sm + alignment_mismatch.clamp(0.0, 1.0))
}

pub fn effective_speed(class: RailClass, penalty: f64) -> f64 {
    class_profile(class).0 * (1.0 - penalty)
}

pub fn effective_capacity(class: RailClass, penalty: f64) -> f64 {
    class_profile(class).2 * (1.0 - penalty)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind { Road, Rail }

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Edge {
    pub a: NodeId,
    pub b: NodeId,
    pub len: f64,
    pub kind: EdgeKind,
    pub owner: AgencyId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub id: LineId,
    pub class: RailClass,
    pub stops: Vec<NodeId>,
    /// 0 = aligned with the demand corridor .. 1 = fully misaligned.
    pub alignment_mismatch: f64,
    pub owner: AgencyId,
    /// Indices into Network::edges of the rail edges between consecutive stops.
    pub segments: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetParams {
    pub road_speed: f64,
    /// Mode-choice steepness: p_transit = 1/(1+exp(mode_beta*(t_transit - t_road))).
    pub mode_beta: f64,
}

impl Default for NetParams {
    fn default() -> Self {
        Self { road_speed: 30.0, mode_beta: 5.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Network {
    pub params: NetParams,
    pub nodes: u32,
    pub edges: Vec<Edge>,
    pub lines: Vec<Line>,
    next_line: u32,
}

/// Player/agency command (D-8): always tagged with its issuing agency.
#[derive(Debug, Clone, PartialEq)]
pub struct TransitCommand {
    pub agency: AgencyId,
    pub action: TransitAction,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransitAction {
    AddNode,
    AddEdge { a: NodeId, b: NodeId, len: f64, kind: EdgeKind },
    AddLine { class: RailClass, stops: Vec<NodeId>, alignment_mismatch: f64 },
    RemoveLine(LineId),
    /// Bulldoze a station: drops every edge touching it and removes it from lines (a line left with <2 stops
    /// is removed; a mid-line removal is bridged by a new rail edge). Node ids stay stable (the node is orphaned).
    RemoveNode(NodeId),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Applied { Node(NodeId), Edge(usize), Line(LineId), Removed }

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransitError {
    UnknownNode(NodeId),
    BadLength,
    NoRailEdge(NodeId, NodeId),
    TooFewStops,
    UnknownLine(LineId),
    NotOwner,
}

impl Line {
    /// Mean stop spacing = mean segment length.
    pub fn spacing(&self, net: &Network) -> f64 {
        self.segments.iter().map(|&i| net.edges[i].len).sum::<f64>() / self.segments.len() as f64
    }
    pub fn penalty(&self, net: &Network) -> f64 {
        mismatch_penalty(self.class, self.spacing(net), self.alignment_mismatch)
    }
    pub fn speed(&self, net: &Network) -> f64 {
        effective_speed(self.class, self.penalty(net))
    }
}

impl Network {
    pub fn new(params: NetParams) -> Self {
        Self { params, ..Default::default() }
    }

    fn check(&self, n: NodeId) -> Result<(), TransitError> {
        if n.0 < self.nodes { Ok(()) } else { Err(TransitError::UnknownNode(n)) }
    }

    pub fn apply(&mut self, c: &TransitCommand) -> Result<Applied, TransitError> {
        match &c.action {
            TransitAction::AddNode => {
                self.nodes += 1;
                Ok(Applied::Node(NodeId(self.nodes - 1)))
            }
            TransitAction::AddEdge { a, b, len, kind } => {
                self.check(*a)?;
                self.check(*b)?;
                if !len.is_finite() || *len <= 0.0 {
                    return Err(TransitError::BadLength);
                }
                self.edges.push(Edge { a: *a, b: *b, len: *len, kind: *kind, owner: c.agency });
                Ok(Applied::Edge(self.edges.len() - 1))
            }
            TransitAction::AddLine { class, stops, alignment_mismatch } => {
                if stops.len() < 2 {
                    return Err(TransitError::TooFewStops);
                }
                let mut segments = vec![];
                for w in stops.windows(2) {
                    self.check(w[0])?;
                    self.check(w[1])?;
                    let i = self
                        .edges
                        .iter()
                        .position(|e| e.kind == EdgeKind::Rail && ((e.a, e.b) == (w[0], w[1]) || (e.a, e.b) == (w[1], w[0])))
                        .ok_or(TransitError::NoRailEdge(w[0], w[1]))?;
                    segments.push(i);
                }
                let id = LineId(self.next_line);
                self.next_line += 1;
                self.lines.push(Line { id, class: *class, stops: stops.clone(), alignment_mismatch: *alignment_mismatch, owner: c.agency, segments });
                Ok(Applied::Line(id))
            }
            TransitAction::RemoveLine(id) => {
                let i = self.lines.iter().position(|l| l.id == *id).ok_or(TransitError::UnknownLine(*id))?;
                if self.lines[i].owner != c.agency {
                    return Err(TransitError::NotOwner);
                }
                self.lines.remove(i);
                Ok(Applied::Removed)
            }
            TransitAction::RemoveNode(n) => {
                self.check(*n)?;
                let touching = |e: &Edge| e.a == *n || e.b == *n;
                if self.edges.iter().any(|e| touching(e) && e.owner != c.agency) {
                    return Err(TransitError::NotOwner);
                }
                // Bridge mid-line gaps first so the line stays connected.
                let mut bridges = vec![];
                for l in self.lines.iter().filter(|l| l.owner == c.agency) {
                    if let Some(i) = l.stops.iter().position(|s| s == n).filter(|i| *i > 0 && *i + 1 < l.stops.len()) {
                        let (p, q) = (l.stops[i - 1], l.stops[i + 1]);
                        let len: f64 = l.segments[i - 1..=i].iter().map(|&s| self.edges[s].len).sum();
                        bridges.push(Edge { a: p, b: q, len, kind: EdgeKind::Rail, owner: c.agency });
                    }
                }
                self.edges.extend(bridges);
                self.edges.retain(|e| !touching(e));
                for l in &mut self.lines {
                    l.stops.retain(|s| s != n);
                }
                self.lines.retain(|l| l.stops.len() >= 2);
                let edges = &self.edges;
                for l in &mut self.lines {
                    l.segments = l
                        .stops
                        .windows(2)
                        .filter_map(|w| {
                            edges.iter().position(|e| {
                                e.kind == EdgeKind::Rail && ((e.a, e.b) == (w[0], w[1]) || (e.a, e.b) == (w[1], w[0]))
                            })
                        })
                        .collect();
                }
                Ok(Applied::Removed)
            }
        }
    }

    /// Travel time over edge i, or None if unusable (rail edge no line serves).
    fn edge_time(&self, i: usize, road: bool, rail: bool) -> Option<f64> {
        let e = &self.edges[i];
        match e.kind {
            EdgeKind::Road => road.then(|| e.len / self.params.road_speed),
            EdgeKind::Rail if rail => self
                .lines
                .iter()
                .filter(|l| l.segments.contains(&i))
                .map(|l| e.len / l.speed(self))
                .min_by(|x, y| x.total_cmp(y)),
            EdgeKind::Rail => None,
        }
    }

    fn adjacency(&self, road: bool, rail: bool) -> Vec<Vec<(u32, f64)>> {
        let mut adj = vec![vec![]; self.nodes as usize];
        for i in 0..self.edges.len() {
            if let Some(t) = self.edge_time(i, road, rail) {
                let e = &self.edges[i];
                adj[e.a.0 as usize].push((e.b.0, t));
                adj[e.b.0 as usize].push((e.a.0, t));
            }
        }
        adj
    }
}

/// Deterministic Dijkstra from `src`: heap key (cost bits, node) — non-negative f64 bits order like the values.
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
            let nd = du + w;
            if nd < d[v as usize] {
                d[v as usize] = nd;
                h.push(Reverse((nd.to_bits(), v)));
            }
        }
    }
    d
}

fn matrix(net: &Network, zone_nodes: &[NodeId], road: bool, rail: bool) -> CostMatrix {
    let adj = net.adjacency(road, rail);
    let rows: Vec<Vec<f64>> = zone_nodes.iter().map(|n| dijkstra(&adj, n.0)).collect();
    CostMatrix::from_fn(zone_nodes.len(), |i, j| if i == j { 0.0 } else { rows[i][zone_nodes[j].0 as usize] })
}

/// Zone-to-zone generalised cost C[i][j] (travel time of the best road/rail path), index i == zone i in ascending ZoneId
/// order, `zone_nodes[i]` = graph node of zone i. Compatible with growth::CostMatrix (INFINITY = unreachable, diag 0).
/// ponytail: full recompute per call (O(Z*E log V)); dirty-row incremental per aggregation.md when Z is large.
/// Transfers/waiting are not modelled yet.
pub fn cost_matrix(net: &Network, zone_nodes: &[NodeId]) -> CostMatrix {
    matrix(net, zone_nodes, true, true)
}

/// Per-zone transit share s_i (for growth::RoadAgency::observe): origin i's destination-weighted mean probability of
/// choosing rail over road, logit on the time difference. Zero if rail cannot reach a destination. `weights[j]` e.g. jobs.
pub fn transit_shares(net: &Network, zone_nodes: &[NodeId], weights: &[f64]) -> Vec<f64> {
    assert_eq!(zone_nodes.len(), weights.len());
    let tr = matrix(net, zone_nodes, false, true);
    let rd = matrix(net, zone_nodes, true, false);
    let n = zone_nodes.len();
    (0..n)
        .map(|i| {
            let (mut num, mut den) = (0.0, 0.0);
            for j in (0..n).filter(|&j| j != i) {
                let p = if tr.get(i, j).is_finite() {
                    1.0 / (1.0 + (net.params.mode_beta * (tr.get(i, j) - rd.get(i, j).min(1e6))).exp())
                } else {
                    0.0
                };
                num += weights[j] * p;
                den += weights[j];
            }
            if den > 0.0 { num / den } else { 0.0 }
        })
        .collect()
}
