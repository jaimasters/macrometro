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

/// How a rail edge crosses a road (or water). Level crossings stop road traffic while a train passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossingKind { Level, Bridge, Underpass }

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crossing {
    pub kind: CrossingKind,
    /// Distance along the edge from `a`.
    pub at: f64,
    /// Crossed road edge in the city road graph (`u32::MAX` = water).
    pub road: u32,
}

/// Track counts: 1 = one-way a->b, 2 = one each way, 4 = two each way (expresses can overtake).
pub const MAX_TRACKS: u8 = 4;

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub a: NodeId,
    pub b: NodeId,
    pub len: f64,
    pub kind: EdgeKind,
    pub owner: AgencyId,
    pub tracks: u8,
    /// Speed limit on this segment (curves, level crossings); INFINITY = none.
    pub vcap: f64,
    pub crossings: Vec<Crossing>,
    /// One track used in both directions (single-track running with passing loops at stations). Cleared when a
    /// second track is added. `tracks == 1` without this is a one-way loop track.
    pub bidir: bool,
}

/// Price of laying one more track per unit length.
pub const TRACK_COST_PER_LEN: f64 = 1.5;

impl Edge {
    /// Cost to go to the next track count (1 -> 2 adds one track, 2 -> 4 adds two); None at the maximum.
    pub fn upgrade_cost(&self) -> Option<f64> {
        (self.tracks < MAX_TRACKS).then(|| self.len * TRACK_COST_PER_LEN * self.tracks as f64)
    }

    pub fn track_cost_between(net: &Network, a: NodeId, b: NodeId) -> Option<f64> {
        net.rail_edge(a, b).and_then(|i| net.edges[i].upgrade_cost())
    }

    pub fn new(a: NodeId, b: NodeId, len: f64, kind: EdgeKind, owner: AgencyId) -> Self {
        Self { a, b, len, kind, owner, tracks: 2, vcap: f64::INFINITY, crossings: vec![], bidir: false }
    }
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
    /// Platform count per node (missing = 1). Raised by `AddPlatform`; trains need a free platform (C1).
    pub platforms: Vec<u8>,
    next_line: u32,
}

/// Most platforms a station can have.
pub const MAX_PLATFORMS: u8 = 4;

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
    /// Extend a line past its terminus `from` to `to` over an existing rail edge (the new end becomes a stop).
    ExtendLine { line: LineId, from: NodeId, to: NodeId },
    /// One more platform at a station (up to MAX_PLATFORMS).
    AddPlatform(NodeId),
    /// A rail edge with explicit track count (1 = one-way a->b), speed cap and crossings.
    AddRail { a: NodeId, b: NodeId, len: f64, tracks: u8, vcap: f64, crossings: Vec<Crossing> },
    /// Single bidirectional track between two stations (the default for new lines).
    AddSingleRail { a: NodeId, b: NodeId, len: f64, vcap: f64, crossings: Vec<Crossing> },
    /// The rail edge between two stations: 1 -> 2 -> 4 tracks.
    UpgradeTracks { a: NodeId, b: NodeId },
    /// Replace a crossing's kind (level -> bridge/underpass).
    UpgradeCrossing { edge: usize, idx: usize, kind: CrossingKind },
    /// Rebuild an edge's curve: new length, speed cap and the crossings along the new path.
    SetCurve { edge: usize, len: f64, vcap: f64, crossings: Vec<Crossing> },
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
    /// `ExtendLine` from a stop that is not an end of the line.
    NotTerminus(NodeId),
    MaxPlatforms(NodeId),
    /// A line may only use a one-way (1-track) segment in its a->b direction, as part of a loop.
    WrongWay(NodeId, NodeId),
    /// A non-loop line needs every segment to carry traffic both ways (2+ tracks).
    OneWayNeedsLoop(usize),
    MaxTracks(usize),
    NoEdge(usize),
    BadTracks,
}

impl Line {
    /// A loop line starts and ends at the same stop; trains circulate one way.
    pub fn looped(&self) -> bool {
        self.stops.len() >= 3 && self.stops.first() == self.stops.last()
    }
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

    pub fn platform_count(&self, n: NodeId) -> u8 {
        self.platforms.get(n.0 as usize).copied().unwrap_or(1)
    }

    pub fn rail_edge(&self, a: NodeId, b: NodeId) -> Option<usize> {
        self.edges.iter().position(|e| e.kind == EdgeKind::Rail && ((e.a, e.b) == (a, b) || (e.a, e.b) == (b, a)))
    }

