//! Station presentation: flat platform footprints (rounded bars along the serving lines), filled with the line
//! colours, a crowding gauge, auto-name labels when zoomed in, hover/selection/delete highlights.
//! Pure geometry in `groups`/`crowding_colour`; the ECS part rebuilds station visuals when stations or lines change.
use crate::camera::MapCam;
use crate::layout::station_name;
use crate::shell::{line_colour, Lines};
use crate::zones::{is_built, GrowthLink, ZoneLayout};
use bevy::prelude::*;
use std::collections::HashMap;
use ui_tools::{route, station_at, ActiveTool, CursorWorld, Stations};

const INK: Color = Color::srgb(0.10, 0.11, 0.14);
const STUB: Color = Color::srgb(0.72, 0.73, 0.76);
const Y_RING: f32 = 2.8;
const Y_OUTLINE: f32 = 3.0;
const Y_FILL: f32 = 3.2;
const Y_TRACK: f32 = 3.5;
const Y_GAUGE: f32 = 3.6;
/// Labels show only when the view is at least this zoomed in (view height in world units).
pub const LABEL_VIEW_MAX: f32 = 150.0;
/// Stations/platforms are enlarged on zoom-out so they stay readable (1.0 at/below this view height).
const ZOOM_REF: f32 = 160.0;
const WIDTH: f32 = 3.6;

/// One platform bar: heading (radians in the XZ plane, folded to [0, pi)) and the lines served along it.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub angle: f32,
    pub lines: Vec<usize>,
}

fn fold(a: f32) -> f32 {
    a.rem_euclid(std::f32::consts::PI)
}

/// Platform groups at `cell`: one per distinct line orientation; an unconnected station gets an empty stub group.
pub fn groups(lines: &Lines, cell: IVec2) -> Vec<Group> {
    let mut out: Vec<Group> = vec![];
    for l in &lines.all {
        let Some(si) = l.cells.iter().position(|c| *c == cell) else { continue };
        let path = route(&l.cells);
        let p = cell.as_vec2();
        let Some(j) = path.iter().position(|q| *q == p) else { continue };
        let (a, b) = (path[j.saturating_sub(1)], path[(j + 1).min(path.len() - 1)]);
        let _ = si;
        let d = b - a;
        let ang = fold(d.y.atan2(d.x));
        match out.iter_mut().find(|g| (g.angle - ang).abs() < 0.12 || (g.angle - ang).abs() > std::f32::consts::PI - 0.12) {
            Some(g) => g.lines.push(l.id),
            None => out.push(Group { angle: ang, lines: vec![l.id] }),
        }
    }
    if out.is_empty() {
        out.push(Group { angle: 0.0, lines: vec![] });
    }
    out
}

/// Platform length by service level (distinct lines serving); an unconnected stub is short.
pub fn platform_len(level: usize) -> f32 {
    if level == 0 { 4.0 } else { 6.0 + 3.0 * level.min(4) as f32 }
}

/// Gauge colour: calm while filling, amber past 60%, red past 85%.
pub fn crowding_colour(c: f32) -> Color {
    if c < 0.6 {
        Color::srgb(0.30, 0.78, 0.55)
    } else if c < 0.85 {
        Color::srgb(0.97, 0.70, 0.12)
    } else {
        Color::srgb(0.90, 0.22, 0.22)
    }
}

/// Stand-in crowding in 0..1 until transit publishes waiting passengers (cross-crate need): built-zone population
/// near the station relative to platform capacity.
pub fn crowding_stub(zone_pops: impl Iterator<Item = f32>, level: usize) -> f32 {
    let near: f32 = zone_pops.sum();
    (near / (700.0 * (1.0 + 0.8 * level as f32))).clamp(0.0, 1.0)
}

#[derive(Resource, Default)]
pub struct SelectedStation(pub Option<IVec2>);

