//! Wires ui_tools commands into transit and economy, and draws the player's network as a flat transit map:
//! white stations with dark outlines, thick coloured octilinear lines, capsule trains.
use crate::camera::MapCam;
use crate::zones::{CityPlan, GrowthLink, ZoneLayout};
use city::placer::{merge_crossings, road_crossings, LEVEL_VCAP, crossing_upgrade_cost, place_track, Ctx, Placement, DEFAULT_RADIUS, RADII, RAIL_COST_PER_LEN};
use bevy::prelude::*;
use economy::{Category, CurrentBudget, Economy, SpendCommand, SpendQueue};
use sim_core::AgencyId;
use sound::{SoundEvent, SoundKind};
use std::collections::HashMap;
use transit::{Crossing, CrossingKind, Edge, EdgeKind, NodeId, RailClass, TransitAction, TransitCommand, TransitCommands, TransitNet};
use ui_tools::{octilinear, path_len, route, ActiveAgency, ToolAction, ToolCommand};

/// The local player's agency (agency 0 is the city).
pub const PLAYER: AgencyId = AgencyId(1);
const STATION_COST: f64 = 50.0;
/// Price of one extra platform (B2 station menu; C1 trains need platforms to enter).
/// Price of one more train on a line.
pub const TRAIN_COST: f64 = 60.0;
pub const SIDING_COST: f64 = 50.0;
pub const PLATFORM_COST: f64 = 40.0;

// Flat layer stack (world y): land 0 < ghosts 0.5 < zones 1 < lines 2 < preview 2.6 < station outline 3 < station 3.3 < train 4.5.
pub const LINE_COLOURS: [(f32, f32, f32); 5] =
    [(0.15, 0.45, 0.90), (0.88, 0.25, 0.30), (0.10, 0.62, 0.58), (0.90, 0.72, 0.10), (0.55, 0.35, 0.75)];

/// Colour of line `id` (cycles the palette).
pub fn line_colour(id: usize) -> Color {
    let (r, g, b) = LINE_COLOURS[id % LINE_COLOURS.len()];
    Color::srgb(r, g, b)
}

/// Grid cell -> transit node for player-placed stations. Zone nodes occupy 0..n, stations follow.
#[derive(Resource)]
pub struct StationMap {
    next: u32,
    pub by_cell: HashMap<IVec2, NodeId>,
}

impl StationMap {
    pub fn new(first_free: u32) -> Self {
        Self { next: first_free, by_cell: HashMap::new() }
    }

    /// Transit node of the station on `cell`, if any.
    pub fn node(&self, cell: IVec2) -> Option<NodeId> {
        self.by_cell.get(&cell).copied()
    }

    /// Player stations as (transit node, cell), ascending node id (deterministic).
    pub fn stations(&self) -> impl Iterator<Item = (NodeId, IVec2)> {
        let mut v: Vec<(NodeId, IVec2)> = self.by_cell.iter().map(|(c, n)| (*n, *c)).collect();
        v.sort_by_key(|s| s.0);
        v.into_iter()
    }
}

fn nearest_zone(centres: &[Vec2], p: Vec2) -> Option<usize> {
    (0..centres.len()).min_by(|&a, &b| centres[a].distance(p).total_cmp(&centres[b].distance(p)))
}

/// How one rail segment between two stations is laid: the placer's length, speed cap and crossings, measured from `from`.
#[derive(Clone, Debug, PartialEq)]
pub struct HopPlan {
    pub from: IVec2,
    pub len: f64,
    pub vcap: f64,
    pub crossings: Vec<Crossing>,
}

/// Segment plans by station pair (order-free key). `Some` = lay this; `None` = the segment already exists. A segment
/// with no entry is laid straight along the octilinear route (tests, legacy).
pub type Hops = HashMap<(IVec2, IVec2), Option<HopPlan>>;

/// Order-free key of a segment between two station cells.
pub fn hop_key(a: IVec2, b: IVec2) -> (IVec2, IVec2) {
    if (a.x, a.y) <= (b.x, b.y) { (a, b) } else { (b, a) }
}

/// Station-to-station polylines the placer laid (arcs), and each segment's curve-radius level (index into `RADII`),
/// for drawing, saving and later upgrades.
#[derive(Resource, Default)]
pub struct RailGeoms(pub HashMap<(IVec2, IVec2), Vec<Vec2>>, pub HashMap<(IVec2, IVec2), u8>);

/// The drawn path of a line over its stations (placer arcs where laid, octilinear otherwise) and the index in it of
/// each station.
pub fn line_geometry(cells: &[IVec2], geoms: &RailGeoms) -> (Vec<Vec2>, Vec<usize>) {
    let mut path: Vec<Vec2> = vec![];
    let mut idx = vec![];
    for w in cells.windows(2) {
        let key = hop_key(w[0], w[1]);
        let mut hop = match geoms.0.get(&key) {
            Some(g) if g.len() >= 2 => {
                let mut g = g.clone();
                if w[0] != key.0 {
                    g.reverse();
                }
                g
            }
            _ => route(&[w[0], w[1]]),
        };
        if path.is_empty() {
            idx.push(0);
        } else {
            hop.remove(0);
        }
        path.extend(hop);
        idx.push(path.len() - 1);
    }
    (path, idx)
}

