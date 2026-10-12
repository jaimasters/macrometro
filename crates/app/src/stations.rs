//! Station presentation: flat platform footprints (rounded bars along the serving lines), filled with the line
//! colours, a crowding gauge, auto-name labels when zoomed in, hover/selection/delete highlights.
//! Pure geometry in `groups`/`crowding_colour`; the ECS part rebuilds station visuals when stations or lines change.
use crate::camera::MapCam;
use crate::layout::station_name;
use crate::shell::{line_colour, Lines, StationMap};
use crate::zones::{is_built, GrowthLink, ZoneLayout};
use bevy::prelude::*;
use std::collections::HashMap;
use transit::TransitNet;
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
/// Screen pixels kept free at the bottom for the palette and hint.
const TOOLBAR_CLEAR: f32 = 130.0;
/// Whole platform block across the track (all bars plus gaps), sized to sit inside 3 track widths at rail zoom.
const BLOCK: f32 = 2.4;
const GAP: f32 = 0.3;
const OUTLINE: f32 = 0.4;
/// Zoom tiers (view height): above FAR_VIEW a transit-map marker, above NEAR_VIEW a compact platform, else the full island.
pub const FAR_VIEW: f32 = 150.0;
pub const NEAR_VIEW: f32 = 60.0;

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    Far,
    Mid,
    Near,
}

/// Drawn width of one track at this zoom: individual rails when zoomed in, the line ribbon otherwise.
pub fn track_width(view_h: f32) -> f32 {
    if view_h < crate::rails::RAIL_VIEW { crate::rails::RAIL_PITCH } else { crate::rails::LINE_WIDTH }
}

/// Footprint (along, across the track) of a station with `np` platforms and service `level` at this zoom.
/// Mirrors the meshes `rebuild` spawns for each tier.
pub fn footprint(view_h: f32, np: u8, level: usize) -> Vec2 {
    let len = platform_len(level);
    match tier(view_h) {
        Tier::Near => Vec2::new(len + OUTLINE, BLOCK + OUTLINE),
        Tier::Mid => Vec2::new(len + 1.0, 2.0),
        Tier::Far => Vec2::splat(marker_size(view_h)),
    }
    .max(Vec2::new(0.0, platform_width(np) + OUTLINE))
}

pub fn tier(view_h: f32) -> Tier {
    if view_h > FAR_VIEW { Tier::Far } else if view_h > NEAR_VIEW { Tier::Mid } else { Tier::Near }
}

/// Far-marker diameter in world units: about a line wide, growing with the view but never huge.
pub fn marker_size(view_h: f32) -> f32 {
    (view_h * 0.022).clamp(3.2, 7.0)
}

/// Width of one platform bar: slimmer when a station has several side by side.
pub fn platform_width(n: u8) -> f32 {
    let n = n.max(1) as f32;
    (BLOCK - (n - 1.0) * GAP) / n
}

/// Distance between neighbouring platform bar centres.
pub fn platform_pitch(n: u8) -> f32 {
    platform_width(n) + GAP
}

/// Sideways offset of platform `slot` of `n` from the station centre line.
pub fn platform_offset(slot: u8, n: u8) -> f32 {
    (slot as f32 - (n as f32 - 1.0) / 2.0) * platform_pitch(n)
}

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
struct MarkerRing;
#[derive(Component)]
struct MarkerFill;
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
    white: Handle<StandardMaterial>,
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
            .add_systems(Update, (select_station, rebuild, update_sidings, scale_with_zoom, highlight, update_gauges, place_labels).chain());
    }
}