#[derive(Component)]
struct StationRoot(#[allow(dead_code)] IVec2);
#[derive(Component)]
struct Ring(IVec2);
#[derive(Component)]
struct GaugeFill {
    cell: IVec2,
    dir: Vec2,
    full: f32,
    level: usize,
}
#[derive(Component)]
struct Label {
    world: Vec2,
}

#[derive(Resource)]
struct Mats {
    ink: Handle<StandardMaterial>,
    stub: Handle<StandardMaterial>,
    track: Handle<StandardMaterial>,
    gauge: [Handle<StandardMaterial>; 3],
    ring_hover: Handle<StandardMaterial>,
    ring_select: Handle<StandardMaterial>,
    ring_delete: Handle<StandardMaterial>,
}

pub struct StationsPlugin;

impl Plugin for StationsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SelectedStation>()
            .add_systems(Startup, setup_mats)
            .add_systems(Update, (select_station, rebuild, scale_with_zoom, highlight, update_gauges, place_labels).chain());
    }
}

fn setup_mats(mut commands: Commands, mut m: ResMut<Assets<StandardMaterial>>) {
    let mut flat = |c: Color| m.add(StandardMaterial { base_color: c, unlit: true, ..default() });
    commands.insert_resource(Mats {
        ink: flat(INK),
        stub: flat(STUB),
        track: flat(Color::srgb(0.20, 0.21, 0.25)),
        gauge: [flat(crowding_colour(0.0)), flat(crowding_colour(0.7)), flat(crowding_colour(1.0))],
        ring_hover: flat(Color::srgb(1.0, 1.0, 1.0)),
        ring_select: flat(Color::srgb(0.25, 0.55, 1.0)),
        ring_delete: flat(Color::srgb(0.90, 0.22, 0.22)),
    });
}

fn stadium(meshes: &mut Assets<Mesh>, cache: &mut HashMap<(i32, i32), Handle<Mesh>>, len: f32, width: f32) -> Handle<Mesh> {
    cache
        .entry(((len * 10.0) as i32, (width * 10.0) as i32))
        .or_insert_with(|| meshes.add(Capsule2d::new(width / 2.0, (len - width).max(0.01))))
        .clone()
}

/// Flat pose for a mesh built in the XY plane (Capsule2d, long axis Y): lie it on the ground, long axis along `angle`.
fn pose(local: Vec3, angle: f32) -> Transform {
    Transform {
        translation: local,
        rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2 - angle) * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
        scale: Vec3::ONE,
    }
}