/// A priced curve upgrade of one segment: the next radius up, re-laid through the same apex.
pub struct CurveUpgrade {
    pub placement: Placement,
    pub level: u8,
    /// Edge index in the transit net and whether its `a` end is `key.0`'s station.
    pub edge: usize,
    pub forward: bool,
    pub crossings: Vec<Crossing>,
    /// Charged: eminent domain for what the wider arc sweeps, extra track, and crossings it newly meets.
    pub cost: f64,
}

/// Price (and lay out) the next-radius upgrade of the segment a-b; None when maxed, unknown or unplaceable.
pub fn curve_upgrade(ctx: &Ctx, geoms: &RailGeoms, net: &transit::Network, map: &StationMap, a: IVec2, b: IVec2) -> Option<CurveUpgrade> {
    let key = hop_key(a, b);
    let pts = geoms.0.get(&key)?;
    let level = geoms.1.get(&key).copied().unwrap_or(0) + 1;
    let radius = *RADII.get(level as usize)?;
    let (na, nb) = (map.node(key.0)?, map.node(key.1)?);
    let edge = net.rail_edge(na, nb)?;
    let e = &net.edges[edge];
    let (p0, p1) = (key.0.as_vec2(), key.1.as_vec2());
    let chord = (p1 - p0).normalize_or_zero();
    // apex = the point of the old path furthest from the straight chord (None if it was straight)
    let dev = |q: &Vec2| (*q - p0).perp_dot(chord).abs();
    let apex = pts.iter().copied().max_by(|x, y| dev(x).total_cmp(&dev(y))).filter(|q| dev(q) > 0.5);
    let placement = place_track(ctx, p0, p1, apex, radius)?;
    let forward = e.a == na;
    // crossings already built keep their kind (a bridge stays a bridge); only newly met roads are charged
    let mut cost = placement.cost.eminent + (placement.len - e.len).max(0.0) * RAIL_COST_PER_LEN;
    let crossings: Vec<Crossing> = placement
        .crossings
        .iter()
        .zip(&placement.crossing_costs)
        .map(|(c, price)| {
            let old = (c.road != u32::MAX).then(|| e.crossings.iter().find(|o| o.road == c.road)).flatten();
            if old.is_none() {
                cost += price;
            }
            let at = if forward { c.at } else { placement.len - c.at };
            Crossing { kind: old.map_or(c.kind, |o| o.kind), at, road: c.road }
        })
        .collect();
    Some(CurveUpgrade { placement, level, edge, forward, crossings, cost })
}

/// Grade-separating every level crossing on segment a-b: (edge, [(crossing index, kind)], cost).
pub fn crossing_upgrades(net: &transit::Network, map: &StationMap, a: IVec2, b: IVec2, underpass: bool) -> Option<(usize, Vec<(usize, CrossingKind)>, f64)> {
    let edge = net.rail_edge(map.node(a)?, map.node(b)?)?;
    let to = if underpass { CrossingKind::Underpass } else { CrossingKind::Bridge };
    let ups: Vec<(usize, CrossingKind)> = net.edges[edge].crossings.iter().enumerate().filter(|(_, c)| c.kind == CrossingKind::Level).map(|(i, _)| (i, to)).collect();
    let cost = ups.len() as f64 * crossing_upgrade_cost(CrossingKind::Level, to)?;
    (!ups.is_empty()).then_some((edge, ups, cost))
}

/// Lane class per road edge the player has upgraded (absent = the generated default).
#[derive(Resource, Default)]
pub struct RoadClasses(pub HashMap<usize, city::lanes::LaneClass>);

/// Next-class widening of road `e`: lane + eminent-domain price and the zones it razes.
pub fn road_upgrade(ctx: &Ctx, classes: &RoadClasses, e: usize) -> Option<city::lanes::LaneUpgrade> {
    let ed = ctx.plan.graph.edges.get(e)?;
    let from = classes.0.get(&e).copied().unwrap_or_else(|| city::lanes::default_class(ed.class));
    let strip = [ctx.plan.graph.nodes[ed.a as usize], ctx.plan.graph.nodes[ed.b as usize]];
    city::lanes::upgrade(ctx.plan, ctx.pop, ctx.gone, &strip, ed.len, from)
}

/// A road built (by city growth) across track that is already laid needs a crossing too. Re-find the crossings of every
/// laid hop when the set of built roads changes and add the ones that are missing (existing kinds are kept).
fn reconcile_crossings(
    link: Res<GrowthLink>,
    cp: Res<CityPlan>,
    lines: Res<Lines>,
    geoms: Res<RailGeoms>,
    map: Res<StationMap>,
    net: Res<TransitNet>,
    mut transit: ResMut<TransitCommands>,
    mut seen: Local<usize>,
) {
    let built = &link.snapshots.latest().built;
    let n = built.iter().filter(|b| **b).count();
    if n == *seen {
        return;
    }
    *seen = n;
    let mut done: Vec<(IVec2, IVec2)> = vec![];
    for l in &lines.all {
        for w in l.cells.windows(2) {
            let key = hop_key(w[0], w[1]);
            if done.contains(&key) {
                continue;
            }
            done.push(key);
            let Some((na, nb)) = map.node(key.0).zip(map.node(key.1)) else { continue };
            let Some(i) = net.0.rail_edge(na, nb) else { continue };
            let e = &net.0.edges[i];
            let pts = match geoms.0.get(&key) {
                Some(g) if g.len() >= 2 => g.clone(),
                _ => route(&[key.0, key.1]),
            };
            let mut found = road_crossings(&cp.plan, built, &pts);
            if e.a != na {
                // geometry runs key.0 -> key.1, the edge from its own `a`
                for c in &mut found {
                    c.at = e.len - c.at;
                }
            }
            if let Some(all) = merge_crossings(&e.crossings, &found) {
                let vcap = if all.iter().any(|c| c.kind == CrossingKind::Level) { e.vcap.min(LEVEL_VCAP) } else { e.vcap };
                transit.0.push(TransitCommand { agency: PLAYER, action: TransitAction::SetCurve { edge: i, len: e.len, vcap, crossings: all } });
            }
        }
    }
}

