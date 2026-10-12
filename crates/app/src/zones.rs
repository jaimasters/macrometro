//! Growth zones from the generated city plan, run as a sim_core `Simulation`, drawn as footprints by population.
//! The sim thread owns zone growth, the city road builder (D-7) and traffic/mode choice (docs/arch/city-gen.md);
//! the ECS side mirrors built roads into transit, feeds it transit times and draws what the snapshot says.
use bevy::prelude::*;
use city::{hour_of, RoadBuilder, TownPlan, Traffic, TrafficOut, ROADS_PER_RATE, TRAFFIC_EVERY};
use crate::persist::GrowthSave;
use economy::Scorecard;
use growth::{AgencyParams, CostMatrix, GrowthParams, GrowthSim, Zone, ZoneId, Zones};
use sim_core::{AgencyCommand, Handoff, Sim, Simulation};
use std::sync::{Arc, Mutex};
use transit::{EdgeKind, NodeId, TransitAction, TransitCommand};

/// Layer heights (flat 2D stack: land 0 < roads .3 < ghosts .5 < zones 1 < lines 2 < stations 3 < trains 4.5).
pub const Y_ZONE: f32 = 1.0;
pub const Y_ROAD: f32 = 0.3;

/// Growth cadence: ONE tunable constant. Per-tick fractional growth cap at full transit service (10 ticks/s).
/// 0.0015 => a new ring of zones fills in roughly every 1-2 minutes; a fresh town stays readable ~5 minutes.
/// The city's road building pace is derived from it (city::ROADS_PER_RATE roads per unit of rate).
pub const GROWTH_RATE: f64 = 0.0015;
/// Fraction of GROWTH_RATE a zone gets with no transit service at all; transit share lifts it to 1.0.
const CAR_ONLY_GROWTH: f64 = 0.4;
pub use city::BUILT_POP;

pub use crate::layout::{build_layout, plan_layout, ZoneGeo, ZoneKind};

/// Growth zones (ascending id) from a layout; non-core zones start unbuilt.
pub fn zones_from(geo: &[ZoneGeo]) -> Zones {
    Zones::new(
        geo.iter()
            .enumerate()
            .map(|(i, g)| Zone { id: ZoneId(i as u32), pop: g.start_pop, jobs: g.start_pop * 0.4, capacity: g.capacity, crowding: 0.0 })
            .collect(),
    )
}

/// The generated plan plus per-edge frontage lists. Transit node ids: zone i = NodeId(i), road node n = NodeId(zones + n).
#[derive(Resource, Clone)]
pub struct CityPlan {
    pub plan: Arc<TownPlan>,
    pub fronting: Vec<Vec<usize>>,
}

impl CityPlan {
    pub fn new(plan: TownPlan) -> Self {
        let fronting = plan.fronting();
        Self { plan: Arc::new(plan), fronting }
    }
    pub fn road_node(&self, n: u32) -> NodeId {
        NodeId(self.plan.parcels.len() as u32 + n)
    }
    /// Total transit nodes taken by zones and road nodes (player stations follow).
    pub fn node_count(&self) -> u32 {
        (self.plan.parcels.len() + self.plan.graph.nodes.len()) as u32
    }
}

/// Transit commands mirroring built road `edges` (and each frontage zone's link to its access node).
pub fn mirror_commands(agency: sim_core::AgencyId, cp: &CityPlan, edges: &[usize]) -> Vec<TransitCommand> {
    let g = &cp.plan.graph;
    let mut v = vec![];
    for &e in edges {
        let ed = &g.edges[e];
        let action = TransitAction::AddEdge { a: cp.road_node(ed.a), b: cp.road_node(ed.b), len: ed.len as f64, kind: EdgeKind::Road };
        v.push(TransitCommand { agency, action });
        for &z in &cp.fronting[e] {
            let n = cp.plan.access_node(z);
            let len = g.nodes[n as usize].distance(cp.plan.parcels[z].center).max(1.0) as f64;
            let action = TransitAction::AddEdge { a: NodeId(z as u32), b: cp.road_node(n), len, kind: EdgeKind::Road };
            v.push(TransitCommand { agency, action });
        }
    }
    v
}

