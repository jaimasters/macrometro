//! Deterministic fixed-step train sim (C1). Pure f64, no bevy, no RNG: one `step` = one `DT`, trains update in id order
//! against a snapshot of the previous tick, so results are identical run to run.
//!
//! Each line is a 1-D route. A train's `p` is the distance its *front* has travelled in its current direction (so the
//! terminus is always `p = L` and a reversal restarts at `p = length`). Every stop is a stop, so a train only ever
//! has one segment ahead of it: it accelerates from the platform, cruises, brakes to stop exactly on the platform
//! mark, and holds before the station (the "throat") until a platform slot is free.
use crate::core::*;
use std::collections::BTreeMap;

/// Fixed step (seconds): the app's FixedUpdate rate.
pub const DT: f64 = 1.0 / 64.0;
/// Bumper-to-bumper clearance kept behind the train ahead.
pub const MIN_GAP: f64 = 1.0;
/// Stop tolerance on the platform mark.
pub const STOP_EPS: f64 = 0.02;
/// Braking is planned at this share of the real brake rate so the discrete step never overshoots.
const PLAN: f64 = 0.9;
/// Gap between successive departures from the first stop (seconds).
pub const HEADWAY: f64 = 40.0;
const MAX_TRAINS: usize = 8;

/// Tunable table per train type (world units, seconds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrainType {
    pub vmax: f64,
    pub accel: f64,
    pub brake: f64,
    pub length: f64,
    pub dwell: f64,
}