/// Zones razed so far by eminent domain (indexed by zone).
#[derive(Resource, Default)]
pub struct Razed(pub Vec<bool>);

/// The rail command for hop c0 -> c1 (a, b are the stations' transit nodes).
fn rail_for(hops: &Hops, c: (IVec2, IVec2), a: NodeId, b: NodeId, agency: AgencyId) -> Option<TransitCommand> {
    let fallback = || {
        let len = path_len(&octilinear(c.0.as_vec2(), c.1.as_vec2())).max(1.0) as f64;
        Some(TransitCommand { agency, action: TransitAction::AddEdge { a, b, len, kind: EdgeKind::Rail } })
    };
    match hops.get(&hop_key(c.0, c.1)) {
        None => fallback(),
        Some(None) => None,
        Some(Some(h)) => {
            // crossings were measured from `h.from`; flip them if this line runs the other way
            let crossings = h.crossings.iter().map(|x| if h.from == c.0 { *x } else { Crossing { at: h.len - x.at, ..*x } }).collect();
            Some(TransitCommand { agency, action: TransitAction::AddSingleRail { a, b, len: h.len, vcap: h.vcap, crossings } })
        }
    }
}

pub fn to_transit(cmd: &ToolCommand, map: &mut StationMap, centres: &[Vec2]) -> (Vec<TransitCommand>, Vec<IVec2>) {
    to_transit_with(cmd, map, centres, &Hops::new())
}

/// Translate a player command into agency-tagged transit commands (D-8). A new station is a node
/// joined to its nearest zone by a road edge so rail between stations can change zone costs.
/// Returns the commands and the cells that became new stations.
pub fn to_transit_with(cmd: &ToolCommand, map: &mut StationMap, centres: &[Vec2], hops: &Hops) -> (Vec<TransitCommand>, Vec<IVec2>) {
    let (agency, mut out, mut fresh) = (cmd.agency, vec![], vec![]);
    // existing station an extend/platform command refers to (looked up before `node` borrows the map)
    let known = match &cmd.action {
        ToolAction::ExtendLine { from, .. } => map.by_cell.get(from).copied(),
        ToolAction::AddPlatform { cell } => map.by_cell.get(cell).copied(),
        _ => None,
    };
    let mut node = |cell: IVec2, out: &mut Vec<TransitCommand>, fresh: &mut Vec<IVec2>| -> NodeId {
        if let Some(n) = map.by_cell.get(&cell) {
            return *n;
        }
        let n = NodeId(map.next);
        map.next += 1;
        map.by_cell.insert(cell, n);
        fresh.push(cell);
        out.push(TransitCommand { agency, action: TransitAction::AddNode });
        // island platform by default: two platform tracks, so each station is a passing loop
        out.push(TransitCommand { agency, action: TransitAction::AddPlatform(n) });
        let p = cell.as_vec2();
        if let Some(z) = nearest_zone(centres, p) {
            let len = centres[z].distance(p).max(1.0) as f64;
            out.push(TransitCommand { agency, action: TransitAction::AddEdge { a: n, b: NodeId(z as u32), len, kind: EdgeKind::Road } });
        }
        n
    };
    match &cmd.action {
        ToolAction::DeleteStation { cell } => {
            if let Some(n) = map.by_cell.remove(cell) {
                out.push(TransitCommand { agency, action: TransitAction::RemoveNode(n) });
            }
        }
        ToolAction::PlaceStation { cell } => {
            node(*cell, &mut out, &mut fresh);
        }
        ToolAction::UpgradeTracks { a, b } => {
            if let (Some(na), Some(nb)) = (map.by_cell.get(a).copied(), map.by_cell.get(b).copied()) {
                out.push(TransitCommand { agency, action: TransitAction::UpgradeTracks { a: na, b: nb } });
            }
        }
        // upgrades that need the live net/city are applied in `apply_tool_commands`
        ToolAction::UpgradeCurve { .. } | ToolAction::UpgradeCrossings { .. } | ToolAction::UpgradeRoad { .. } | ToolAction::AddTrain { .. } | ToolAction::AddSiding { .. } => {}
        ToolAction::AddPlatform { .. } => {
            if let Some(n) = known {
                out.push(TransitCommand { agency, action: TransitAction::AddPlatform(n) });
            }
        }
        ToolAction::ExtendLine { line, from, to } => {
            if let Some(f) = known {
                let t = node(*to, &mut out, &mut fresh);
                out.extend(rail_for(hops, (*from, *to), f, t, agency));
                out.push(TransitCommand { agency, action: TransitAction::ExtendLine { line: transit::LineId(*line as u32), from: f, to: t } });
            }
        }
        ToolAction::DrawLine { cells } => {
            let stops: Vec<NodeId> = cells.iter().map(|&c| node(c, &mut out, &mut fresh)).collect();
            for (w, c) in stops.windows(2).zip(cells.windows(2)) {
                out.extend(rail_for(hops, (c[0], c[1]), w[0], w[1], agency));
            }
            out.push(TransitCommand { agency, action: TransitAction::AddLine { class: RailClass::LightRail, stops, alignment_mismatch: 0.0 } });
        }
    }
    (out, fresh)
}