fn setup_mats(mut commands: Commands, mut m: ResMut<Assets<StandardMaterial>>) {
    let mut flat = |c: Color| m.add(StandardMaterial { base_color: c, unlit: true, ..default() });
    commands.insert_resource(Mats {
        ink: flat(INK),
        stub: flat(STUB),
        white: flat(Color::WHITE),
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
    net: Res<TransitNet>,
    map: Res<StationMap>,
    mut plat_sig: Local<Vec<u8>>,
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
    let sig: Vec<u8> = stations.0.iter().map(|c| map.node(*c).map_or(1, |n| net.0.platform_count(n))).collect();
    if !(stations.is_changed() || lines.is_changed() || sig != *plat_sig) {
        return;
    }
    *plat_sig = sig;
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
        let np = map.node(cell).map_or(1, |n| net.0.platform_count(n));
        let bw = platform_width(np);
        for g in &gs {
            let a = g.angle;
            let dir = Vec2::new(a.cos(), a.sin());
            let perp = Vec2::new(-dir.y, dir.x);
            // hover/select ring around the whole platform block
            let ring = stadium(&mut meshes, &mut cache, len + 1.5, BLOCK + 1.5);
            kids.push(commands.spawn((Mesh3d(ring), MeshMaterial3d(mats.ring_hover.clone()), pose(Vec3::new(0.0, Y_RING, 0.0), a), Ring(cell), Visibility::Hidden)).id());
            // one bar per platform, each striped in the serving lines' colours
            for slot in 0..np {
                let po = platform_offset(slot, np);
                let outline = stadium(&mut meshes, &mut cache, len + OUTLINE, bw + OUTLINE);
                kids.push(commands.spawn((Mesh3d(outline), MeshMaterial3d(mats.ink.clone()), pose(Vec3::new(perp.x * po, Y_OUTLINE, perp.y * po), a))).id());
                let n = g.lines.len().max(1);
                for k in 0..n {
                    let sw = bw / n as f32;
                    let off = po + (k as f32 - (n as f32 - 1.0) / 2.0) * sw;
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
                    Transform { translation: Vec3::new(0.0, Y_TRACK, 0.0), rotation: flat_rot, scale: Vec3::new(full + 0.6, 1.0, 0.6) },
                ))
                .id(),
        );
        kids.push(
            commands
                .spawn((
                    GaugeFill { cell, dir, full, level },
                    Mesh3d(meshes.add(Cuboid::new(1.0, 0.1, 1.0))),
                    MeshMaterial3d(mats.gauge[0].clone()),
                    Transform { translation: Vec3::new(0.0, Y_GAUGE, 0.0), rotation: flat_rot, scale: Vec3::new(0.01, 1.0, 0.4) },
                ))
                .id(),
        );
        for k in &kids {
            commands.entity(*k).insert(Tier::Near);
        }
        let ga = gs[0].angle;
        // mid: one compact platform rectangle along the track
        let mid_len = platform_len(level) + 1.0;
        let mo = meshes.add(Capsule2d::new(1.0, (mid_len - 2.0).max(0.01)));
        kids.push(commands.spawn((Tier::Mid, Visibility::Hidden, Mesh3d(mo.clone()), MeshMaterial3d(mats.ink.clone()), { let mut t = pose(Vec3::new(0.0, Y_OUTLINE, 0.0), ga); t.scale = Vec3::new(1.0, 1.0, 1.0); t })).id());
        let mi = meshes.add(Capsule2d::new(0.7, (mid_len - 2.0).max(0.01)));
        kids.push(commands.spawn((Tier::Mid, Visibility::Hidden, Mesh3d(mi), MeshMaterial3d(mats.white.clone()), pose(Vec3::new(0.0, Y_FILL, 0.0), ga))).id());
        // far: transit-map marker, white disc with a dark outline (sized per frame)
        let disc = meshes.add(Cylinder::new(0.5, 0.2));
        kids.push(commands.spawn((Tier::Far, MarkerRing, Visibility::Hidden, Mesh3d(disc.clone()), MeshMaterial3d(mats.ink.clone()), Transform::from_xyz(0.0, Y_OUTLINE, 0.0))).id());
        kids.push(commands.spawn((Tier::Far, MarkerFill, Visibility::Hidden, Mesh3d(disc), MeshMaterial3d(mats.white.clone()), Transform::from_xyz(0.0, Y_FILL, 0.0))).id());
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

/// Siding spur drawn beside a station that has one, with its parked trains (near zoom).
#[derive(Component)]
struct Siding(IVec2);

fn update_sidings(
    stations: Res<Stations>,
    map: Res<StationMap>,
    trains: Res<crate::rails::Trains>,
    mats: Option<Res<Mats>>,
    mut meshes: ResMut<Assets<Mesh>>,
    old: Query<(Entity, &Siding)>,
    mut commands: Commands,
    mut cache: Local<HashMap<IVec2, usize>>,
) {
    let Some(mats) = mats else { return };
    for &c in &stations.0 {
        let Some(n) = map.node(c) else { continue };
        let want = trains.0.sidings.contains_key(&n.0).then(|| 1 + trains.0.parked_at(n));
        if cache.get(&c).copied() == want.map(|w| w) && want.is_some() == cache.contains_key(&c) {
            continue;
        }
        for (e, s) in &old {
            if s.0 == c {
                commands.entity(e).despawn();
            }
        }
        match want {
            None => {
                cache.remove(&c);
            }
            Some(w) => {
                cache.insert(c, w);
                let p = c.as_vec2() + Vec2::new(0.0, 7.0);
                let bar = meshes.add(Cuboid::new(1.0, 0.1, 1.0));
                commands.spawn((Siding(c), Tier::Near, Mesh3d(bar.clone()), MeshMaterial3d(mats.ink.clone()), Transform { translation: Vec3::new(p.x, Y_FILL, p.y), scale: Vec3::new(14.0, 1.0, 0.5), ..default() }));
                for k in 0..w - 1 {
                    commands.spawn((Siding(c), Tier::Near, Mesh3d(bar.clone()), MeshMaterial3d(mats.white.clone()), Transform { translation: Vec3::new(p.x - 4.0 + 6.0 * k as f32, Y_FILL + 0.1, p.y), scale: Vec3::new(5.0, 1.0, 1.4), ..default() }));
                }
            }
        }
    }
}

fn scale_with_zoom(
    cam: Option<Res<MapCam>>,
    mut tiers: Query<(&Tier, &mut Visibility)>,
    mut ring: Query<&mut Transform, (With<MarkerRing>, Without<MarkerFill>)>,
    mut fill: Query<&mut Transform, (With<MarkerFill>, Without<MarkerRing>)>,
) {
    let view = cam.map_or(100.0, |c| c.view_h);
    let now = tier(view);
    for (t, mut v) in &mut tiers {
        let want = if *t == now { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
    }
    let d = marker_size(view);
    for mut t in &mut ring {
        t.scale = Vec3::new(d, 1.0, d);
    }
    for mut t in &mut fill {
        t.scale = Vec3::new(d * 0.68, 1.0, d * 0.68);
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
        t.scale = Vec3::new(fill, 1.0, 0.4);
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
    // keep clear of the bottom toolbar + hint
    let floor = camera.logical_viewport_size().map_or(f32::MAX, |s| s.y - TOOLBAR_CLEAR);
    for (l, mut node, mut v) in &mut q {
        let pos = camera.world_to_viewport(tf, Vec3::new(l.world.x, 0.0, l.world.y)).ok().filter(|p| p.y < floor);
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
    fn platform_bars_are_distinct_and_centred() {
        assert_eq!(platform_offset(0, 1), 0.0);
        let offs: Vec<f32> = (0..3).map(|i| platform_offset(i, 3)).collect();
        assert!(offs[0] < offs[1] && offs[1] < offs[2] && (offs[0] + offs[2]).abs() < 1e-6 && offs[1] == 0.0);
        assert!(offs[1] - offs[0] > platform_width(3), "bars do not touch");
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

    #[test]
    fn station_footprint_fits_track_gauge_and_platform_at_every_zoom() {
        for view in [30.0, 75.0, 120.0, 400.0, 3000.0] {
            let tw = track_width(view);
            for np in [1u8, 2] {
                for level in [0usize, 1, 3] {
                    let f = footprint(view, np, level);
                    assert!(f.y < 3.0 * tw, "view {view} np {np}: across {} >= 3x track {tw}", f.y);
                    assert!(f.x <= platform_len(level) + tw, "view {view} level {level}: along {} overruns platform", f.x);
                }
            }
        }
    }

    #[test]
    fn zoom_tiers_switch_at_their_thresholds_and_the_marker_stays_bounded() {
        assert_eq!(tier(30.0), Tier::Near);
        assert_eq!(tier(NEAR_VIEW + 1.0), Tier::Mid);
        assert_eq!(tier(FAR_VIEW + 1.0), Tier::Far);
        assert_eq!(marker_size(10.0), 3.2);
        assert_eq!(marker_size(5000.0), 7.0);
        assert!(marker_size(300.0) > marker_size(150.0));
    }
}
