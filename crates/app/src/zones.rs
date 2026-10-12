//! Growth zones from world_gen cities, run as a sim_core `Simulation`, drawn as extruded blocks by population.
use bevy::prelude::*;
use growth::{CostMatrix, GrowthParams, GrowthSim, RoadAgency, RoadPlan as ZonePlan, Zone, ZoneId, Zones};
use sim_core::{AgencyCommand, Handoff, Sim, Simulation};
use std::sync::{Arc, Mutex};
use transit::{EdgeKind, NodeId, TransitAction, TransitCommand};

/// Layer heights (flat 2D stack: land 0 < roads .3 < ghosts .5 < zones 1 < lines 2 < stations 3 < trains 4.5).
pub const Y_ZONE: f32 = 1.0;
pub const Y_ROAD: f32 = 0.3;

/// Growth cadence: ONE tunable constant. Per-tick fractional growth cap at full transit service (10 ticks/s).
/// 0.0015 => a new ring of zones fills in roughly every 1-2 minutes; a fresh town stays readable ~5 minutes.
pub const GROWTH_RATE: f64 = 0.0015;
/// Fraction of GROWTH_RATE a zone gets with no transit service at all; transit share lifts it to 1.0.
const CAR_ONLY_GROWTH: f64 = 0.4;
/// A zone with at least this population counts as built (drawn, and its children may start growing).
pub const BUILT_POP: f64 = 30.0;

pub use crate::layout::{build_layout, road_pairs, ZoneGeo, ZoneKind};

/// Growth zones (ascending id) from a layout; non-core zones start unbuilt.
pub fn zones_from(geo: &[ZoneGeo]) -> Zones {
    Zones::new(
        geo.iter()
            .enumerate()
            .map(|(i, g)| Zone { id: ZoneId(i as u32), pop: g.start_pop, jobs: g.start_pop * 0.4, capacity: g.capacity, crowding: 0.0 })
            .collect(),
    )
}

/// Startup transit commands: one node per zone (NodeId(i) == zone i), road edges between neighbours.
pub fn zone_network(agency: sim_core::AgencyId, geo: &[ZoneGeo], pairs: &[(usize, usize)]) -> Vec<TransitCommand> {
    let mut v: Vec<TransitCommand> = geo.iter().map(|_| TransitCommand { agency, action: TransitAction::AddNode }).collect();
    for &(a, b) in pairs {
        let len = geo[a].center.distance(geo[b].center) as f64;
        v.push(TransitCommand {
            agency,
            action: TransitAction::AddEdge { a: NodeId(a as u32), b: NodeId(b as u32), len, kind: EdgeKind::Road },
        });
    }
    v
}