fn select_station(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    tool: Res<ActiveTool>,
    cursor: Res<CursorWorld>,
    stations: Res<Stations>,
    mut sel: ResMut<SelectedStation>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        sel.0 = None;
    }
    if *tool == ActiveTool::Pan && mouse.just_pressed(MouseButton::Left) {
        // click a station to select it (opens its menu); click empty map to dismiss
        if let Some(p) = cursor.0 {
            sel.0 = station_at(&stations.0, p);
        }
    }
    if sel.0.is_some_and(|c| !stations.0.contains(&c)) {
        sel.0 = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn rebuild(
    stations: Res<Stations>,
    lines: Res<Lines>,
    mats: Option<Res<Mats>>,
    layout: Res<ZoneLayout>,
    seed: Res<world_gen::WorldSeed>,
    old: Query<Entity, Or<(With<StationRoot>, With<Label>)>>,
    mut line_mats: Local<HashMap<usize, Handle<StandardMaterial>>>,
    mut cache: Local<HashMap<(i32, i32), Handle<Mesh>>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let Some(mats) = mats else { return };
    if !(stations.is_changed() || lines.is_changed()) {
        return;
    }
    for e in &old {
        commands.entity(e).despawn();
    }
    for &cell in &stations.0 {
        let gs = groups(&lines, cell);
        let level: usize = gs.iter().map(|g| g.lines.len()).sum();
        let len = platform_len(level);
        let p = cell.as_vec2();
        let root = commands
            .spawn((StationRoot(cell), Transform::from_xyz(p.x, 0.0, p.y), Visibility::default()))
            .id();
        let mut kids = vec![];
        for g in &gs {
            let a = g.angle;
            let dir = Vec2::new(a.cos(), a.sin());
            let perp = Vec2::new(-dir.y, dir.x);
            for (w, y, mat) in [(WIDTH + 5.0, Y_RING, mats.ring_hover.clone()), (WIDTH + 1.6, Y_OUTLINE, mats.ink.clone())] {
                let mesh = stadium(&mut meshes, &mut cache, len + (w - WIDTH), w);
                let t = pose(Vec3::new(0.0, y, 0.0), a);
                let mut e = commands.spawn((Mesh3d(mesh), MeshMaterial3d(mat), t));
                if y == Y_RING {
                    e.insert((Ring(cell), Visibility::Hidden));
                }
                kids.push(e.id());
            }
            // fills: one strip per line, each its line's colour; stub = neutral
            let n = g.lines.len().max(1);
            for k in 0..n {
                let sw = WIDTH / n as f32;
                let off = (k as f32 - (n as f32 - 1.0) / 2.0) * sw;
                let mat = match g.lines.get(k) {
                    Some(&id) => line_mats
                        .entry(id)
                        .or_insert_with(|| materials.add(StandardMaterial { base_color: line_colour(id), unlit: true, ..default() }))
                        .clone(),
                    None => mats.stub.clone(),
                };
                let mesh = stadium(&mut meshes, &mut cache, len, sw.max(1.0));
                kids.push(commands.spawn((Mesh3d(mesh), MeshMaterial3d(mat), pose(Vec3::new(perp.x * off, Y_FILL, perp.y * off), a))).id());
            }
        }
        // gauge along the first platform: dark track + fill that grows with crowding
        let a = gs[0].angle;
        let dir = Vec2::new(a.cos(), a.sin());
        let full = len * 0.72;
        let flat_rot = Quat::from_rotation_y(-a);
        kids.push(
            commands
                .spawn((
                    Mesh3d(meshes.add(Cuboid::new(1.0, 0.1, 1.0))),
                    MeshMaterial3d(mats.track.clone()),
                    Transform { translation: Vec3::new(0.0, Y_TRACK, 0.0), rotation: flat_rot, scale: Vec3::new(full + 0.6, 1.0, 1.0) },
                ))
                .id(),
        );
        kids.push(
            commands
                .spawn((
                    GaugeFill { cell, dir, full, level },
                    Mesh3d(meshes.add(Cuboid::new(1.0, 0.1, 1.0))),
                    MeshMaterial3d(mats.gauge[0].clone()),
                    Transform { translation: Vec3::new(0.0, Y_GAUGE, 0.0), rotation: flat_rot, scale: Vec3::new(0.01, 1.0, 0.7) },
                ))
                .id(),
        );
        commands.entity(root).add_children(&kids);
        // name label (UI node, positioned each frame)
        commands
            .spawn((
                Label { world: p + Vec2::new(0.0, 9.0) },
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(160.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                Visibility::Hidden,
            ))
            .with_child((
                Node { padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)), border_radius: BorderRadius::all(Val::Px(8.0)), ..default() },
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.88)),
                Text::new(station_name(seed.0, &layout.0, p)),
                TextFont { font: crate::hud::ui_font(), font_size: FontSize::Px(12.0), ..default() },
                TextColor(INK),
            ));
    }
}

fn scale_with_zoom(cam: Option<Res<MapCam>>, mut q: Query<&mut Transform, With<StationRoot>>) {
    let k = cam.map_or(1.0, |c| (c.view_h / ZOOM_REF).clamp(1.0, 2.4));
    for mut t in &mut q {
        t.scale = Vec3::new(k, 1.0, k);
    }
}

