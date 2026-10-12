//! Wires ui_tools commands into transit and economy, and draws the player's network as a flat transit map:
//! white stations with dark outlines, thick coloured octilinear lines, capsule trains.
use crate::camera::MapCam;
use crate::zones::ZoneLayout;
use bevy::prelude::*;
use economy::{Category, CurrentBudget, Economy, Scorecard, Scorecards, SpendCommand, SpendQueue};
use sim_core::AgencyId;
use sound::{SoundEvent, SoundKind};
use std::collections::HashMap;
use transit::{EdgeKind, NodeId, RailClass, TransitAction, TransitCommand, TransitCommands, TransitShares};
use ui_tools::{octilinear, path_len, route, segment_transform, ActiveAgency, ToolAction, ToolCommand};

/// The local player's agency (agency 0 is the city).
pub const PLAYER: AgencyId = AgencyId(1);
const TRAIN_SPEED: f32 = 14.0;
const STATION_COST: f64 = 50.0;
const COST_PER_LEN: f64 = 2.0;

// Flat layer stack (world y): land 0 < ghosts 0.5 < zones 1 < lines 2 < preview 2.6 < station outline 3 < station 3.3 < train 4.5.
const Y_LINE: f32 = 2.0;
const Y_STATION_OUT: f32 = 3.0;
const Y_STATION: f32 = 3.3;
const Y_TRAIN: f32 = 4.5;
const LINE_WIDTH: f32 = 3.2;
const LINE_COLOURS: [(f32, f32, f32); 5] =
    [(0.15, 0.45, 0.90), (0.88, 0.25, 0.30), (0.10, 0.62, 0.58), (0.90, 0.72, 0.10), (0.55, 0.35, 0.75)];

/// Grid cell -> transit node for player-placed stations. Zone nodes occupy 0..n, stations follow.
#[derive(Resource)]
pub struct StationMap {
    next: u32,
    by_cell: HashMap<IVec2, NodeId>,
}

impl StationMap {
    pub fn new(first_free: u32) -> Self {
        Self { next: first_free, by_cell: HashMap::new() }
    }
}

fn nearest_zone(centres: &[Vec2], p: Vec2) -> Option<usize> {
    (0..centres.len()).min_by(|&a, &b| centres[a].distance(p).total_cmp(&centres[b].distance(p)))
}

/// Translate a player command into agency-tagged transit commands (D-8). A new station is a node
/// joined to its nearest zone by a road edge so rail between stations can change zone costs.
/// Returns the commands and the cells that became new stations.
pub fn to_transit(cmd: &ToolCommand, map: &mut StationMap, centres: &[Vec2]) -> (Vec<TransitCommand>, Vec<IVec2>) {
    let (agency, mut out, mut fresh) = (cmd.agency, vec![], vec![]);
    let mut node = |cell: IVec2, out: &mut Vec<TransitCommand>, fresh: &mut Vec<IVec2>| -> NodeId {
        if let Some(n) = map.by_cell.get(&cell) {
            return *n;
        }
        let n = NodeId(map.next);
        map.next += 1;
        map.by_cell.insert(cell, n);
        fresh.push(cell);
        out.push(TransitCommand { agency, action: TransitAction::AddNode });
        let p = cell.as_vec2();
        if let Some(z) = nearest_zone(centres, p) {
            let len = centres[z].distance(p).max(1.0) as f64;
            out.push(TransitCommand { agency, action: TransitAction::AddEdge { a: n, b: NodeId(z as u32), len, kind: EdgeKind::Road } });
        }
        n
    };
    match &cmd.action {
        ToolAction::PlaceStation { cell } => {
            node(*cell, &mut out, &mut fresh);
        }
        ToolAction::DrawLine { cells } => {
            let stops: Vec<NodeId> = cells.iter().map(|&c| node(c, &mut out, &mut fresh)).collect();
            for (w, c) in stops.windows(2).zip(cells.windows(2)) {
                let len = path_len(&octilinear(c[0].as_vec2(), c[1].as_vec2())).max(1.0) as f64;
                out.push(TransitCommand { agency, action: TransitAction::AddEdge { a: w[0], b: w[1], len, kind: EdgeKind::Rail } });
            }
            out.push(TransitCommand { agency, action: TransitAction::AddLine { class: RailClass::LightRail, stops, alignment_mismatch: 0.0 } });
        }
    }
    (out, fresh)
}

