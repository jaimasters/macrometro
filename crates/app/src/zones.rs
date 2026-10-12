//! Growth zones from world_gen cities, run as a sim_core `Simulation`, drawn as extruded blocks by population.
use bevy::prelude::*;
use growth::{CostMatrix, GrowthParams, GrowthSim, RoadAgency, RoadPlan as ZonePlan, Zone, ZoneId, Zones};
use sim_core::{AgencyCommand, Handoff, Sim, Simulation};
use std::sync::{Arc, Mutex};
use transit::{EdgeKind, NodeId, TransitAction, TransitCommand};
use world_gen::{CityRegion, CHUNK_SIZE};

/// Layer height of zone glyphs (flat 2D stack: land 0 < ghosts < zones < lines < stations < trains).
pub const Y_ZONE: f32 = 1.0;

/// Peak zone capacity (residents) at a city centre; falls off to the region edge.
const PEAK_CAPACITY: f64 = 2000.0;

/// Land-use type; drives glyph shape and colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneKind {
    Residential,
    Commercial,
    Industrial,
}

impl ZoneKind {
    pub fn colour(self) -> Color {
        match self {
            ZoneKind::Residential => Color::srgb(0.52, 0.74, 0.58),
            ZoneKind::Commercial => Color::srgb(0.95, 0.62, 0.28),
            ZoneKind::Industrial => Color::srgb(0.60, 0.54, 0.76),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoneGeo {
    pub center: Vec2,
    pub city: u32,
    pub kind: ZoneKind,
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
            let ring = dx.max(dz);
            let kind = if ring == 0 {
                ZoneKind::Commercial
            } else if ring == r && (ch.x + ch.z) % 2 == 0 {
                ZoneKind::Industrial
            } else {
                ZoneKind::Residential
            };
            geo.push(ZoneGeo { center: Vec2::new(ox as f32 + half, oz as f32 + half), city: c.id.0, kind });
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

/// One flat glyph (a small building footprint) of a zone; `slot` orders them so density reveals them in turn.
#[derive(Component)]
pub struct ZoneBlock(pub usize);

#[derive(Component)]
pub struct Glyph {
    pub slot: usize,
    pub jitter: f32,
}

/// Glyph slot offsets within a zone (3x3, centre first).
const SLOTS: [(f32, f32); 9] =
    [(0.0, 0.0), (1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0), (1.0, 1.0), (-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0)];
const SLOT_PITCH: f32 = 7.0;

fn hash01(a: u64, b: u64) -> f32 {
    let mut h = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    ((h >> 40) as f32) / (1u64 << 24) as f32
}

/// How many glyphs a zone shows, from its population (density); `jitter` varies neighbours by one.
pub fn glyph_count(pop: f32, jitter: f32) -> usize {
    let d = (pop / PEAK_CAPACITY as f32 * 3.0).clamp(0.0, 1.0).sqrt();
    ((1.0 + 8.0 * d + jitter * 1.5).floor() as usize).clamp(1, 9)
}

pub fn glyph_size(pop: f32, jitter: f32) -> f32 {
    4.0 + 3.5 * (pop / PEAK_CAPACITY as f32 * 3.0).clamp(0.0, 1.0).sqrt() * (0.6 + 0.8 * jitter)
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
    seed: Res<world_gen::WorldSeed>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    // Flat footprints seen from above: squares (residential), circles (commercial), diamonds (industrial).
    let square = meshes.add(Cuboid::new(1.0, 0.3, 1.0));
    let circle = meshes.add(Cylinder::new(0.5, 0.3));
    let mut mat = |k: ZoneKind| mats.add(StandardMaterial { base_color: k.colour(), unlit: true, ..default() });
    let mats3 = [(ZoneKind::Residential, mat(ZoneKind::Residential)), (ZoneKind::Commercial, mat(ZoneKind::Commercial)), (ZoneKind::Industrial, mat(ZoneKind::Industrial))];
    for (i, g) in layout.0.iter().enumerate() {
        let material = mats3.iter().find(|(k, _)| *k == g.kind).unwrap().1.clone();
        let mesh = if g.kind == ZoneKind::Commercial { circle.clone() } else { square.clone() };
        for (slot, (ox, oz)) in SLOTS.iter().enumerate() {
            let jitter = hash01(i as u64, slot as u64);
            let p = g.center + Vec2::new(*ox, *oz) * SLOT_PITCH;
            if world_gen::height(seed.0, p.x as f64, p.y as f64) < world_gen::SEA_LEVEL {
                continue; // no buildings on water
            }
            let rot = if g.kind == ZoneKind::Industrial { Quat::from_rotation_y(std::f32::consts::FRAC_PI_4) } else { Quat::IDENTITY };
            commands.spawn((
                ZoneBlock(i),
                Glyph { slot, jitter },
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform {
                    translation: Vec3::new(
                        g.center.x + ox * SLOT_PITCH + (jitter - 0.5) * 3.0,
                        Y_ZONE,
                        g.center.y + oz * SLOT_PITCH + (hash01(i as u64, slot as u64 + 50) - 0.5) * 3.0,
                    ),
                    rotation: rot,
                    scale: Vec3::ONE,
                },
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
    mut last: Local<Option<u64>>,
    mut q: Query<(&ZoneBlock, &Glyph, &mut Transform, &mut Visibility)>,
) {
    let s = link.snapshots.latest();
    if *last == Some(s.tick) {
        return;
    }
    *last = Some(s.tick);
    for (b, g, mut t, mut v) in &mut q {
        let Some(&pop) = s.pop.get(b.0) else { continue };
        // zone-level jitter (slot 0's) so neighbouring zones differ
        let zone_j = hash01(b.0 as u64, 99);
        *v = if g.slot < glyph_count(pop, zone_j) { Visibility::Inherited } else { Visibility::Hidden };
        let size = glyph_size(pop, g.jitter);
        t.scale = Vec3::new(size, 1.0, size);
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
    fn glyphs_grow_with_density_and_cover_kinds() {
        assert_eq!(glyph_count(0.0, 0.0), 1);
        assert_eq!(glyph_count(PEAK_CAPACITY as f32, 0.9), 9);
        assert!(glyph_count(100.0, 0.0) < glyph_count(1500.0, 0.0));
        assert!(glyph_size(10.0, 0.5) < glyph_size(1900.0, 0.5));
        let (_, geo, _) = build_zones(&generate_cities(1, 3));
        for k in [ZoneKind::Residential, ZoneKind::Commercial] {
            assert!(geo.iter().any(|g| g.kind == k));
        }
    }
}