/// ECS -> sim thread inputs. The ECS side copies transit's published `Costs`/`TransitShares` here.
/// ponytail: sim_core::Command has no payload room for a matrix, so a shared mutex stands in for a command.
#[derive(Default)]
pub struct Shared {
    pub costs: Mutex<Option<CostMatrix>>,
    pub shares: Mutex<Vec<f64>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GrowthSnapshot {
    pub tick: u64,
    pub pop: Vec<f32>,
    /// City road plans not yet built (D-7 ghosts).
    pub pending: Vec<ZonePlan>,
    pub road_reserve: f64,
    pub hash: u64,
}

pub struct GrowthSimulation {
    parents: Vec<Option<usize>>,
    sim: GrowthSim,
    agency: RoadAgency,
    shared: Arc<Shared>,
    road_reserve: f64,
}

impl GrowthSimulation {
    pub fn new(zones: Zones, parents: Vec<Option<usize>>, shared: Arc<Shared>) -> Self {
        let params = GrowthParams { rate: GROWTH_RATE, seed_pop: 10.0, ..GrowthParams::default() };
        Self { parents, sim: GrowthSim::new(zones, params), agency: RoadAgency::default(), shared, road_reserve: 0.0 }
    }
}

impl Simulation for GrowthSimulation {
    type Snapshot = GrowthSnapshot;
    /// No player commands reach growth directly; transit/economy consume them in the ECS.
    fn apply(&mut self, _cmd: &AgencyCommand) {}
    fn step(&mut self, tick: u64) {
        let n = self.sim.zones.0.len();
        let shares = self.shared.shares.lock().unwrap();
        // Transit service gates the pace: no service => CAR_ONLY_GROWTH of the rate, full service => all of it.
        let mean_share = if shares.is_empty() { 0.0 } else { shares.iter().sum::<f64>() / shares.len() as f64 };
        self.sim.params.rate = GROWTH_RATE * (CAR_ONLY_GROWTH + (1.0 - CAR_ONLY_GROWTH) * mean_share.clamp(0.0, 1.0));
        let before: Vec<(f64, f64)> = self.sim.zones.0.iter().map(|z| (z.pop, z.jobs)).collect();
        if let Some(c) = self.shared.costs.lock().unwrap().as_ref().filter(|c| c.n == n) {
            self.sim.step(c);
        }
        // Growth spreads outward: a zone cannot grow until the zone it hangs off is built.
        for i in 0..n {
            if let Some(p) = self.parents[i] {
                if before[p].0 < BUILT_POP && before[i].0 < BUILT_POP {
                    (self.sim.zones.0[i].pop, self.sim.zones.0[i].jobs) = before[i];
                }
            }
        }
        // All-zero shares until transit publishes (mgr-transit): every zone then reads as underserved.
        let zeros = vec![0.0; n];
        let s: &[f64] = if shares.len() == n { &shares } else { &zeros };
        self.road_reserve = self.agency.observe(tick, &self.sim.zones, s).road_reserve;
    }
    fn snapshot(&self, tick: u64) -> GrowthSnapshot {
        GrowthSnapshot {
            tick,
            pop: self.sim.zones.0.iter().map(|z| z.pop as f32).collect(),
            pending: self.agency.pending().to_vec(),
            road_reserve: self.road_reserve,
            hash: self.sim.state_hash(),
        }
    }
    fn state_hash(&self) -> u64 {
        self.sim.state_hash()
    }
}

#[derive(Resource)]
pub struct ZoneLayout(pub Vec<ZoneGeo>);

#[derive(Resource)]
pub struct GrowthLink {
    pub snapshots: Arc<Handoff<GrowthSnapshot>>,
    pub shared: Arc<Shared>,
}

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

/// Road stroke from a zone to its parent; shown once the zone is built.
#[derive(Component)]
pub struct ZoneRoad(pub usize);

/// Footprint scale from fill: grows from 55% to 110% of the base size as the zone fills.
pub fn footprint_scale(pop: f32, capacity: f64) -> f32 {
    0.55 + 0.55 * (pop / capacity.max(1.0) as f32).clamp(0.0, 1.0).sqrt()
}

pub fn is_built(pop: f32) -> bool {
    pop as f64 >= BUILT_POP
}

/// Segment drawn for a zone's planned road: from the zone centre to its nearest other zone.
pub fn plan_segment(layout: &[ZoneGeo], zone: usize) -> Option<(Vec2, Vec2)> {
    let a = layout.get(zone)?.center;
    layout
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != zone)
        .min_by(|(_, x), (_, y)| a.distance(x.center).total_cmp(&a.distance(y.center)))
        .map(|(_, g)| (a, g.center))
}

pub struct ZonesPlugin;

impl Plugin for ZonesPlugin {
    fn build(&self, app: &mut App) {
        let cities = app.world().get_resource::<world_gen::Cities>().map(|c| c.0.clone()).unwrap_or_default();
        let seed = app.world().get_resource::<world_gen::WorldSeed>().map_or(0, |s| s.0);
        let geo = build_layout(seed, &cities);
        let pairs = road_pairs(&geo);
        let shared = Arc::new(Shared::default());
        let parents = geo.iter().map(|g| g.parent).collect();
        let (sim, _tx, snapshots) = Sim::new(GrowthSimulation::new(zones_from(&geo), parents, shared.clone()));
        #[cfg(not(target_arch = "wasm32"))]
        app.insert_resource(SimGuard(sim_core::SimThread::spawn(sim, sim_core::SIM_HZ)));
        #[cfg(target_arch = "wasm32")]
        {
            app.insert_non_send(WasmPump { sim, clock: sim_core::FixedClock::new(sim_core::SIM_HZ) });
            app.add_systems(Update, pump_wasm);
        }
        let n = geo.len();
        // Queue the zone road network (agency 0 = the city) for transit's first FixedUpdate.
        app.world_mut()
            .get_resource_or_insert_with(transit::TransitCommands::default)
            .0
            .extend(zone_network(sim_core::AgencyId(0), &geo, &pairs));
        app.world_mut().get_resource_or_insert_with(transit::ZoneNodes::default).0 = (0..n as u32).map(NodeId).collect();
        app.insert_resource(ZoneLayout(geo)).insert_resource(GrowthLink { snapshots, shared });
        app.add_systems(Startup, spawn_blocks).add_systems(Update, (sync_inputs, update_blocks, emit_ghosts, feed_economy));
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
    let road_mat = mat(Color::srgb(0.80, 0.78, 0.72));
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
        if let Some(p) = g.parent {
            commands.spawn((
                ZoneRoad(i),
                Mesh3d(rect.clone()),
                MeshMaterial3d(road_mat.clone()),
                ui_tools::segment_transform(layout.0[p].center, g.center, Y_ROAD, 1.6, 0.2),
                Visibility::Hidden,
            ));
        }
    }
}