#[derive(Resource)]
struct ToolVisuals {
    cylinder: Handle<Mesh>,
    cuboid: Handle<Mesh>,
    capsule: Handle<Mesh>,
    station_out: Handle<StandardMaterial>,
    station: Handle<StandardMaterial>,
    train: Handle<StandardMaterial>,
    lines: Vec<Handle<StandardMaterial>>,
}

#[derive(Resource, Default)]
struct LineCount(usize);

/// A train shuttling back and forth along a line's route (visual marker; not simulated load).
#[derive(Component)]
struct Train {
    path: Vec<Vec2>,
    dist: f32,
    line: usize,
    /// Completed end-to-end legs, to fire an arrival sound at each terminus.
    leg: i64,
}

/// Position after travelling `d` along `path`, bouncing at both ends.
pub fn ping_pong(path: &[Vec2], d: f32) -> Vec2 {
    let total: f32 = path.windows(2).map(|w| w[0].distance(w[1])).sum();
    if path.is_empty() || total <= 0.0 {
        return path.first().copied().unwrap_or(Vec2::ZERO);
    }
    let m = d.rem_euclid(2.0 * total);
    let mut rest = if m > total { 2.0 * total - m } else { m };
    for w in path.windows(2) {
        let l = w[0].distance(w[1]);
        if rest <= l {
            return w[0].lerp(w[1], rest / l.max(1e-6));
        }
        rest -= l;
    }
    *path.last().unwrap()
}

pub struct ShellPlugin;

impl Plugin for ShellPlugin {
    fn build(&self, app: &mut App) {
        let first_free = app.world().get_resource::<ZoneLayout>().map_or(0, |z| z.0.len() as u32);
        if let Some(mut e) = app.world_mut().get_resource_mut::<Economy>() {
            e.0.add_agency(PLAYER, 0.5);
        }
        app.insert_resource(StationMap::new(first_free))
            .init_resource::<LineCount>()
            .insert_resource(ActiveAgency(PLAYER))
            .add_systems(Startup, setup_visuals)
            .add_systems(Update, (apply_tool_commands, move_trains, stub_metrics));
    }
}

fn setup_visuals(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    let mut flat = |c: Color| mats.add(StandardMaterial { base_color: c, unlit: true, ..default() });
    commands.insert_resource(ToolVisuals {
        cylinder: meshes.add(Cylinder::new(1.0, 1.0)),
        cuboid: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        capsule: meshes.add(Capsule3d::new(2.0, 3.5)),
        station_out: flat(Color::srgb(0.10, 0.11, 0.14)),
        station: flat(Color::WHITE),
        train: flat(Color::srgb(0.98, 0.98, 0.98)),
        lines: LINE_COLOURS.iter().map(|&(r, g, b)| flat(Color::srgb(r, g, b))).collect(),
    });
}

#[allow(clippy::too_many_arguments)]
fn apply_tool_commands(
    mut cmds: MessageReader<ToolCommand>,
    mut transit: ResMut<TransitCommands>,
    mut map: ResMut<StationMap>,
    mut lines: ResMut<LineCount>,
    layout: Res<ZoneLayout>,
    cam: Option<Res<MapCam>>,
    vis: Option<Res<ToolVisuals>>,
    mut spend: ResMut<SpendQueue>,
    mut sound: MessageWriter<SoundEvent>,
    mut commands: Commands,
) {
    let Some(vis) = vis else { return };
    let centres: Vec<Vec2> = layout.0.iter().map(|g| g.center).collect();
    let pan = |p: Vec2| cam.as_ref().map_or(0.0, |c| ((p.x - c.focus.x) / c.view_h).clamp(-1.0, 1.0));
    for c in cmds.read() {
        let (tc, fresh) = to_transit(c, &mut map, &centres);
        transit.0.extend(tc);
        let route_len = match &c.action {
            ToolAction::PlaceStation { .. } => 0.0,
            ToolAction::DrawLine { cells } => path_len(&route(cells)) as f64,
        };
        let amount = STATION_COST * fresh.len() as f64 + COST_PER_LEN * route_len;
        if amount > 0.0 {
            spend.0.push(SpendCommand { agency: c.agency, category: Category::Construction, amount });
        }
        for cell in fresh {
            let p = cell.as_vec2();
            for (mat, r, y) in [(&vis.station_out, 5.4, Y_STATION_OUT), (&vis.station, 3.9, Y_STATION)] {
                commands.spawn((
                    Mesh3d(vis.cylinder.clone()),
                    MeshMaterial3d(mat.clone()),
                    Transform::from_xyz(p.x, y, p.y).with_scale(Vec3::new(r, 0.3, r)),
                ));
            }
            sound.write(SoundEvent { kind: SoundKind::StationPlaced, pitch: cell.x / 4 + cell.y / 4, pan: pan(p) });
        }
        if let ToolAction::DrawLine { cells } = &c.action {
            let line = lines.0;
            lines.0 += 1;
            let mat = vis.lines[line % vis.lines.len()].clone();
            let path = route(cells);
            for w in path.windows(2) {
                commands.spawn((
                    Mesh3d(vis.cuboid.clone()),
                    MeshMaterial3d(mat.clone()),
                    segment_transform(w[0], w[1], Y_LINE, LINE_WIDTH, 0.3),
                ));
            }
            for p in &path {
                // round joints so bends are clean
                commands.spawn((
                    Mesh3d(vis.cylinder.clone()),
                    MeshMaterial3d(mat.clone()),
                    Transform::from_xyz(p.x, Y_LINE, p.y).with_scale(Vec3::new(LINE_WIDTH / 2.0, 0.3, LINE_WIDTH / 2.0)),
                ));
            }
            let len = path_len(&path);
            for dist in [0.0, len] {
                commands.spawn((
                    Train { path: path.clone(), dist, line, leg: 0 },
                    Mesh3d(vis.capsule.clone()),
                    MeshMaterial3d(vis.train.clone()),
                    Transform::from_xyz(path[0].x, Y_TRAIN, path[0].y),
                ));
            }
            sound.write(SoundEvent { kind: SoundKind::LineCreated, pitch: line as i32, pan: pan(path[0]) });
        }
    }
}