/// Player lines as station cells (mirrors transit; used to redraw after deletions).
#[derive(Resource, Default)]
pub struct Lines {
    next: usize,
    pub all: Vec<LineRec>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LineRec {
    pub id: usize,
    pub cells: Vec<IVec2>,
}

impl Lines {
    pub fn add(&mut self, cells: Vec<IVec2>) -> usize {
        self.next += 1;
        self.all.push(LineRec { id: self.next - 1, cells });
        self.next - 1
    }

    /// Drop `cell` from every line; lines left with < 2 stops are removed.
    /// Returns ids of lines that changed (including removed ones).
    pub fn remove_station(&mut self, cell: IVec2) -> Vec<usize> {
        let mut changed = vec![];
        for l in &mut self.all {
            if l.cells.contains(&cell) {
                l.cells.retain(|c| *c != cell);
                changed.push(l.id);
            }
        }
        self.all.retain(|l| l.cells.len() >= 2);
        changed
    }

    /// Line ids that end (first or last cell) at `cell`.
    pub fn terminating_at(&self, cell: IVec2) -> Vec<usize> {
        self.all.iter().filter(|l| l.cells.first() == Some(&cell) || l.cells.last() == Some(&cell)).map(|l| l.id).collect()
    }

    /// Extend line `id` past its terminus `from` to `to`; false if `from` is not an end.
    pub fn extend(&mut self, id: usize, from: IVec2, to: IVec2) -> bool {
        let Some(l) = self.all.iter_mut().find(|l| l.id == id) else { return false };
        if l.cells.last() == Some(&from) {
            l.cells.push(to);
        } else if l.cells.first() == Some(&from) {
            l.cells.insert(0, to);
        } else {
            return false;
        }
        true
    }
}

pub struct ShellPlugin;

impl Plugin for ShellPlugin {
    fn build(&self, app: &mut App) {
        let first_free = app.world().get_resource::<crate::zones::CityPlan>().map_or(0, |c| c.node_count());
        if let Some(mut e) = app.world_mut().get_resource_mut::<Economy>() {
            e.0.add_agency(PLAYER, 0.5);
        }
        app.insert_resource(StationMap::new(first_free))
            .init_resource::<Lines>()
            .init_resource::<RailGeoms>()
            .init_resource::<RoadClasses>()
            .add_systems(Update, reconcile_crossings)
            .init_resource::<Razed>()
            .insert_resource(ActiveAgency(PLAYER))
            .add_systems(Update, apply_tool_commands);
    }
}

/// Where the player's drag bulges away from the straight line between two stations (the octilinear route's bend):
/// the placer goes round that side.
pub fn steer_point(a: IVec2, b: IVec2) -> Option<Vec2> {
    let (pa, pb) = (a.as_vec2(), b.as_vec2());
    let chord = pb - pa;
    let n = Vec2::new(-chord.y, chord.x).normalize_or_zero();
    let off = |p: &Vec2| (*p - pa).dot(n).abs();
    octilinear(pa, pb).into_iter().max_by(|x, y| off(x).total_cmp(&off(y))).filter(|p| off(p) > 2.0)
}

#[allow(clippy::too_many_arguments)]
/// Resources bundled so the system stays under Bevy's parameter limit.
#[derive(bevy::ecs::system::SystemParam)]
struct ExtraRes<'w> {
    classes: ResMut<'w, RoadClasses>,
    trains: ResMut<'w, crate::rails::Trains>,
}