/// Startup transit commands: one node per zone then one per road node, then the roads built at game start.
pub fn zone_network(agency: sim_core::AgencyId, cp: &CityPlan, built: &[bool]) -> Vec<TransitCommand> {
    let mut v: Vec<TransitCommand> =
        (0..cp.node_count()).map(|_| TransitCommand { agency, action: TransitAction::AddNode }).collect();
    let edges: Vec<usize> = (0..built.len()).filter(|&e| built[e]).collect();
    v.extend(mirror_commands(agency, cp, &edges));
    v
}

/// Door-to-door transit minutes between zones plus line capacity, computed by the ECS side when transit changes.
pub struct TransitInput {
    pub times: CostMatrix,
    pub capacity: f64,
}

/// ECS -> sim thread inputs. The ECS side copies transit's published `Costs` and its transit-time matrix here.
/// ponytail: sim_core::Command has no payload room for a matrix, so a shared mutex stands in for a command.
#[derive(Default)]
pub struct Shared {
    pub costs: Mutex<Option<CostMatrix>>,
    pub transit: Mutex<Option<Arc<TransitInput>>>,
    /// Latest saveable growth state, refreshed by the sim every SAVE_EVERY ticks (read by autosave).
    pub save: Mutex<Option<GrowthSave>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GrowthSnapshot {
    pub tick: u64,
    pub pop: Vec<f32>,
    /// Per road edge: built yet?
    pub built: Vec<bool>,
    /// Road edges the city has planned but not yet built (D-7 ghosts).
    pub pending: Vec<usize>,
    pub road_reserve: f64,
    /// Per road edge: car flow (veh/h), volume/capacity, congestion slowdown (>= 1).
    pub flow: Vec<f32>,
    pub vc: Vec<f32>,
    pub slow: Vec<f32>,
    /// Per zone transit share of trips.
    pub zone_share: Vec<f32>,
    pub scorecard: Scorecard,
    /// Game hour 0..24.
    pub hour: f32,
    pub hash: u64,
}

/// Scorecard from a traffic assignment; every field 0..=1, higher = better (D-9).
pub fn scorecard(out: &TrafficOut, transit_capacity: f64) -> Scorecard {
    let trips = out.car_trips + out.transit_trips;
    Scorecard {
        accessibility: out.accessibility.clamp(0.0, 1.0),
        trip_time: 1.0 / (1.0 + out.mean_trip_time / 25.0),
        transit_share: out.transit_share(),
        crowding: if transit_capacity > 0.0 { 1.0 - (out.transit_trips / transit_capacity).min(1.0) } else { 1.0 },
        emissions: if trips > 0.0 { 1.0 - (out.car_vkt / (trips * 60.0)).clamp(0.0, 1.0) } else { 1.0 },
        equity: out.equity,
    }
}

/// Ticks between refreshes of the autosave copy of the growth state (10 Hz sim: every 5 s).
const SAVE_EVERY: u64 = 50;

pub struct GrowthSimulation {
    plan: Arc<TownPlan>,
    sim: GrowthSim,
    builder: RoadBuilder,
    traffic: Traffic,
    scorecard: Scorecard,
    shared: Arc<Shared>,
    road_reserve: f64,
}

impl GrowthSimulation {
    /// Saveable state at `tick` (the next tick to run).
    pub fn export(&self, tick: u64) -> GrowthSave {
        GrowthSave {
            tick,
            zones: self.sim.zones.0.iter().map(|z| (z.pop, z.jobs)).collect(),
            built: self.builder.built.clone(),
            pending: self.builder.pending.clone(),
        }
    }

    /// Resume from a save made on the same map; a mismatched save is ignored.
    pub fn restore(&mut self, g: &GrowthSave) {
        if g.zones.len() != self.sim.zones.0.len() {
            return;
        }
        for (z, (pop, jobs)) in self.sim.zones.0.iter_mut().zip(&g.zones) {
            (z.pop, z.jobs) = (*pop, *jobs);
        }
        self.builder.restore(&self.plan, &g.built, &g.pending);
    }