fn move_trains(
    time: Res<Time>,
    cam: Option<Res<MapCam>>,
    mut sound: MessageWriter<SoundEvent>,
    mut q: Query<(&mut Train, &mut Transform)>,
) {
    for (mut t, mut tf) in &mut q {
        t.dist += TRAIN_SPEED * time.delta_secs();
        let p = ping_pong(&t.path, t.dist);
        let ahead = ping_pong(&t.path, t.dist + 0.5);
        let d = ahead - p;
        tf.translation = Vec3::new(p.x, Y_TRAIN, p.y);
        // capsule axis (Y) laid along the heading in the XZ plane
        tf.rotation = Quat::from_rotation_y(-d.y.atan2(d.x)) * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let total = path_len(&t.path).max(1.0);
        let leg = (t.dist / total).floor() as i64;
        if leg != t.leg {
            t.leg = leg;
            let pan = cam.as_ref().map_or(0.0, |c| ((p.x - c.focus.x) / c.view_h).clamp(-1.0, 1.0));
            sound.write(SoundEvent { kind: SoundKind::TrainArrive, pitch: t.line as i32 * 2 + 1, pan });
        }
    }
}

/// Stand-in metrics until a real producer exists (cross-crate need): player scorecard from transit shares,
/// car share as the complement.
fn stub_metrics(shares: Res<TransitShares>, mut cards: ResMut<Scorecards>, mut car: ResMut<economy::CarShare>) {
    if !shares.is_changed() || shares.0.is_empty() {
        return;
    }
    let mean = shares.0.iter().sum::<f64>() / shares.0.len() as f64;
    cards.0 = vec![(PLAYER, Scorecard { accessibility: mean, trip_time: mean, transit_share: mean, crowding: 0.8, emissions: mean, equity: mean })];
    car.0 = (1.0 - mean).clamp(0.0, 1.0);
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
    fn ping_pong_bounces() {
        let p = [Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)];
        assert_eq!(ping_pong(&p, 5.0), Vec2::new(5.0, 0.0));
        assert_eq!(ping_pong(&p, 15.0), Vec2::new(10.0, 5.0));
        assert_eq!(ping_pong(&p, 25.0), Vec2::new(10.0, 5.0));
        assert_eq!(ping_pong(&p, 40.0), Vec2::ZERO);
        assert_eq!(ping_pong(&[], 3.0), Vec2::ZERO);
    }

    #[test]
    fn place_station_adds_node_and_link_once() {
        let centres = [Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)];
        let mut m = StationMap::new(2);
        let c = cmd(7, ToolAction::PlaceStation { cell: IVec2::new(90, 0) });
        let (v, fresh) = to_transit(&c, &mut m, &centres);
        assert_eq!(fresh, vec![IVec2::new(90, 0)]);
        assert_eq!(v.len(), 2);
        assert!(v.iter().all(|t| t.agency == AgencyId(7)));
        assert_eq!(v[1].action, TransitAction::AddEdge { a: NodeId(2), b: NodeId(1), len: 10.0, kind: EdgeKind::Road });
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
}