pub fn train_type(c: RailClass) -> TrainType {
    let t = |vmax, accel, brake, length, dwell| TrainType { vmax, accel, brake, length, dwell };
    match c {
        RailClass::Tram => t(8.0, 2.5, 3.5, 4.0, 3.0),
        RailClass::LightRail => t(14.0, 3.0, 4.0, 6.0, 4.0),
        RailClass::HeavyRail => t(20.0, 3.0, 4.5, 10.0, 5.0),
        RailClass::HighSpeed => t(50.0, 2.5, 4.0, 12.0, 6.0),
        RailClass::DoubleDecker => t(16.0, 3.0, 4.0, 8.0, 5.0),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    /// Moving, or holding at the throat when `v == 0` and no slot.
    Run,
    /// Stopped on its platform; ticks left.
    Dwell(u32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Train {
    pub id: u32,
    pub line: LineId,
    pub route: usize,
    /// Front position, distance travelled in direction `dir` (+1 = stops order, -1 = reverse).
    pub p: f64,
    pub prev_p: f64,
    pub v: f64,
    pub dir: i8,
    /// 0 or 1; 1 only means anything on 4-track segments.
    pub lane: u8,
    /// Travel-order index of the stop being approached (1..=n).
    pub k: usize,
    pub phase: Phase,
    /// Platform slot held: (station, slot index).
    pub slot: Option<(NodeId, u8)>,
    pub len: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct TSeg {
    edge: usize,
    /// Travelling a -> b on that edge.
    fwd: bool,
    start: f64,
    len: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct Route {
    line: LineId,
    class: RailClass,
    stops: Vec<NodeId>,
    edges: Vec<usize>,
    looped: bool,
    total: f64,
    /// Per direction: segments and stop marks in travel coordinates.
    tsegs: [Vec<TSeg>; 2],
    marks: [Vec<f64>; 2],
    spawned: usize,
    since_spawn: u32,
}

impl Route {
    fn new(net: &Network, l: &Line) -> Self {
        let n = l.segments.len();
        let lens: Vec<f64> = l.segments.iter().map(|&i| net.edges[i].len).collect();
        let fwd: Vec<bool> = (0..n).map(|j| net.edges[l.segments[j]].a == l.stops[j]).collect();
        let total: f64 = lens.iter().sum();
        let mut cum = vec![0.0];
        for x in &lens {
            cum.push(cum.last().unwrap() + x);
        }
        let f: Vec<TSeg> = (0..n).map(|j| TSeg { edge: l.segments[j], fwd: fwd[j], start: cum[j], len: lens[j] }).collect();
        let b: Vec<TSeg> =
            (0..n).map(|k| { let j = n - 1 - k; TSeg { edge: l.segments[j], fwd: !fwd[j], start: total - cum[j + 1], len: lens[j] } }).collect();
        let mb: Vec<f64> = (0..=n).map(|k| total - cum[n - k]).collect();
        Self {
            line: l.id,
            class: l.class,
            stops: l.stops.clone(),
            edges: l.segments.clone(),
            looped: l.looped(),
            total,
            tsegs: [f, b],
            marks: [cum, mb],
            spawned: 0,
            since_spawn: u32::MAX / 2,
        }
    }
    fn d(dir: i8) -> usize {
        (dir < 0) as usize
    }
    fn stop_node(&self, dir: i8, k: usize) -> NodeId {
        if dir > 0 { self.stops[k] } else { self.stops[self.stops.len() - 1 - k] }
    }
    fn wants(&self) -> usize {
        let t = train_type(self.class);
        let trip = self.total / (0.7 * t.vmax) + self.edges.len() as f64 * t.dwell;
        let trip = if self.looped { trip } else { 2.0 * trip };
        ((trip / HEADWAY).ceil() as usize).clamp(1, MAX_TRAINS)
    }
}

/// The train world. `sync` + `step` are the only mutators.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrainSim {
    routes: Vec<Route>,
    pub trains: Vec<Train>,
    /// Platform slots per station node: holder train id.
    slots: BTreeMap<u32, Vec<Option<u32>>>,
    next_id: u32,
    pub tick: u64,
    /// Ticks each road edge spent closed by a train on a level crossing, and the ticks counted, since the last `take_closure`.
    closed: BTreeMap<u32, u64>,
    counted: u64,
}

impl TrainSim {
    /// Rebuild routes when lines changed; a line whose stops/segments changed loses its fleet (respawned).
    pub fn sync(&mut self, net: &Network) {
        let same = self.routes.len() == net.lines.len()
            && self.routes.iter().zip(&net.lines).all(|(r, l)| r.line == l.id && r.stops == l.stops && r.edges == l.segments);
        if same {
            return;
        }
        let old = std::mem::take(&mut self.routes);
        self.routes = net.lines.iter().map(|l| Route::new(net, l)).collect();
        let map: Vec<Option<usize>> = old
            .iter()
            .map(|o| self.routes.iter().position(|r| r.line == o.line && r.stops == o.stops && r.edges == o.edges))
            .collect();
        for (oi, ni) in map.iter().enumerate() {
            if let Some(ni) = ni {
                (self.routes[*ni].spawned, self.routes[*ni].since_spawn) = (old[oi].spawned, old[oi].since_spawn);
            }
        }
        self.trains.retain_mut(|t| match map[t.route] {
            Some(n) => {
                t.route = n;
                true
            }
            None => false,
        });
        let live: Vec<u32> = self.trains.iter().map(|t| t.id).collect();
        for s in self.slots.values_mut() {
            for x in s.iter_mut() {
                if x.is_some_and(|id| !live.contains(&id)) {
                    *x = None;
                }
            }
        }
    }

    fn free_slot(&mut self, net: &Network, n: NodeId, id: u32) -> Option<u8> {
        let s = self.slots.entry(n.0).or_default();
        s.resize(net.platform_count(n) as usize, None);
        let i = s.iter().position(|x| x.is_none())?;
        s[i] = Some(id);
        Some(i as u8)
    }

    fn release(&mut self, t: &mut Train) {
        if let Some((n, i)) = t.slot.take() {
            self.slots.get_mut(&n.0).unwrap()[i as usize] = None;
        }
    }

    pub fn step(&mut self, net: &Network) {
        self.sync(net);
        self.tick += 1;
        self.counted += 1;
        let snap = self.trains.clone();
        let mut trains = std::mem::take(&mut self.trains);
        for t in trains.iter_mut() {
            t.prev_p = t.p;
            self.update(net, &snap, t);
        }
        self.trains = trains;
        self.spawn(net);
        for (road, _) in self.level_crossings_occupied(net) {
            *self.closed.entry(road).or_default() += 1;
        }
    }

    fn spawn(&mut self, net: &Network) {
        for ri in 0..self.routes.len() {
            let r = &mut self.routes[ri];
            r.since_spawn = r.since_spawn.saturating_add(1);
            if r.spawned >= r.wants() || (r.since_spawn as f64) * DT < HEADWAY {
                continue;
            }
            let (line, class, node) = (r.line, r.class, r.stops[0]);
            let tt = train_type(class);
            let id = self.next_id;
            let mut t = Train {
                id,
                line,
                route: ri,
                p: 0.0,
                prev_p: 0.0,
                v: 0.0,
                dir: 1,
                lane: 0,
                k: 0,
                phase: Phase::Dwell((tt.dwell / DT).ceil() as u32),
                slot: None,
                len: tt.length,
            };
            // never appear on top of a train that is still clearing the platform
            if self.leader(net, &self.trains, &t).is_some_and(|(g, _)| g < MIN_GAP) {
                continue;
            }
            let Some(slot) = self.free_slot(net, node, id) else { continue };
            t.slot = Some((node, slot));
            self.next_id += 1;
            let r = &mut self.routes[ri];
            r.spawned += 1;
            r.since_spawn = 0;
            self.trains.push(t);
        }
    }

    /// Edge a train's front is on.
    fn edge_under(&self, t: &Train) -> usize {
        let segs = &self.routes[t.route].tsegs[Route::d(t.dir)];
        segs[segs.partition_point(|s| s.start <= t.p).saturating_sub(1).min(segs.len() - 1)].edge
    }

    /// Nearest conflicting leader of train `t` as (bumper gap, leader speed).
    fn leader(&self, net: &Network, snap: &[Train], t: &Train) -> Option<(f64, f64)> {
        let r = &self.routes[t.route];
        let node = r.stop_node(t.dir, t.k);
        let mut best: Option<(f64, f64)> = None;
        for o in snap.iter().filter(|o| o.id != t.id) {
            // leader on a different platform track of the station we are entering: side by side, no conflict
            if let (Some((n1, s1)), Some((n2, s2))) = (t.slot, o.slot) {
                let o_in_zone = o.phase != Phase::Run || o_rem(self, o) <= o.len + MIN_GAP;
                if n1 == n2 && s1 != s2 && n1 == node && o_in_zone {
                    continue;
                }
            }
            let gap = if o.route == t.route {
                if r.looped {
                    (o.p - t.p).rem_euclid(r.total) - o.len
                } else {
                    if o.dir != t.dir {
                        continue;
                    }
                    let d = o.p - t.p;
                    if d <= 0.0 {
                        continue;
                    }
                    d - o.len
                }
            } else {
                // different line: only conflicts on a shared edge travelled the same way
                let orr = &self.routes[o.route];
                let os = &orr.tsegs[Route::d(o.dir)];
                let ok = os.partition_point(|s| s.start <= o.p).saturating_sub(1).min(os.len() - 1);
                let tseg = &r.tsegs[Route::d(t.dir)];
                match tseg.iter().find(|s| s.edge == os[ok].edge && s.fwd == os[ok].fwd && s.start + s.len > t.p) {
                    Some(s) => {
                        let d = s.start + (o.p - os[ok].start) - t.p;
                        if d <= 0.0 {
                            continue;
                        }
                        d - o.len
                    }
                    None => continue,
                }
            };
            if gap > 400.0 {
                continue;
            }
            // 4-track edge: trains in different lanes never conflict
            if !r.looped && t.lane != o.lane && net.edges[self.edge_under(o)].tracks >= 4 {
                continue;
            }
            if best.map_or(true, |b| gap < b.0) {
                best = Some((gap, o.v));
            }
        }
        best
    }

    /// Could `t` move to `lane` without cutting off the train ahead or behind in that lane?
    fn lane_clear(&self, net: &Network, snap: &[Train], t: &Train, lane: u8, b: f64) -> bool {
        let mut tc = t.clone();
        tc.lane = lane;
        let safe = |v: f64| v * v / (2.0 * b) + MIN_GAP + 2.0;
        if self.leader(net, snap, &tc).is_some_and(|(g, _)| g < safe(t.v)) {
            return false;
        }
        snap.iter().filter(|o| o.id != t.id).all(|o| self.leader(net, std::slice::from_ref(&tc), o).map_or(true, |(g, _)| g >= safe(o.v)))
    }

    fn update(&mut self, net: &Network, snap: &[Train], t: &mut Train) {
        let tt = train_type(self.routes[t.route].class);
        if let Phase::Dwell(left) = t.phase {
            if left > 1 {
                t.phase = Phase::Dwell(left - 1);
                return;
            }
            let (looped, n) = (self.routes[t.route].looped, self.routes[t.route].edges.len());
            if t.k < n {
                t.k += 1;
            } else if looped {
                (t.p, t.prev_p, t.k) = (0.0, 0.0, 1);
            } else {
                (t.dir, t.p, t.prev_p, t.k) = (-t.dir, tt.length, tt.length, 1);
            }
            self.release(t);
            t.phase = Phase::Run;
        }
        let (marks, node, seg_edge) = {
            let r = &self.routes[t.route];
            let d = Route::d(t.dir);
            (r.marks[d][t.k], r.stop_node(t.dir, t.k), r.tsegs[d][t.k - 1].edge)
        };
        let throat = tt.length + 3.0;
        let b = tt.brake * PLAN;
        let rem = marks - t.p;
        if t.slot.is_none() && rem - throat <= t.v * t.v / (2.0 * b) + 2.0 {
            if let Some(s) = self.free_slot(net, node, t.id) {
                t.slot = Some((node, s));
            }
        }
        let target = if t.slot.is_some() { rem } else { rem - throat };
        let mut vlim = tt.vmax.min(net.edges[seg_edge].vcap).min((2.0 * b * target.max(0.0)).sqrt());
        // lane change: overtake a slower leader on a 4-track segment, if the other lane is clear
        if net.edges[seg_edge].tracks >= 4 && t.v > 0.0 {
            if let Some((g, lv)) = self.leader(net, snap, t) {
                if g < tt.vmax * tt.vmax / (2.0 * b) && lv < tt.vmax * 0.8 && self.lane_clear(net, snap, t, 1 - t.lane, b) {
                    t.lane = 1 - t.lane;
                }
            }
        }
        let lead = self.leader(net, snap, t);
        if let Some((gap, _)) = lead {
            vlim = vlim.min((2.0 * b * (gap - MIN_GAP - t.v * DT).max(0.0)).sqrt());
        }
        t.v = if vlim > t.v { (t.v + tt.accel * DT).min(vlim) } else { (t.v - tt.brake * DT).max(vlim).max(0.0) };
        let step = t.v * DT;
        if target - step <= STOP_EPS {
            // reached the platform mark (or the throat hold point): stop exactly on it
            t.p += target.max(0.0);
            t.v = 0.0;
            if t.slot.is_some() {
                t.phase = Phase::Dwell((tt.dwell / DT).ceil() as u32);
            }
        } else {
            t.p += step;
        }
    }

    /// Level crossings currently closed by a train: (road edge, rail edge index).
    pub fn level_crossings_occupied(&self, net: &Network) -> Vec<(u32, usize)> {
        let mut out = vec![];
        for t in &self.trains {
            let r = &self.routes[t.route];
            for s in &r.tsegs[Route::d(t.dir)] {
                for c in net.edges[s.edge].crossings.iter().filter(|c| c.kind == CrossingKind::Level) {
                    let at = if s.fwd { c.at } else { s.len - c.at };
                    let cp = s.start + at;
                    if cp >= t.p - t.len - 1.0 && cp <= t.p + 8.0 {
                        out.push((c.road, s.edge));
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Fraction of ticks each road edge was closed since the last call (then reset).
    pub fn take_closure(&mut self) -> BTreeMap<u32, f32> {
        let n = self.counted.max(1) as f32;
        let out = std::mem::take(&mut self.closed).into_iter().map(|(r, c)| (r, c as f32 / n)).collect();
        self.counted = 0;
        out
    }

    /// World-space progress of a train along its line's route: (line, distance from stop 0, rendering interpolation prev).
    pub fn route_pos(&self, t: &Train, alpha: f64) -> f64 {
        let r = &self.routes[t.route];
        let p = t.prev_p + (t.p - t.prev_p) * alpha;
        if t.dir > 0 { p } else { r.total - p }
    }

    /// Total route length of `line` (None if the line has no route).
    pub fn line_total(&self, line: LineId) -> Option<f64> {
        self.routes.iter().find(|r| r.line == line).map(|r| r.total)
    }

    /// Track count of the segment the train is on.
    pub fn tracks_under(&self, net: &Network, t: &Train) -> u8 {
        net.edges[self.edge_under(t)].tracks
    }

    /// Platform slot (0-based) and station the train occupies while stopped.
    pub fn on_platform(&self, t: &Train) -> Option<(NodeId, u8)> {
        matches!(t.phase, Phase::Dwell(_)).then_some(t.slot).flatten()
    }
}

fn o_rem(sim: &TrainSim, o: &Train) -> f64 {
    let r = &sim.routes[o.route];
    r.marks[Route::d(o.dir)][o.k.min(r.edges.len())] - o.p
}

#[cfg(test)]
#[path = "trains_tests.rs"]
mod tests;