/// Copy transit's published matrix/shares to the sim thread when they change.
fn sync_inputs(costs: Res<growth::Costs>, shares: Res<transit::TransitShares>, link: Res<GrowthLink>) {
    if costs.is_changed() {
        *link.shared.costs.lock().unwrap() = costs.0.clone();
    }
    if shares.is_changed() {
        link.shared.shares.lock().unwrap().clone_from(&shares.0);
    }
}

fn update_blocks(
    link: Res<GrowthLink>,
    layout: Res<ZoneLayout>,
    mut last: Local<Option<u64>>,
    mut blocks: Query<(&ZoneBlock, &mut Transform, &mut Visibility), Without<ZoneRoad>>,
    mut roads: Query<(&ZoneRoad, &mut Visibility), Without<ZoneBlock>>,
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
    for (r, mut v) in &mut roads {
        *v = vis(s.pop.get(r.0).is_some_and(|&p| is_built(p)));
    }
}

/// Publish the full pending-plan set to the ghost renderer whenever it changes.
fn emit_ghosts(
    link: Res<GrowthLink>,
    layout: Res<ZoneLayout>,
    mut last: Local<Vec<ZonePlan>>,
    mut out: MessageWriter<ui_tools::GhostRoads>,
) {
    let s = link.snapshots.latest();
    if *last == s.pending {
        return;
    }
    last.clone_from(&s.pending);
    let segments = s.pending.iter().filter_map(|p| plan_segment(&layout.0, p.zone.0 as usize)).collect();
    out.write(ui_tools::GhostRoads { segments });
}

fn feed_economy(link: Res<GrowthLink>, demand: Option<ResMut<economy::RoadDemand>>) {
    if let Some(mut d) = demand {
        d.0 = link.snapshots.latest().road_reserve;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use world_gen::generate_cities;

    fn layout(seed: u64) -> Vec<ZoneGeo> {
        build_layout(seed, &generate_cities(seed, 3))
    }

    fn sim(seed: u64) -> (GrowthSimulation, usize) {
        let geo = layout(seed);
        let n = geo.len();
        let shared = Arc::new(Shared::default());
        *shared.costs.lock().unwrap() = Some(CostMatrix::from_fn(n, |i, j| if i == j { 0.0 } else { 3.0 }));
        (GrowthSimulation::new(zones_from(&geo), geo.iter().map(|g| g.parent).collect(), shared), n)
    }

    fn built(s: &GrowthSnapshot) -> usize {
        s.pop.iter().filter(|&&p| is_built(p)).count()
    }

    #[test]
    fn zone_network_matches_layout() {
        let geo = layout(1);
        let pairs = road_pairs(&geo);
        assert_eq!(zone_network(sim_core::AgencyId(0), &geo, &pairs).len(), geo.len() + pairs.len());
    }

    #[test]
    fn zone_count_after_five_minutes_stays_under_target() {
        // 10 ticks/s: 3000 ticks = 5 minutes. Fresh game: a few built zones, far from the whole town.
        let (mut s, n) = sim(1);
        let start = built(&s.snapshot(0));
        for t in 0..3000 {
            s.step(t);
        }
        let after = built(&s.snapshot(3000));
        assert!(after > start, "town must grow: {start} -> {after}");
        assert!(after <= 60 && after * 2 < n, "5 min: {after} of {n} built (start {start})");
    }

    #[test]
    fn growth_is_deterministic_and_gated_by_parent() {
        let run = || {
            let (mut s, _) = sim(2);
            for t in 0..400 {
                s.step(t);
            }
            s.snapshot(400)
        };
        let a = run();
        assert_eq!(a, run());
        let geo = layout(2);
        for (i, g) in geo.iter().enumerate() {
            if is_built(a.pop[i]) && geo[i].start_pop == 0.0 {
                assert!(is_built(a.pop[g.parent.unwrap()]), "zone {i} built before its parent");
            }
        }
    }

    #[test]
    fn transit_service_speeds_growth() {
        let total = |share: f64| {
            let (mut s, n) = sim(1);
            *s.shared.shares.lock().unwrap() = vec![share; n];
            for t in 0..1500 {
                s.step(t);
            }
            s.snapshot(1500).pop.iter().sum::<f32>()
        };
        assert!(total(1.0) > total(0.0));
    }

    #[test]
    fn underserved_zones_emit_ghost_plans_after_streak() {
        let (mut s, _) = sim(1);
        for t in 0..120 {
            s.step(t);
        }
        let snap = s.snapshot(120);
        assert!(!snap.pending.is_empty() && snap.road_reserve > 0.0);
        let geo = layout(1);
        let (a, b) = plan_segment(&geo, snap.pending[0].zone.0 as usize).unwrap();
        assert!(a != b);
    }

    #[test]
    fn footprint_grows_with_fill() {
        assert!(footprint_scale(30.0, 1000.0) < footprint_scale(900.0, 1000.0));
        assert!(is_built(30.0) && !is_built(29.0));
    }
}
