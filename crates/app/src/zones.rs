//! Growth zones from world_gen cities, run as a sim_core `Simulation`, drawn as extruded blocks by population.
use crate::terrain::ground_y;
use bevy::prelude::*;
use growth::{CostMatrix, GrowthParams, GrowthSim, RoadAgency, RoadPlan as ZonePlan, Zone, ZoneId, Zones};
use sim_core::{AgencyCommand, Handoff, Sim, Simulation};
use std::sync::{Arc, Mutex};
use transit::{EdgeKind, NodeId, TransitAction, TransitCommand};
use world_gen::{CityRegion, WorldSeed, CHUNK_SIZE};

/// Peak zone capacity (residents) at a city centre; falls off to the region edge.
const PEAK_CAPACITY: f64 = 2000.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoneGeo {
    pub center: Vec2,
    pub city: u32,
}

/// One zone per city chunk, ids ascending. Returns zones, geometry and 4-neighbour index pairs.
pub fn build_zones(cities: &[CityRegion]) -> (Zones, Vec<ZoneGeo>, Vec<(usize, usize)>) {
    let (mut zones, mut geo, mut pairs) = (vec![], vec![], vec![]);
    for c in cities {
        let (r, base) = (c.radius as i64, zones.len());
        let w = 2 * r + 1;
        for (k, ch) in c.chunks().enumerate() {
            let (dx, dz) = ((ch.x - c.center.x).abs(), (ch.z - c.center.z).abs());
            let capacity = PEAK_CAPACITY * (1.0 - dx.max(dz) as f64 / (r + 1) as f64);
            let (ox, oz) = ch.origin_cell();
            let half = CHUNK_SIZE as f32 / 2.0;
            let pop = capacity * 0.05;
            zones.push(Zone { id: ZoneId(zones.len() as u32), pop, jobs: pop * 0.4, capacity, crowding: 0.0 });
            geo.push(ZoneGeo { center: Vec2::new(ox as f32 + half, oz as f32 + half), city: c.id.0 });
            let (i, j) = (k as i64 % w, k as i64 / w);
            if i < w - 1 {
                pairs.push((base + k, base + k + 1));
            }
            if j < w - 1 {
                pairs.push((base + k, base + k + w as usize));
            }
        }
    }
    (Zones::new(zones), geo, pairs)
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
    sim: GrowthSim,
    agency: RoadAgency,
    shared: Arc<Shared>,
    road_reserve: f64,
}

impl GrowthSimulation {
    pub fn new(zones: Zones, shared: Arc<Shared>) -> Self {
        Self { sim: GrowthSim::new(zones, GrowthParams::default()), agency: RoadAgency::default(), shared, road_reserve: 0.0 }
    }
}

impl Simulation for GrowthSimulation {
    type Snapshot = GrowthSnapshot;
    /// No player commands reach growth directly; transit/economy consume them in the ECS.
    fn apply(&mut self, _cmd: &AgencyCommand) {}
    fn step(&mut self, tick: u64) {
        let n = self.sim.zones.0.len();
        if let Some(c) = self.shared.costs.lock().unwrap().as_ref().filter(|c| c.n == n) {
            self.sim.step(c);
        }
        let shares = self.shared.shares.lock().unwrap();
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

#[derive(Component)]
pub struct ZoneBlock(pub usize);

/// Block footprint (cells) and height scale: ~ sqrt so large zones don't dwarf the terrain.
const BLOCK_FOOTPRINT: f32 = 20.0;

pub fn block_height(pop: f32) -> f32 {
    1.0 + 0.8 * pop.max(0.0).sqrt()
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
        let (zones, geo, pairs) = build_zones(&cities);
        let shared = Arc::new(Shared::default());
        let (sim, _tx, snapshots) = Sim::new(GrowthSimulation::new(zones, shared.clone()));
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
    seed: Res<WorldSeed>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let mat = mats.add(StandardMaterial { base_color: Color::srgb(0.85, 0.55, 0.30), perceptual_roughness: 1.0, ..default() });
    for (i, g) in layout.0.iter().enumerate() {
        let y = ground_y(seed.0, g.center.x, g.center.y);
        commands.spawn((
            ZoneBlock(i),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_xyz(g.center.x, y, g.center.y).with_scale(Vec3::new(BLOCK_FOOTPRINT, 0.1, BLOCK_FOOTPRINT)),
        ));
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
    seed: Res<WorldSeed>,
    layout: Res<ZoneLayout>,
    mut last: Local<Option<u64>>,
    mut q: Query<(&ZoneBlock, &mut Transform)>,
) {
    let s = link.snapshots.latest();
    if *last == Some(s.tick) {
        return;
    }
    *last = Some(s.tick);
    for (b, mut t) in &mut q {
        let (Some(&pop), Some(g)) = (s.pop.get(b.0), layout.0.get(b.0)) else { continue };
        let h = block_height(pop);
        t.scale.y = h;
        t.translation.y = ground_y(seed.0, g.center.x, g.center.y) + h / 2.0;
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

    #[test]
    fn zones_cover_cities_with_neighbour_edges() {
        let cities = generate_cities(1, 3);
        let (zones, geo, pairs) = build_zones(&cities);
        let expect: usize = cities.iter().map(|c| (2 * c.radius as usize + 1).pow(2)).sum();
        assert_eq!((zones.0.len(), geo.len()), (expect, expect));
        assert!(zones.0.windows(2).all(|w| w[0].id < w[1].id));
        let per: usize = cities.iter().map(|c| 2 * (2 * c.radius as usize + 1) * (2 * c.radius as usize)).sum();
        assert_eq!(pairs.len(), per);
        assert!(pairs.iter().all(|&(a, b)| geo[a].city == geo[b].city && geo[a].center.distance(geo[b].center) == 32.0));
        assert_eq!(zone_network(sim_core::AgencyId(0), &geo, &pairs).len(), expect + per);
    }

    #[test]
    fn simulation_grows_with_costs_and_is_deterministic() {
        let (zones, _, _) = build_zones(&generate_cities(1, 1));
        let n = zones.0.len();
        let run = || {
            let shared = Arc::new(Shared::default());
            *shared.costs.lock().unwrap() = Some(CostMatrix::from_fn(n, |i, j| if i == j { 0.0 } else { 3.0 }));
            let mut s = GrowthSimulation::new(zones.clone(), shared);
            let before = s.snapshot(0);
            for t in 0..50 {
                s.step(t);
            }
            (before, s.snapshot(50))
        };
        let (b, a) = run();
        assert!(a.pop.iter().sum::<f32>() > b.pop.iter().sum::<f32>());
        assert_eq!(a, run().1);
    }

    #[test]
    fn underserved_zones_emit_ghost_plans_after_streak() {
        let (zones, geo, _) = build_zones(&generate_cities(1, 1));
        let mut s = GrowthSimulation::new(zones, Arc::new(Shared::default()));
        for t in 0..120 {
            s.step(t);
        }
        let snap = s.snapshot(120);
        assert!(!snap.pending.is_empty() && snap.road_reserve > 0.0);
        let (a, b) = plan_segment(&geo, snap.pending[0].zone.0 as usize).unwrap();
        assert!(a != b);
    }

    #[test]
    fn block_height_monotone() {
        assert!(block_height(0.0) < block_height(100.0) && block_height(100.0) < block_height(1000.0));
    }
}