    /// A hop over a 1-track edge must follow its a->b direction and belong to a loop line.
    fn check_hop(&self, i: usize, from: NodeId, to: NodeId, looped: bool) -> Result<(), TransitError> {
        let e = &self.edges[i];
        if e.tracks > 1 || e.bidir {
            return Ok(());
        }
        if !looped {
            return Err(TransitError::OneWayNeedsLoop(i));
        }
        if (e.a, e.b) != (from, to) {
            return Err(TransitError::WrongWay(from, to));
        }
        Ok(())
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
                self.edges.push(Edge::new(*a, *b, *len, *kind, c.agency));
                Ok(Applied::Edge(self.edges.len() - 1))
            }
            TransitAction::AddLine { class, stops, alignment_mismatch } => {
                if stops.len() < 2 {
                    return Err(TransitError::TooFewStops);
                }
                let looped = stops.len() >= 3 && stops.first() == stops.last();
                let mut segments = vec![];
                for w in stops.windows(2) {
                    self.check(w[0])?;
                    self.check(w[1])?;
                    let i = self.rail_edge(w[0], w[1]).ok_or(TransitError::NoRailEdge(w[0], w[1]))?;
                    self.check_hop(i, w[0], w[1], looped)?;
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
            TransitAction::ExtendLine { line, from, to } => {
                self.check(*from)?;
                self.check(*to)?;
                let i = self.lines.iter().position(|l| l.id == *line).ok_or(TransitError::UnknownLine(*line))?;
                if self.lines[i].owner != c.agency {
                    return Err(TransitError::NotOwner);
                }
                let seg = self.rail_edge(*from, *to).ok_or(TransitError::NoRailEdge(*from, *to))?;
                self.check_hop(seg, *from, *to, false)?;
                let l = &mut self.lines[i];
                if l.looped() {
                    return Err(TransitError::NotTerminus(*from));
                }
                if l.stops.last() == Some(from) {
                    l.stops.push(*to);
                    l.segments.push(seg);
                } else if l.stops.first() == Some(from) {
                    l.stops.insert(0, *to);
                    l.segments.insert(0, seg);
                } else {
                    return Err(TransitError::NotTerminus(*from));
                }
                Ok(Applied::Line(*line))
            }
            TransitAction::AddPlatform(n) => {
                self.check(*n)?;
                let now = self.platform_count(*n);
                if now >= MAX_PLATFORMS {
                    return Err(TransitError::MaxPlatforms(*n));
                }
                if self.platforms.len() <= n.0 as usize {
                    self.platforms.resize(n.0 as usize + 1, 1);
                }
                self.platforms[n.0 as usize] = now + 1;
                Ok(Applied::Removed)
            }
            TransitAction::AddRail { a, b, len, tracks, vcap, crossings } => {
                self.check(*a)?;
                self.check(*b)?;
                if !len.is_finite() || *len <= 0.0 {
                    return Err(TransitError::BadLength);
                }
                if !matches!(tracks, 1 | 2 | 4) {
                    return Err(TransitError::BadTracks);
                }
                let mut e = Edge::new(*a, *b, *len, EdgeKind::Rail, c.agency);
                (e.tracks, e.vcap, e.crossings) = (*tracks, *vcap, crossings.clone());
                self.edges.push(e);
                Ok(Applied::Edge(self.edges.len() - 1))
            }
            TransitAction::AddSingleRail { a, b, len, vcap, crossings } => {
                self.check(*a)?;
                self.check(*b)?;
                if !len.is_finite() || *len <= 0.0 {
                    return Err(TransitError::BadLength);
                }
                let mut e = Edge::new(*a, *b, *len, EdgeKind::Rail, c.agency);
                (e.tracks, e.vcap, e.crossings, e.bidir) = (1, *vcap, crossings.clone(), true);
                self.edges.push(e);
                Ok(Applied::Edge(self.edges.len() - 1))
            }
            TransitAction::UpgradeTracks { a, b } => {
                let i = self.rail_edge(*a, *b).ok_or(TransitError::NoRailEdge(*a, *b))?;
                let e = &mut self.edges[i];
                if e.tracks >= MAX_TRACKS {
                    return Err(TransitError::MaxTracks(i));
                }
                e.tracks *= 2;
                e.bidir = false;
                Ok(Applied::Edge(i))
            }
            TransitAction::UpgradeCrossing { edge, idx, kind } => {
                let cr = self.edges.get_mut(*edge).and_then(|e| e.crossings.get_mut(*idx)).ok_or(TransitError::NoEdge(*edge))?;
                cr.kind = *kind;
                Ok(Applied::Edge(*edge))
            }
            TransitAction::SetCurve { edge, len, vcap, crossings } => {
                if !len.is_finite() || *len <= 0.0 {
                    return Err(TransitError::BadLength);
                }
                let e = self.edges.get_mut(*edge).filter(|e| e.kind == EdgeKind::Rail).ok_or(TransitError::NoEdge(*edge))?;
                (e.len, e.vcap, e.crossings) = (*len, *vcap, crossings.clone());
                Ok(Applied::Edge(*edge))
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
                        let (e1, e2) = (&self.edges[l.segments[i - 1]], &self.edges[l.segments[i]]);
                        let mut br = Edge::new(p, q, e1.len + e2.len, EdgeKind::Rail, c.agency);
                        br.tracks = e1.tracks.min(e2.tracks);
                        br.vcap = e1.vcap.min(e2.vcap);
                        bridges.push(br);
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