fn apply_tool_commands(
    mut cmds: MessageReader<ToolCommand>,
    mut transit: ResMut<TransitCommands>,
    mut map: ResMut<StationMap>,
    mut lines: ResMut<Lines>,
    layout: Res<ZoneLayout>,
    cam: Option<Res<MapCam>>,
    net: Res<TransitNet>,
    cp: Res<CityPlan>,
    link: Res<GrowthLink>,
    seed: Res<world_gen::WorldSeed>,
    mut razed: ResMut<Razed>,
    mut geoms: ResMut<RailGeoms>,
    extra: ExtraRes,
    mut eco: ResMut<Economy>,
    mut spend: ResMut<SpendQueue>,
    mut sound: MessageWriter<SoundEvent>,
) {
    let ExtraRes { mut classes, mut trains } = extra;
    let centres: Vec<Vec2> = layout.0.iter().map(|g| g.center).collect();
    let pan = |p: Vec2| cam.as_ref().map_or(0.0, |c| ((p.x - c.focus.x) / c.view_h).clamp(-1.0, 1.0));
    for c in cmds.read() {
        match &c.action {
            ToolAction::AddSiding { cell } => {
                if let Some(n) = map.node(*cell) {
                    if trains.0.add_siding(n) {
                        spend.0.push(SpendCommand { agency: c.agency, category: Category::Construction, amount: SIDING_COST });
                    }
                }
                continue;
            }
            ToolAction::AddTrain { line } => {
                if trains.0.add_train(transit::LineId(*line as u32)) {
                    spend.0.push(SpendCommand { agency: c.agency, category: Category::Construction, amount: TRAIN_COST });
                }
                continue;
            }
            ToolAction::UpgradeRoad { edge } => {
                let snap = link.snapshots.latest();
                razed.0.resize(cp.plan.parcels.len(), false);
                let land = |p: Vec2| city::is_land(seed.0, p);
                let ctx = Ctx { plan: &cp.plan, built: &snap.built, pop: &snap.pop, gone: &razed.0, land: &land };
                let Some(u) = road_upgrade(&ctx, &classes, *edge) else { continue };
                for &z in &u.demolition.zones {
                    razed.0[z] = true;
                }
                link.shared.demolish.lock().unwrap().extend(u.demolition.zones.iter().copied());
                link.shared.lanes.lock().unwrap().push((*edge, u.to as u8));
                classes.0.insert(*edge, u.to);
                if let Some(ag) = eco.0.agencies.get_mut(&c.agency) {
                    ag.standing = (ag.standing - u.demolition.standing).max(0.0);
                }
                spend.0.push(SpendCommand { agency: c.agency, category: Category::Construction, amount: u.cost });
                continue;
            }
            ToolAction::UpgradeCrossings { a, b, underpass } => {
                if let Some((edge, ups, cost)) = crossing_upgrades(&net.0, &map, *a, *b, *underpass) {
                    for (idx, kind) in ups {
                        transit.0.push(TransitCommand { agency: c.agency, action: TransitAction::UpgradeCrossing { edge, idx, kind } });
                    }
                    spend.0.push(SpendCommand { agency: c.agency, category: Category::Construction, amount: cost });
                    sound.write(SoundEvent { kind: SoundKind::StationPlaced, pitch: a.x / 4 + a.y / 4 + 6, pan: pan(a.as_vec2()) });
                }
                continue;
            }
            ToolAction::UpgradeCurve { a, b } => {
                let snap = link.snapshots.latest();
                razed.0.resize(cp.plan.parcels.len(), false);
                let land = |p: Vec2| city::is_land(seed.0, p);
                let ctx = Ctx { plan: &cp.plan, built: &snap.built, pop: &snap.pop, gone: &razed.0, land: &land };
                let Some(u) = curve_upgrade(&ctx, &geoms, &net.0, &map, *a, *b) else { continue };
                let key = hop_key(*a, *b);
                transit.0.push(TransitCommand {
                    agency: c.agency,
                    action: TransitAction::SetCurve { edge: u.edge, len: u.placement.len.max(1.0), vcap: u.placement.vcap, crossings: u.crossings.clone() },
                });
                for &z in &u.placement.demolition.zones {
                    razed.0[z] = true;
                }
                link.shared.demolish.lock().unwrap().extend(u.placement.demolition.zones.iter().copied());
                if let Some(ag) = eco.0.agencies.get_mut(&c.agency) {
                    ag.standing = (ag.standing - u.placement.demolition.standing).max(0.0);
                }
                geoms.0.insert(key, u.placement.pts.clone());
                geoms.1.insert(key, u.level);
                spend.0.push(SpendCommand { agency: c.agency, category: Category::Construction, amount: u.cost });
                sound.write(SoundEvent { kind: SoundKind::StationPlaced, pitch: a.x / 4 + a.y / 4 + 5, pan: pan(a.as_vec2()) });
                continue;
            }
            _ => {}
        }
        // a track upgrade is priced from the segment itself; a maxed or unknown segment is refused outright
        let track_cost = if let ToolAction::UpgradeTracks { a, b } = &c.action {
            match map.node(*a).zip(map.node(*b)).and_then(|(na, nb)| Edge::track_cost_between(&net.0, na, nb)) {
                Some(cost) => cost,
                None => continue,
            }
        } else {
            0.0
        };
        // new segments are auto-routed (arcs, round obstacles, steered by the drag) and priced: track, crossings,
        // and eminent domain for whatever cannot be avoided
        let pairs: Vec<(IVec2, IVec2)> = match &c.action {
            ToolAction::DrawLine { cells } => cells.windows(2).map(|w| (w[0], w[1])).collect(),
            ToolAction::ExtendLine { from, to, .. } => vec![(*from, *to)],
            _ => vec![],
        };
        let (mut hops, mut hop_cost, mut standing_hit, mut razed_now) = (Hops::new(), 0.0, 0.0, vec![]);
        if !pairs.is_empty() {
            let snap = link.snapshots.latest();
            let n = cp.plan.parcels.len();
            razed.0.resize(n, false);
            let mut gone = razed.0.clone();
            let land = |p: Vec2| city::is_land(seed.0, p);
            let mut refused = false;
            for (c0, c1) in pairs {
                let key = hop_key(c0, c1);
                if hops.contains_key(&key) {
                    continue;
                }
                if map.node(c0).zip(map.node(c1)).and_then(|(a, b)| net.0.rail_edge(a, b)).is_some() {
                    hops.insert(key, None);
                    continue;
                }
                let ctx = Ctx { plan: &cp.plan, built: &snap.built, pop: &snap.pop, gone: &gone, land: &land };
                let Some(p) = place_track(&ctx, c0.as_vec2(), c1.as_vec2(), steer_point(c0, c1), DEFAULT_RADIUS) else {
                    refused = true;
                    break;
                };
                for &z in &p.demolition.zones {
                    gone[z] = true;
                    razed_now.push(z);
                }
                hop_cost += p.cost.total;
                standing_hit += p.demolition.standing;
                let mut pts = p.pts.clone();
                if c0 != key.0 {
                    pts.reverse();
                }
                geoms.0.insert(key, pts);
                geoms.1.remove(&key);
                hops.insert(key, Some(HopPlan { from: c0, len: p.len.max(1.0), vcap: p.vcap, crossings: p.crossings.clone() }));
            }
            if refused {
                warn!("track refused: too much water to bridge");
                continue;
            }
        }
        let (tc, fresh) = to_transit_with(c, &mut map, &centres, &hops);
        transit.0.extend(tc);
        if let ToolAction::DeleteStation { cell } = &c.action {
            lines.remove_station(*cell);
            continue;
        }
        if !razed_now.is_empty() {
            for &z in &razed_now {
                razed.0[z] = true;
            }
            link.shared.demolish.lock().unwrap().extend(razed_now);
        }
        if let Some(a) = eco.0.agencies.get_mut(&c.agency) {
            a.standing = (a.standing - standing_hit).max(0.0);
        }
        let platform = if matches!(c.action, ToolAction::AddPlatform { .. }) { PLATFORM_COST } else { 0.0 };
        let amount = STATION_COST * fresh.len() as f64 + hop_cost + platform + track_cost;
        if amount > 0.0 {
            spend.0.push(SpendCommand { agency: c.agency, category: Category::Construction, amount });
        }
        for cell in fresh {
            let p = cell.as_vec2();
            sound.write(SoundEvent { kind: SoundKind::StationPlaced, pitch: cell.x / 4 + cell.y / 4, pan: pan(p) });
        }
        if let ToolAction::AddPlatform { cell } = &c.action {
            sound.write(SoundEvent { kind: SoundKind::StationPlaced, pitch: cell.x / 4 + cell.y / 4 + 2, pan: pan(cell.as_vec2()) });
        }
        if let ToolAction::UpgradeTracks { a, .. } = &c.action {
            sound.write(SoundEvent { kind: SoundKind::StationPlaced, pitch: a.x / 4 + a.y / 4 + 4, pan: pan(a.as_vec2()) });
        }
        if let ToolAction::ExtendLine { line, from, to } = &c.action {
            if lines.extend(*line, *from, *to) {
                sound.write(SoundEvent { kind: SoundKind::LineCreated, pitch: *line as i32, pan: pan(to.as_vec2()) });
            }
        }
        if let ToolAction::DrawLine { cells } = &c.action {
            let id = lines.add(cells.clone());
            sound.write(SoundEvent { kind: SoundKind::LineCreated, pitch: id as i32, pan: pan(route(cells)[0]) });
        }
    }
}

