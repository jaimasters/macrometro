//! Wires ui_tools commands into transit, draws stations/lines, shows a one-line HUD.
use crate::zones::ZoneLayout;
use bevy::prelude::*;
use std::collections::HashMap;
use economy::{Category, CurrentBudget, Economy, Scorecard, Scorecards, SpendCommand, SpendQueue};
use sim_core::AgencyId;
use transit::{EdgeKind, NodeId, RailClass, TransitAction, TransitCommand, TransitCommands, TransitShares};
use ui_tools::{segment_transform, ActiveAgency, ActiveTool, GroundHeight, ToolAction, ToolCommand};

/// The local player's agency (agency 0 is the city).
pub const PLAYER: AgencyId = AgencyId(1);
const TRAIN_SPEED: f32 = 14.0;
const STATION_COST: f64 = 50.0;
const COST_PER_LEN: f64 = 2.0;

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
                let len = c[0].as_vec2().distance(c[1].as_vec2()).max(1.0) as f64;
                out.push(TransitCommand { agency, action: TransitAction::AddEdge { a: w[0], b: w[1], len, kind: EdgeKind::Rail } });
            }
            out.push(TransitCommand { agency, action: TransitAction::AddLine { class: RailClass::LightRail, stops, alignment_mismatch: 0.0 } });
        }
    }
    (out, fresh)
}

#[derive(Resource)]
struct ToolVisuals {
    cube: Handle<Mesh>,
    station: Handle<StandardMaterial>,
    line: Handle<StandardMaterial>,
    train: Handle<StandardMaterial>,
}