    pub fn new(zones: Zones, plan: Arc<TownPlan>, shared: Arc<Shared>) -> Self {
        let params = GrowthParams { rate: GROWTH_RATE, seed_pop: 10.0, ..GrowthParams::default() };
        // underserved-streak and plan lead scale with the growth cadence (one knob)
        let agency = AgencyParams { t_ticks: (0.15 / GROWTH_RATE) as u32, lead_ticks: city::LEAD_TICKS, ..AgencyParams::default() };
        Self {
            builder: RoadBuilder::new(&plan, agency),
            traffic: Traffic::new(&plan),
            sim: GrowthSim::new(zones, params),
            scorecard: Scorecard::default(),
            plan,
            shared,
            road_reserve: 0.0,
        }
    }
}

impl Simulation for GrowthSimulation {
    type Snapshot = GrowthSnapshot;
    /// No player commands reach growth directly; transit/economy consume them in the ECS.
    fn apply(&mut self, _cmd: &AgencyCommand) {}
    fn step(&mut self, tick: u64) {
        let n = self.sim.zones.0.len();
        let transit = self.shared.transit.lock().unwrap().clone();
        // Transit service gates the pace: no service => CAR_ONLY_GROWTH of the rate, full service => all of it.
        let mean_share = self.traffic.out.transit_share().clamp(0.0, 1.0);
        let rate = GROWTH_RATE * (CAR_ONLY_GROWTH + (1.0 - CAR_ONLY_GROWTH) * mean_share);
        self.sim.params.rate = rate;
        let before: Vec<(f64, f64)> = self.sim.zones.0.iter().map(|z| (z.pop, z.jobs)).collect();
        if let Some(c) = self.shared.costs.lock().unwrap().as_ref().filter(|c| c.n == n) {
            self.sim.step(c);
        }
        // A zone only develops with road access: no built frontage road, no growth.
        for i in 0..n {
            if !self.builder.built[self.plan.parcels[i].access] {
                (self.sim.zones.0[i].pop, self.sim.zones.0[i].jobs) = before[i];
            }
        }
        // The city builds roads outward as the town fills in (D-7); poorly served zones pull more.
        self.builder.step(tick, &self.plan, &self.sim.zones, &self.traffic.out.zone_share, rate * ROADS_PER_RATE);
        if tick % TRAFFIC_EVERY == 0 {
            let t = transit.as_ref().map(|t| &t.times);
            self.traffic.assign(&self.builder.built, &self.sim.zones, t, hour_of(tick));
            self.scorecard = scorecard(&self.traffic.out, transit.as_ref().map_or(0.0, |t| t.capacity));
        }
        self.road_reserve = self.builder.reserve(&self.plan);
        if tick % SAVE_EVERY == 0 {
            *self.shared.save.lock().unwrap() = Some(self.export(tick + 1));
        }
    }
    fn snapshot(&self, tick: u64) -> GrowthSnapshot {
        let ne = self.plan.graph.edges.len();
        GrowthSnapshot {
            tick,
            pop: self.sim.zones.0.iter().map(|z| z.pop as f32).collect(),
            built: self.builder.built.clone(),
            pending: self.builder.pending.iter().map(|p| p.0).collect(),
            road_reserve: self.road_reserve,
            flow: self.traffic.flow.iter().map(|&q| q as f32).collect(),
            vc: (0..ne).map(|e| self.traffic.vc(e) as f32).collect(),
            slow: (0..ne).map(|e| self.traffic.slowdown(e) as f32).collect(),
            zone_share: self.traffic.out.zone_share.iter().map(|&s| s as f32).collect(),
            scorecard: self.scorecard,
            hour: hour_of(tick) as f32,
            hash: self.state_hash(),
        }
    }
    fn state_hash(&self) -> u64 {
        self.sim.state_hash().rotate_left(7) ^ self.builder.hash()
    }
}

#[derive(Resource)]
pub struct ZoneLayout(pub Vec<ZoneGeo>);

#[derive(Resource)]
pub struct GrowthLink {
    pub snapshots: Arc<Handoff<GrowthSnapshot>>,
    pub shared: Arc<Shared>,
}

/// Which road edges have already been mirrored into transit.
#[derive(Resource)]
struct MirroredRoads(Vec<bool>);

#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource)]
#[allow(dead_code)] // owns the thread so it joins on app drop
struct SimGuard(sim_core::SimThread);

/// wasm has no threads: the app pumps the sim from the frame loop (D-arch-1).
#[cfg(target_arch = "wasm32")]
struct WasmPump {
    sim: Sim<GrowthSimulation>,
    clock: sim_core::FixedClock,
}

/// One building footprint per zone (flat, oriented along its road).
#[derive(Component)]
pub struct ZoneBlock(pub usize);

/// Footprint scale from fill: grows from 55% to 110% of the base size as the zone fills.
pub fn footprint_scale(pop: f32, capacity: f64) -> f32 {
    0.55 + 0.55 * (pop / capacity.max(1.0) as f32).clamp(0.0, 1.0).sqrt()
}

pub fn is_built(pop: f32) -> bool {
    pop as f64 >= BUILT_POP
}

pub struct ZonesPlugin;

impl Plugin for ZonesPlugin {
    fn build(&self, app: &mut App) {
        let cities = app.world().get_resource::<world_gen::Cities>().map(|c| c.0.clone()).unwrap_or_default();
        let seed = app.world().get_resource::<world_gen::WorldSeed>().map_or(0, |s| s.0);
        let cp = CityPlan::new(plan_layout(seed, &cities));
        let geo = cp.plan.parcels.clone();
        let shared = Arc::new(Shared::default());
        let mut sim_state = GrowthSimulation::new(zones_from(&geo), cp.plan.clone(), shared.clone());
        let saved = app.world().get_resource::<crate::persist::LoadedGame>().and_then(|l| l.0.as_ref()).map(|g| g.growth.clone());
        if let Some(g) = &saved {
            sim_state.restore(g);
            *shared.save.lock().unwrap() = Some(g.clone());
        }
        let initial_built = sim_state.builder.built.clone();
        let (mut sim, _tx, snapshots) = Sim::new(sim_state);
        sim.tick = saved.map_or(0, |g| g.tick);
        #[cfg(not(target_arch = "wasm32"))]
        app.insert_resource(SimGuard(sim_core::SimThread::spawn(sim, sim_core::SIM_HZ)));
        #[cfg(target_arch = "wasm32")]
        {
            app.insert_non_send(WasmPump { sim, clock: sim_core::FixedClock::new(sim_core::SIM_HZ) });
            app.add_systems(Update, pump_wasm);
        }
        let n = geo.len();
        // Queue the zone + road network (agency 0 = the city) for transit's first FixedUpdate.
        app.world_mut()
            .get_resource_or_insert_with(transit::TransitCommands::default)
            .0
            .extend(zone_network(sim_core::AgencyId(0), &cp, &initial_built));
        app.world_mut().get_resource_or_insert_with(transit::ZoneNodes::default).0 = (0..n as u32).map(NodeId).collect();
        app.insert_resource(ZoneLayout(geo))
            .insert_resource(cp)
            .insert_resource(MirroredRoads(initial_built))
            .insert_resource(GrowthLink { snapshots, shared });
        app.add_systems(Startup, spawn_blocks).add_systems(
            Update,
            (sync_inputs, publish_transit, update_blocks, emit_ghosts, feed_economy, mirror_roads, apply_metrics),
        );
    }
}

#[cfg(target_arch = "wasm32")]
fn pump_wasm(time: Res<Time>, mut p: NonSendMut<WasmPump>) {
    let p = &mut *p;
    p.sim.pump(&mut p.clock, time.delta(), 2);
}

fn spawn_blocks(
    layout: Res<ZoneLayout>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let rect = meshes.add(Cuboid::new(1.0, 0.3, 1.0));
    let round = meshes.add(Cylinder::new(0.5, 0.3));
    let mut mat = |c: Color| mats.add(StandardMaterial { base_color: c, unlit: true, ..default() });
    let kinds = [ZoneKind::Residential, ZoneKind::Commercial, ZoneKind::Industrial].map(|k| (k, mat(k.colour())));
    for (i, g) in layout.0.iter().enumerate() {
        let material = kinds.iter().find(|(k, _)| *k == g.kind).unwrap().1.clone();
        // commercial = round plaza, industrial = long yard, residential = block (all oriented to the road)
        let (mesh, size) = match g.kind {
            ZoneKind::Commercial => (round.clone(), Vec2::splat(g.size.y + 1.0)),
            ZoneKind::Industrial => (rect.clone(), Vec2::new(g.size.x * 1.3, g.size.y * 1.1)),
            ZoneKind::Residential => (rect.clone(), g.size),
        };
        commands.spawn((
            ZoneBlock(i),
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform {
                translation: Vec3::new(g.center.x, Y_ZONE, g.center.y),
                rotation: Quat::from_rotation_y(-g.angle),
                scale: Vec3::new(size.x, 1.0, size.y),
            },
            Visibility::Hidden,
        ));
    }
}

/// Copy transit's published matrix to the sim thread when it changes.
fn sync_inputs(costs: Res<growth::Costs>, link: Res<GrowthLink>) {
    if costs.is_changed() {
        *link.shared.costs.lock().unwrap() = costs.0.clone();
    }
}

/// Recompute door-to-door transit times over the player's network whenever transit changes.
fn publish_transit(
    net: Res<transit::TransitNet>,
    map: Res<crate::shell::StationMap>,
    layout: Res<ZoneLayout>,
    link: Res<GrowthLink>,
) {
    if !net.is_changed() {
        return;
    }
    let stations: Vec<city::StationPos> =
        map.stations().map(|(node, cell)| city::StationPos { node, p: cell.as_vec2() }).collect();
    let centres: Vec<Vec2> = layout.0.iter().map(|g| g.center).collect();
    let (times, capacity) = city::transit_times(&net.0, &centres, &stations);
    *link.shared.transit.lock().unwrap() = Some(Arc::new(TransitInput { times, capacity }));
}

fn update_blocks(
    link: Res<GrowthLink>,
    layout: Res<ZoneLayout>,
    mut last: Local<Option<u64>>,
    mut blocks: Query<(&ZoneBlock, &mut Transform, &mut Visibility)>,
) {
    let s = link.snapshots.latest();
    if *last == Some(s.tick) {
        return;
    }
    *last = Some(s.tick);
    let vis = |built: bool| if built { Visibility::Inherited } else { Visibility::Hidden };
    for (b, mut t, mut v) in &mut blocks {
        let (Some(&pop), Some(g)) = (s.pop.get(b.0), layout.0.get(b.0)) else { continue };
        *v = vis(is_built(pop));
        let base = match g.kind {
            ZoneKind::Commercial => Vec2::splat(g.size.y + 1.0),
            ZoneKind::Industrial => Vec2::new(g.size.x * 1.3, g.size.y * 1.1),
            ZoneKind::Residential => g.size,
        };
        let k = footprint_scale(pop, g.capacity);
        t.scale = Vec3::new(base.x * k, 1.0, base.y * k);
    }
}

/// Publish the full pending-road set to the ghost renderer whenever it changes.
fn emit_ghosts(
    link: Res<GrowthLink>,
    cp: Res<CityPlan>,
    mut last: Local<Vec<usize>>,
    mut out: MessageWriter<ui_tools::GhostRoads>,
) {
    let s = link.snapshots.latest();
    if *last == s.pending {
        return;
    }
    last.clone_from(&s.pending);
    out.write(ui_tools::GhostRoads { segments: s.pending.iter().map(|&e| cp.plan.graph.pos(e)).collect() });
}

fn feed_economy(link: Res<GrowthLink>, demand: Option<ResMut<economy::RoadDemand>>) {
    if let Some(mut d) = demand {
        d.0 = link.snapshots.latest().road_reserve;
    }
}

/// Built roads appear in the transit graph (stations link to them, costs follow).
fn mirror_roads(
    link: Res<GrowthLink>,
    cp: Res<CityPlan>,
    mut done: ResMut<MirroredRoads>,
    mut cmds: ResMut<transit::TransitCommands>,
) {
    let s = link.snapshots.latest();
    let fresh: Vec<usize> = (0..s.built.len()).filter(|&e| s.built[e] && !done.0[e]).collect();
    if fresh.is_empty() {
        return;
    }
    cmds.0.extend(mirror_commands(sim_core::AgencyId(0), &cp, &fresh));
    for e in fresh {
        done.0[e] = true;
    }
}

/// Real mode share / scorecard from the traffic sim replace the old stubs.
fn apply_metrics(
    link: Res<GrowthLink>,
    mut last: Local<Option<u64>>,
    mut shares: ResMut<transit::TransitShares>,
    mut cards: ResMut<economy::Scorecards>,
    mut car: ResMut<economy::CarShare>,
) {
    let s = link.snapshots.latest();
    let key = s.tick / TRAFFIC_EVERY;
    if *last == Some(key) {
        return;
    }
    *last = Some(key);
    shares.0 = s.zone_share.iter().map(|&x| x as f64).collect();
    cards.0 = vec![(crate::shell::PLAYER, s.scorecard)];
    car.0 = (1.0 - s.scorecard.transit_share).clamp(0.0, 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use world_gen::generate_cities;

    fn plan(seed: u64) -> Arc<TownPlan> {
        Arc::new(plan_layout(seed, &generate_cities(seed, 3)))
    }

    fn sim(seed: u64) -> (GrowthSimulation, usize) {
        let p = plan(seed);
        let n = p.parcels.len();
        let shared = Arc::new(Shared::default());
        *shared.costs.lock().unwrap() = Some(CostMatrix::from_fn(n, |i, j| if i == j { 0.0 } else { 3.0 }));
        (GrowthSimulation::new(zones_from(&p.parcels), p, shared), n)
    }

    #[test]
    fn export_then_restore_resumes_the_same_city() {
        let (mut a, _) = sim(5);
        run(&mut a, 400);
        let saved = a.export(400);
        let (mut b, _) = sim(5);
        b.restore(&saved);
        assert_eq!(b.state_hash(), a.state_hash(), "restored city matches");
        assert_eq!(b.export(400), saved);
        // a save from another map is ignored rather than corrupting the sim
        let (mut c, _) = sim(5);
        c.restore(&GrowthSave { zones: vec![(1.0, 1.0)], ..saved.clone() });
        assert_ne!(c.state_hash(), a.state_hash());
    }

    fn built(s: &GrowthSnapshot) -> usize {
        s.pop.iter().filter(|&&p| is_built(p)).count()
    }

    fn run(s: &mut GrowthSimulation, ticks: u64) -> GrowthSnapshot {
        for t in 0..ticks {
            s.step(t);
        }
        s.snapshot(ticks)
    }

    #[test]
    fn zone_network_covers_zones_roads_and_the_starting_village() {
        let cp = CityPlan::new(plan_layout(1, &generate_cities(1, 3)));
        let built = RoadBuilder::new(&cp.plan, AgencyParams::default()).built;
        let cmds = zone_network(sim_core::AgencyId(0), &cp, &built);
        let nodes = cmds.iter().filter(|c| c.action == TransitAction::AddNode).count();
        assert_eq!(nodes as u32, cp.node_count());
        let mut net = transit::Network::default();
        for c in &cmds {
            net.apply(c).unwrap_or_else(|e| panic!("{c:?}: {e:?}"));
        }
        assert!(net.edges.len() >= built.iter().filter(|b| **b).count());
        // every populated start zone reaches the core over built roads
        let zn: Vec<NodeId> = (0..cp.plan.parcels.len() as u32).map(NodeId).collect();
        let m = transit::cost_matrix(&net, &zn);
        let core = cp.plan.parcels.iter().position(|p| p.core && p.town == 0).unwrap();
        for (i, _) in cp.plan.parcels.iter().enumerate().filter(|(_, p)| p.start_pop > 0.0 && p.town == 0) {
            assert!(m.get(core, i).is_finite(), "start zone {i} cut off from the core");
        }
    }

    #[test]
    fn zone_count_after_five_minutes_stays_under_target() {
        // 10 ticks/s: 3000 ticks = 5 minutes. Fresh game: a few built zones, far from the whole town.
        let (mut s, n) = sim(1);
        let start = built(&s.snapshot(0));
        let snap = run(&mut s, 3000);
        let after = built(&snap);
        eprintln!("5 min: {start} -> {after} built of {n}; roads {} built of {}", snap.built.iter().filter(|b| **b).count(), snap.built.len());
        assert!(after > start, "town must grow: {start} -> {after}");
        assert!(after <= 60 && after * 2 < n, "5 min: {after} of {n} built (start {start})");
    }

    #[test]
    fn every_developed_zone_has_road_access_and_runs_are_deterministic() {
        let a = run(&mut sim(2).0, 1500);
        assert_eq!(a, run(&mut sim(2).0, 1500));
        let p = plan(2);
        for (i, z) in p.parcels.iter().enumerate() {
            if is_built(a.pop[i]) {
                assert!(a.built[z.access], "zone {i} developed on an unbuilt road");
            }
            if !a.built[z.access] {
                assert_eq!(a.pop[i] as f64, z.start_pop, "zone {i} grew without road access");
            }
        }
    }

    #[test]
    fn transit_service_speeds_growth() {
        let total = |minutes: Option<f64>| {
            let (mut s, n) = sim(1);
            if let Some(m) = minutes {
                let times = CostMatrix::from_fn(n, |i, j| if i == j { 0.0 } else { m });
                *s.shared.transit.lock().unwrap() = Some(Arc::new(TransitInput { times, capacity: 1e6 }));
            }
            run(&mut s, 1500).pop.iter().sum::<f32>()
        };
        assert!(total(Some(5.0)) > total(None));
    }

    #[test]
    fn city_builds_roads_ghosted_first_and_traffic_appears() {
        let (mut s, _) = sim(1);
        let start = s.snapshot(0).built.iter().filter(|b| **b).count();
        let (mut saw_ghost, mut flow) = (false, 0.0f32);
        for t in 0..2400 {
            s.step(t);
            let snap = s.snapshot(t);
            saw_ghost |= !snap.pending.is_empty() && snap.road_reserve > 0.0;
            flow = flow.max(snap.flow.iter().cloned().fold(0.0, f32::max));
        }
        let end = s.snapshot(2400);
        assert!(saw_ghost, "no planned-road ghost ever shown");
        assert!(end.built.iter().filter(|b| **b).count() > start, "no road was built");
        assert!(flow > 5.0, "no traffic on the roads (peak flow {flow})");
        assert!(end.scorecard.transit_share == 0.0 && end.scorecard.accessibility > 0.0);
    }

    /// Tuning aid: `cargo test -p app soak -- --ignored --nocapture` prints town/traffic growth over 30 game minutes.
    #[test]
    #[ignore]
    fn soak_prints_traffic_over_time() {
        let (mut s, _) = sim(1);
        for t in 0..18000u64 {
            s.step(t);
            if t % 1800 == 0 {
                let sn = s.snapshot(t);
                let maxvc = sn.vc.iter().cloned().fold(0.0, f32::max);
                eprintln!(
                    "min {:>4.1} built zones {:>3} pop {:>7.0} roads {:>3} max v/c {:.2} trip time {:.1} access {:.2}",
                    t as f64 / 600.0, built(&sn), sn.pop.iter().sum::<f32>(), sn.built.iter().filter(|b| **b).count(), maxvc,
                    s.traffic.out.mean_trip_time, sn.scorecard.accessibility
                );
            }
        }
    }

    #[test]
    fn scorecard_is_normalised_and_rewards_transit() {
        let out = |t: f64, c: f64| TrafficOut { car_trips: c, transit_trips: t, car_vkt: c * 40.0, mean_trip_time: 20.0, accessibility: 0.5, equity: 0.9, zone_share: vec![] };
        let (low, high) = (scorecard(&out(10.0, 90.0), 1000.0), scorecard(&out(80.0, 20.0), 1000.0));
        for c in [low, high] {
            for v in [c.accessibility, c.trip_time, c.transit_share, c.crowding, c.emissions, c.equity] {
                assert!((0.0..=1.0).contains(&v), "{c:?}");
            }
        }
        assert!(high.transit_share > low.transit_share && high.emissions > low.emissions);
        assert!(high.crowding < low.crowding, "more riders -> more crowding");
    }

    #[test]
    fn footprint_grows_with_fill() {
        assert!(footprint_scale(30.0, 1000.0) < footprint_scale(900.0, 1000.0));
        assert!(is_built(30.0) && !is_built(29.0));
    }
}