/// (budget, spent, standing, city road reserve) for the HUD chips.
pub fn budget_summary(eco: &Economy, budget: &CurrentBudget) -> (f64, f64, f64, f64) {
    let (amount, reserve) = budget
        .0
        .as_ref()
        .map_or((0.0, 0.0), |b| (b.per_agency.get(&PLAYER).map_or(0.0, |a| a.amount), b.road_reserve));
    let (spent, standing) = eco
        .0
        .agencies
        .get(&PLAYER)
        .map_or((0.0, 0.0), |a| (a.spend.values().sum::<f64>(), a.standing));
    (amount, spent, standing, reserve)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(a: u32, action: ToolAction) -> ToolCommand {
        ToolCommand { agency: AgencyId(a), action }
    }

    #[test]
    fn upgrade_tracks_maps_cells_to_nodes_and_is_priced() {
        let centres = [Vec2::ZERO];
        let mut m = StationMap::new(1);
        let (a, b) = (IVec2::new(0, 12), IVec2::new(40, 12));
        let mut net = transit::Network::default();
        net.apply(&TransitCommand { agency: AgencyId(0), action: TransitAction::AddNode }).unwrap();
        let (v, _) = to_transit(&cmd(1, ToolAction::DrawLine { cells: vec![a, b] }), &mut m, &centres);
        for t in &v {
            net.apply(t).unwrap();
        }
        let (na, nb) = (m.node(a).unwrap(), m.node(b).unwrap());
        let before = Edge::track_cost_between(&net, na, nb).unwrap();
        let (up, _) = to_transit(&cmd(1, ToolAction::UpgradeTracks { a, b }), &mut m, &centres);
        assert_eq!(up[0].action, TransitAction::UpgradeTracks { a: na, b: nb });
        net.apply(&up[0]).unwrap();
        assert_eq!(net.edges.iter().find(|e| e.kind == EdgeKind::Rail).unwrap().tracks, 4);
        assert_eq!(Edge::track_cost_between(&net, na, nb), None, "maxed out: no further upgrade offered");
        assert!(before > 0.0);
    }

    #[test]
    fn deleting_station_updates_lines_and_transit() {
        let centres = [Vec2::ZERO];
        let mut m = StationMap::new(1);
        let (a, b, c3) = (IVec2::new(0, 12), IVec2::new(40, 12), IVec2::new(80, 12));
        let mut net = transit::Network::default();
        net.apply(&TransitCommand { agency: AgencyId(0), action: TransitAction::AddNode }).unwrap();
        let (v, _) = to_transit(&cmd(1, ToolAction::DrawLine { cells: vec![a, b] }), &mut m, &centres);
        let (v2, _) = to_transit(&cmd(1, ToolAction::DrawLine { cells: vec![b, c3] }), &mut m, &centres);
        for t in v.iter().chain(&v2) {
            net.apply(t).unwrap();
        }
        let mut lines = Lines::default();
        lines.add(vec![a, b]);
        lines.add(vec![b, c3]);
        // delete the shared middle station: both lines drop below 2 stops
        let (d, _) = to_transit(&cmd(1, ToolAction::DeleteStation { cell: b }), &mut m, &centres);
        assert_eq!(d.len(), 1);
        net.apply(&d[0]).unwrap();
        assert!(net.lines.is_empty());
        assert_eq!(lines.remove_station(b), vec![0, 1]);
        assert!(lines.all.is_empty());
        // deleting an unknown station emits nothing
        assert!(to_transit(&cmd(1, ToolAction::DeleteStation { cell: b }), &mut m, &centres).0.is_empty());
    }

    #[test]
    fn place_station_adds_node_and_link_once() {
        let centres = [Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)];
        let mut m = StationMap::new(2);
        let c = cmd(7, ToolAction::PlaceStation { cell: IVec2::new(90, 0) });
        let (v, fresh) = to_transit(&c, &mut m, &centres);
        assert_eq!(fresh, vec![IVec2::new(90, 0)]);
        assert_eq!(v.len(), 3, "node, island platform, road link");
        assert!(v.iter().all(|t| t.agency == AgencyId(7)));
        assert_eq!(v[2].action, TransitAction::AddEdge { a: NodeId(2), b: NodeId(1), len: 10.0, kind: EdgeKind::Road });
        let (again, fresh) = to_transit(&c, &mut m, &centres);
        assert!(again.is_empty() && fresh.is_empty());
    }

    #[test]
    fn draw_line_makes_rail_edge_then_line_with_valid_nodes() {
        let centres = [Vec2::ZERO];
        let mut m = StationMap::new(1);
        let c = cmd(1, ToolAction::DrawLine { cells: vec![IVec2::new(0, 12), IVec2::new(60, 40)] });
        let (v, fresh) = to_transit(&c, &mut m, &centres);
        assert_eq!(fresh.len(), 2);
        let mut net = transit::Network::default();
        net.apply(&TransitCommand { agency: AgencyId(0), action: TransitAction::AddNode }).unwrap();
        for t in &v {
            net.apply(t).unwrap_or_else(|e| panic!("{t:?}: {e:?}"));
        }
        assert_eq!(net.lines.len(), 1);
        assert_eq!(net.lines[0].stops.len(), 2);
    }

    #[test]
    fn extending_a_line_adds_station_rail_edge_and_stop() {
        let centres = [Vec2::ZERO];
        let mut m = StationMap::new(1);
        let (a, b, c3) = (IVec2::new(0, 12), IVec2::new(40, 12), IVec2::new(80, 12));
        let mut net = transit::Network::default();
        net.apply(&TransitCommand { agency: AgencyId(0), action: TransitAction::AddNode }).unwrap();
        let (v, _) = to_transit(&cmd(1, ToolAction::DrawLine { cells: vec![a, b] }), &mut m, &centres);
        for t in &v {
            net.apply(t).unwrap();
        }
        let (e, fresh) = to_transit(&cmd(1, ToolAction::ExtendLine { line: 0, from: b, to: c3 }), &mut m, &centres);
        assert_eq!(fresh, vec![c3]);
        for t in &e {
            net.apply(t).unwrap_or_else(|er| panic!("{t:?}: {er:?}"));
        }
        assert_eq!(net.lines[0].stops.len(), 3);
        // extending from a non-station emits nothing
        assert!(to_transit(&cmd(1, ToolAction::ExtendLine { line: 0, from: IVec2::new(4, 4), to: c3 }), &mut m, &centres).0.is_empty());
        let mut lines = Lines::default();
        lines.add(vec![a, b]);
        assert_eq!(lines.terminating_at(b), vec![0]);
        assert!(lines.terminating_at(IVec2::new(9, 9)).is_empty());
        assert!(lines.extend(0, b, c3) && !lines.extend(0, b, a), "only a terminus extends");
        assert_eq!(lines.all[0].cells, vec![a, b, c3]);
        // an end of a line is also a terminus at the other side
        assert!(lines.extend(0, a, IVec2::new(-40, 12)));
        assert_eq!(lines.all[0].cells[0], IVec2::new(-40, 12));
    }

    #[test]
    fn platform_command_reaches_transit_only_for_real_stations() {
        let centres = [Vec2::ZERO];
        let mut m = StationMap::new(1);
        let cell = IVec2::new(8, 8);
        assert!(to_transit(&cmd(1, ToolAction::AddPlatform { cell }), &mut m, &centres).0.is_empty());
        to_transit(&cmd(1, ToolAction::PlaceStation { cell }), &mut m, &centres);
        let (v, _) = to_transit(&cmd(1, ToolAction::AddPlatform { cell }), &mut m, &centres);
        assert_eq!(v[0].action, TransitAction::AddPlatform(NodeId(1)));
    }

    #[test]
    fn crossing_upgrade_is_priced_per_level_crossing_and_only_once() {
        let centres = [Vec2::ZERO];
        let mut m = StationMap::new(1);
        let (a, b) = (IVec2::new(0, 12), IVec2::new(40, 12));
        let mut net = transit::Network::default();
        net.apply(&TransitCommand { agency: AgencyId(0), action: TransitAction::AddNode }).unwrap();
        let mut hops = Hops::new();
        let level = Crossing { kind: CrossingKind::Level, at: 10.0, road: 3 };
        hops.insert(hop_key(a, b), Some(HopPlan { from: a, len: 40.0, vcap: 6.0, crossings: vec![level, Crossing { at: 20.0, road: 4, ..level }] }));
        for t in to_transit_with(&cmd(1, ToolAction::DrawLine { cells: vec![a, b] }), &mut m, &centres, &hops).0 {
            net.apply(&t).unwrap();
        }
        let (edge, ups, cost) = crossing_upgrades(&net, &m, a, b, false).unwrap();
        assert_eq!(ups.len(), 2);
        assert_eq!(cost, 2.0 * crossing_upgrade_cost(CrossingKind::Level, CrossingKind::Bridge).unwrap());
        for (idx, kind) in ups {
            net.apply(&TransitCommand { agency: AgencyId(1), action: TransitAction::UpgradeCrossing { edge, idx, kind } }).unwrap();
        }
        assert!(crossing_upgrades(&net, &m, a, b, true).is_none(), "nothing left to separate");
    }

    #[test]
    fn road_upgrade_steps_through_classes_and_is_priced() {
        let plan = city::plan_towns(7, &world_gen::generate_cities(7, 3)[..2]);
        let pop = vec![0.0f32; plan.parcels.len()];
        let gone = vec![false; plan.parcels.len()];
        let built = vec![true; plan.graph.edges.len()];
        let land = |_: Vec2| true;
        let ctx = Ctx { plan: &plan, built: &built, pop: &pop, gone: &gone, land: &land };
        let mut classes = RoadClasses::default();
        let first = road_upgrade(&ctx, &classes, 0).unwrap();
        assert!(first.cost > 0.0 && first.to > city::lanes::default_class(plan.graph.edges[0].class));
        classes.0.insert(0, first.to);
        assert!(road_upgrade(&ctx, &classes, 0).unwrap().to > first.to, "next click goes one class further");
        classes.0.insert(0, city::lanes::LaneClass::TenLane);
        assert!(road_upgrade(&ctx, &classes, 0).is_none());
    }

    #[test]
    fn a_road_built_after_the_track_still_gets_a_crossing() {
        let plan = city::plan_towns(7, &world_gen::generate_cities(7, 3)[..2]);
        // a track straight across the middle of some road edge
        let e = 0;
        let (p, q) = plan.graph.pos(e);
        let mid = (p + q) / 2.0;
        let d = (q - p).normalize();
        let n = Vec2::new(-d.y, d.x);
        let pts = [mid - n * 20.0, mid + n * 20.0];
        let none = vec![false; plan.graph.edges.len()];
        let mut built = none.clone();
        assert!(road_crossings(&plan, &none, &pts).is_empty(), "no road yet, no crossing");
        built[e] = true;
        let found = road_crossings(&plan, &built, &pts);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].road, e as u32);
        // laid track (no crossings) gains it; a second pass adds nothing
        let all = merge_crossings(&[], &found).unwrap();
        assert!(merge_crossings(&all, &found).is_none());
    }

    #[test]
    fn new_lines_default_to_single_track_and_stations_to_island_platforms() {
        let centres = [Vec2::ZERO];
        let mut m = StationMap::new(1);
        let (a, b) = (IVec2::new(0, 12), IVec2::new(40, 12));
        let mut net = transit::Network::default();
        net.apply(&TransitCommand { agency: AgencyId(0), action: TransitAction::AddNode }).unwrap();
        for t in to_transit_with(&cmd(1, ToolAction::DrawLine { cells: vec![a, b] }), &mut m, &centres, &Hops::new()).0 {
            net.apply(&t).unwrap_or_else(|e| panic!("{e:?}"));
        }
        // no placer plan -> the straight fallback edge is a plain AddEdge; with a plan it is a single bidirectional rail
        let mut hops = Hops::new();
        hops.insert(hop_key(IVec2::new(0, 12), IVec2::new(80, 12)), Some(HopPlan { from: IVec2::new(0, 12), len: 80.0, vcap: f64::INFINITY, crossings: vec![] }));
        let c = IVec2::new(80, 12);
        for t in to_transit_with(&cmd(1, ToolAction::DrawLine { cells: vec![a, c] }), &mut m, &centres, &hops).0 {
            net.apply(&t).unwrap_or_else(|e| panic!("{e:?}"));
        }
        let e = net.edges.iter().find(|e| e.kind == EdgeKind::Rail && e.len == 80.0).unwrap();
        assert!(e.tracks == 1 && e.bidir, "single track, both directions");
        assert_eq!(net.platform_count(m.node(a).unwrap()), 2, "island platform: two platform tracks");
        let i = net.rail_edge(m.node(a).unwrap(), m.node(c).unwrap()).unwrap();
        net.apply(&TransitCommand { agency: AgencyId(1), action: TransitAction::UpgradeTracks { a: m.node(a).unwrap(), b: m.node(c).unwrap() } }).unwrap();
        assert!(net.edges[i].tracks == 2 && !net.edges[i].bidir, "+Track makes it a double track");
    }
}