/// A train shuttling back and forth along a line's stops (visual marker; not simulated load).
#[derive(Component)]
struct Train {
    path: Vec<Vec2>,
    dist: f32,
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

#[derive(Component)]
struct Hud;

pub struct ShellPlugin;

impl Plugin for ShellPlugin {
    fn build(&self, app: &mut App) {
        let first_free = app.world().get_resource::<ZoneLayout>().map_or(0, |z| z.0.len() as u32);
        if let Some(mut e) = app.world_mut().get_resource_mut::<Economy>() {
            e.0.add_agency(PLAYER, 0.5);
        }
        app.insert_resource(StationMap::new(first_free))
            .insert_resource(ActiveAgency(PLAYER))
            .add_systems(Startup, setup_visuals)
            .add_systems(Update, (apply_tool_commands, move_trains, stub_metrics, update_hud));
    }
}

fn setup_visuals(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    commands.insert_resource(ToolVisuals {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        station: mats.add(Color::srgb(0.95, 0.25, 0.25)),
        line: mats.add(Color::srgb(0.25, 0.55, 0.95)),
        train: mats.add(Color::srgb(1.0, 0.85, 0.2)),
    });
    commands.spawn((
        Hud,
        Text::new(""),
        TextColor(Color::WHITE),
        Node { position_type: PositionType::Absolute, left: Val::Px(8.0), top: Val::Px(8.0), ..default() },
    ));
}

fn apply_tool_commands(
    mut cmds: MessageReader<ToolCommand>,
    mut transit: ResMut<TransitCommands>,
    mut map: ResMut<StationMap>,
    layout: Res<ZoneLayout>,
    ground: Res<GroundHeight>,
    vis: Option<Res<ToolVisuals>>,
    mut spend: ResMut<SpendQueue>,
    mut commands: Commands,
) {
    let Some(vis) = vis else { return };
    let centres: Vec<Vec2> = layout.0.iter().map(|g| g.center).collect();
    let y = |p: Vec2| (ground.0)(p.x, p.y);
    for c in cmds.read() {
        let (tc, fresh) = to_transit(c, &mut map, &centres);
        transit.0.extend(tc);
        let amount = match &c.action {
            ToolAction::PlaceStation { .. } => STATION_COST * fresh.len() as f64,
            ToolAction::DrawLine { cells } => {
                STATION_COST * fresh.len() as f64
                    + COST_PER_LEN * cells.windows(2).map(|w| w[0].as_vec2().distance(w[1].as_vec2()) as f64).sum::<f64>()
            }
        };
        if amount > 0.0 {
            spend.0.push(SpendCommand { agency: c.agency, category: Category::Construction, amount });
        }
        for cell in fresh {
            let p = cell.as_vec2();
            commands.spawn((
                Mesh3d(vis.cube.clone()),
                MeshMaterial3d(vis.station.clone()),
                Transform::from_xyz(p.x, y(p) + 2.0, p.y).with_scale(Vec3::new(2.5, 4.0, 2.5)),
            ));
        }
        if let ToolAction::DrawLine { cells } = &c.action {
            let path: Vec<Vec2> = cells.iter().map(|c| c.as_vec2()).collect();
            let len: f32 = path.windows(2).map(|w| w[0].distance(w[1])).sum();
            for dist in [0.0, len] {
                commands.spawn((
                    Train { path: path.clone(), dist },
                    Mesh3d(vis.cube.clone()),
                    MeshMaterial3d(vis.train.clone()),
                    Transform::from_scale(Vec3::new(4.0, 2.5, 2.0)),
                ));
            }
            for w in cells.windows(2) {
                let (a, b) = (w[0].as_vec2(), w[1].as_vec2());
                commands.spawn((
                    Mesh3d(vis.cube.clone()),
                    MeshMaterial3d(vis.line.clone()),
                    segment_transform(a, b, y((a + b) * 0.5) + 0.8, 1.2, 0.5),
                ));
            }
        }
    }
}

fn move_trains(time: Res<Time>, ground: Res<GroundHeight>, mut q: Query<(&mut Train, &mut Transform)>) {
    for (mut t, mut tf) in &mut q {
        t.dist += TRAIN_SPEED * time.delta_secs();
        let p = ping_pong(&t.path, t.dist);
        tf.translation = Vec3::new(p.x, (ground.0)(p.x, p.y) + 2.2, p.y);
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

fn update_hud(
    tool: Res<ActiveTool>,
    eco: Res<Economy>,
    budget: Res<CurrentBudget>,
    mut hud: Single<&mut Text, With<Hud>>,
) {
    let (amount, reserve) = budget
        .0
        .as_ref()
        .map_or((0.0, 0.0), |b| (b.per_agency.get(&PLAYER).map_or(0.0, |a| a.amount), b.road_reserve));
    let (spent, standing) = eco
        .0
        .agencies
        .get(&PLAYER)
        .map_or((0.0, 0.0), |a| (a.spend.values().sum::<f64>(), a.standing));
    hud.0 = format!(
        "budget {amount:.0}  spent {spent:.0}  standing {standing:.2}  city road reserve {reserve:.0}\ntool: {:?}   [1] station  [2] line (click stops, Enter to finish)  [Esc] none   RMB orbit, WASD pan, wheel zoom",
        *tool
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::AgencyId;

    fn cmd(a: u32, action: ToolAction) -> ToolCommand {
        ToolCommand { agency: AgencyId(a), action }
    }

    #[test]
    fn ping_pong_bounces() {
        let p = [Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)];
        assert_eq!(ping_pong(&p, 5.0), Vec2::new(5.0, 0.0));
        assert_eq!(ping_pong(&p, 15.0), Vec2::new(10.0, 5.0));
        assert_eq!(ping_pong(&p, 25.0), Vec2::new(10.0, 5.0)); // 25 -> 40-25=15 from start
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
    fn draw_line_makes_rail_edges_then_line_with_valid_nodes() {
        let centres = [Vec2::ZERO];
        let mut m = StationMap::new(1);
        let c = cmd(1, ToolAction::DrawLine { cells: vec![IVec2::new(0, 10), IVec2::new(30, 10), IVec2::new(60, 10)] });
        let (v, fresh) = to_transit(&c, &mut m, &centres);
        assert_eq!(fresh.len(), 3);
        // Apply to a real network to prove the sequence is accepted (zone node 0 first).
        let mut net = transit::Network::default();
        net.apply(&TransitCommand { agency: AgencyId(0), action: TransitAction::AddNode }).unwrap();
        for t in &v {
            net.apply(t).unwrap_or_else(|e| panic!("{t:?}: {e:?}"));
        }
        assert_eq!(net.lines.len(), 1);
        assert_eq!(net.lines[0].stops.len(), 3);
    }
}