fn highlight(
    cursor: Res<CursorWorld>,
    stations: Res<Stations>,
    sel: Res<SelectedStation>,
    tool: Res<ActiveTool>,
    mats: Option<Res<Mats>>,
    mut rings: Query<(&Ring, &mut Visibility, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    let Some(mats) = mats else { return };
    let hover = cursor.0.and_then(|p| station_at(&stations.0, p));
    for (r, mut v, mut m) in &mut rings {
        let pick = if hover == Some(r.0) {
            Some(if *tool == ActiveTool::Delete { &mats.ring_delete } else { &mats.ring_hover })
        } else if sel.0 == Some(r.0) {
            Some(&mats.ring_select)
        } else {
            None
        };
        *v = if pick.is_some() { Visibility::Inherited } else { Visibility::Hidden };
        if let Some(h) = pick {
            if m.0 != *h {
                m.0 = h.clone();
            }
        }
    }
}

fn update_gauges(
    link: Option<Res<GrowthLink>>,
    layout: Res<ZoneLayout>,
    mats: Option<Res<Mats>>,
    mut frame: Local<u32>,
    mut q: Query<(&GaugeFill, &mut Transform, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    let (Some(link), Some(mats)) = (link, mats) else { return };
    *frame += 1;
    if *frame % 20 != 1 {
        return;
    }
    let s = link.snapshots.latest();
    for (g, mut t, mut m) in &mut q {
        let c0 = g.cell.as_vec2();
        let pops = layout
            .0
            .iter()
            .enumerate()
            .filter(|(i, z)| z.center.distance(c0) <= 45.0 && s.pop.get(*i).is_some_and(|&p| is_built(p)))
            .map(|(i, _)| s.pop[i]);
        let c = crowding_stub(pops, g.level);
        let fill = (g.full * c).max(0.3);
        // fill grows from the platform's start end
        let start = -g.dir * (g.full / 2.0) + g.dir * (fill / 2.0);
        t.translation = Vec3::new(start.x, Y_GAUGE, start.y);
        t.scale = Vec3::new(fill, 1.0, 0.7);
        let want = &mats.gauge[if c < 0.6 { 0 } else if c < 0.85 { 1 } else { 2 }];
        if m.0 != *want {
            m.0 = want.clone();
        }
    }
}

fn place_labels(
    cam: Option<Res<MapCam>>,
    camera: Option<Single<(&Camera, &GlobalTransform), With<Camera3d>>>,
    mut q: Query<(&Label, &mut Node, &mut Visibility)>,
) {
    let (Some(cam), Some(camera)) = (cam, camera) else { return };
    let (camera, tf) = *camera;
    let show = cam.view_h <= LABEL_VIEW_MAX;
    for (l, mut node, mut v) in &mut q {
        let pos = camera.world_to_viewport(tf, Vec3::new(l.world.x, 0.0, l.world.y)).ok();
        match (show, pos) {
            (true, Some(p)) => {
                node.left = Val::Px(p.x - 80.0);
                node.top = Val::Px(p.y);
                *v = Visibility::Inherited;
            }
            _ => *v = Visibility::Hidden,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_with(cells: &[Vec<IVec2>]) -> Lines {
        let mut l = Lines::default();
        for c in cells {
            l.add(c.clone());
        }
        l
    }

    #[test]
    fn unconnected_station_is_a_short_neutral_stub() {
        let g = groups(&Lines::default(), IVec2::new(4, 4));
        assert_eq!(g, vec![Group { angle: 0.0, lines: vec![] }]);
        assert!(platform_len(0) < platform_len(1));
    }

    #[test]
    fn platform_follows_line_and_interchange_joins_each_line() {
        let a = IVec2::new(0, 0);
        let hub = IVec2::new(40, 0);
        let l = lines_with(&[vec![a, hub, IVec2::new(80, 0)], vec![IVec2::new(40, -40), hub, IVec2::new(40, 40)], vec![a, hub]]);
        let g = groups(&l, hub);
        // east-west line (+ the third line arriving from the west: same orientation) and the north-south line
        assert_eq!(g.len(), 2);
        let ew = g.iter().find(|g| g.angle < 0.1).unwrap();
        assert_eq!(ew.lines, vec![0, 2]);
        let ns = g.iter().find(|g| (g.angle - std::f32::consts::FRAC_PI_2).abs() < 0.1).unwrap();
        assert_eq!(ns.lines, vec![1]);
        // level scales the platform
        assert!(platform_len(3) > platform_len(1));
        assert_eq!(platform_len(9), platform_len(4));
    }

    #[test]
    fn diagonal_line_gets_diagonal_platform() {
        let l = lines_with(&[vec![IVec2::new(0, 0), IVec2::new(40, 40)]]);
        let g = groups(&l, IVec2::new(0, 0));
        assert!((g[0].angle - std::f32::consts::FRAC_PI_4).abs() < 1e-3);
    }

    #[test]
    fn crowding_fills_then_amber_then_red() {
        let c = |p: f32| crowding_stub([p].into_iter(), 1);
        assert!(c(100.0) < c(700.0) && c(700.0) < c(5000.0));
        assert_eq!(c(1e9), 1.0);
        let rgb = |c: Color| c.to_srgba();
        assert!(rgb(crowding_colour(0.3)).green > rgb(crowding_colour(0.3)).red);
        assert!(rgb(crowding_colour(0.7)).red > 0.9 && rgb(crowding_colour(0.7)).green > 0.6);
        assert!(rgb(crowding_colour(0.95)).red > 0.8 && rgb(crowding_colour(0.95)).green < 0.3);
    }
}
