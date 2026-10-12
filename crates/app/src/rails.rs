//! Player lines drawn as rails (N thin parallel rails per segment when zoomed in, one thick stroke when zoomed
//! out) and the trains running on them, rendered from the transit train sim (interpolated between fixed ticks).
//! A dwelling train sits on the specific platform bar it occupies.
use crate::camera::MapCam;
use crate::shell::{line_geometry, LineRec, Lines, RailGeoms, LINE_COLOURS};
use crate::stations::platform_offset;
use bevy::prelude::*;
use sound::{SoundEvent, SoundKind};
use std::collections::HashMap;
use transit::trains::{Phase, TrainSim};
use transit::{LineId, Network, TransitNet};
use ui_tools::{path_len, segment_transform};

const Y_LINE: f32 = 2.0;
const Y_TRAIN: f32 = 4.5;
const LINE_WIDTH: f32 = 3.2;
/// Below this view height lines draw as their individual rails.
pub const RAIL_VIEW: f32 = 90.0;
const RAIL_W: f32 = 0.16;
/// Half the distance between a track's two rails; sleepers run a little wider.
const RAIL_GAP: f32 = 0.3;
const SLEEPER_STEP: f32 = 1.4;
const SLEEPER_W: f32 = 0.35;
pub const RAIL_PITCH: f32 = 1.1;
const CAPSULE_LEN: f32 = 7.5;

#[derive(Component)]
pub struct LineVisual(pub usize);
#[derive(Component)]
struct Thick;
#[derive(Component)]
struct Detail;
#[derive(Component)]
struct TrainVisual {
    id: u32,
    lat: Vec2,
    dwelling: bool,
}

/// The train sim (fixed step); `Trains.0` is the single source of train state.
#[derive(Resource, Default)]
pub struct Trains(pub TrainSim);

/// Route polyline per app line id (for placing trains).
#[derive(Resource, Default)]
struct LinePaths(HashMap<usize, Vec<Vec2>>);

#[derive(Resource)]
struct Visuals {
    cylinder: Handle<Mesh>,
    cuboid: Handle<Mesh>,
    capsule: Handle<Mesh>,
    lines: Vec<Handle<StandardMaterial>>,
    beds: Vec<Handle<StandardMaterial>>,
}

pub struct RailsPlugin;

impl Plugin for RailsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Trains>()
            .init_resource::<LinePaths>()
            .add_systems(Startup, setup)
            .add_systems(FixedUpdate, step_trains)
            .add_systems(Update, (refresh_lines, lod_lines, sync_trains, move_trains).chain());
    }
}

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    let mut flat = |c: Color| mats.add(StandardMaterial { base_color: c, unlit: true, ..default() });
    commands.insert_resource(Visuals {
        cylinder: meshes.add(Cylinder::new(1.0, 1.0)),
        cuboid: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        capsule: meshes.add(Capsule3d::new(2.0, 3.5)),
        lines: LINE_COLOURS.iter().map(|&(r, g, b)| flat(Color::srgb(r, g, b))).collect(),
        beds: LINE_COLOURS.iter().map(|&(r, g, b)| flat(Color::srgb(r * 0.45, g * 0.45, b * 0.45))).collect(),
    });
}

fn step_trains(net: Res<TransitNet>, map: Option<Res<crate::shell::StationMap>>, layout: Option<Res<crate::zones::ZoneLayout>>, mut trains: ResMut<Trains>, link: Option<Res<crate::zones::GrowthLink>>, cp: Option<Res<crate::zones::CityPlan>>, mut tick: Local<u32>) {
    trains.0.step(&net.0);
    // once a second of sim time hand the road closures (trains on level crossings) to the traffic model
    *tick += 1;
    if *tick % 64 == 1 {
        if let (Some(map), Some(layout), Some(link)) = (map, layout, link.as_ref()) {
            // people arrive at each station at the rate the surrounding city would put on transit (est. spread over ~3 minutes)
            let snap = link.snapshots.latest();
            let zones: Vec<(Vec2, f32, f32)> = layout.0.iter().enumerate().map(|(i, g)| (g.center, snap.pop.get(i).copied().unwrap_or(0.0), snap.zone_share.get(i).copied().unwrap_or(0.0))).collect();
            trains.0.arrivals = map.by_cell.iter().map(|(c, n)| (n.0, crate::station_menu::waiting_estimate(c.as_vec2(), &zones) as f64 / 180.0)).collect();
        }
    }
    if *tick % 64 == 0 {
        if let (Some(link), Some(cp)) = (link, cp) {
            let mut closed = vec![0.0f32; cp.plan.graph.edges.len()];
            for (road, f) in trains.0.take_closure() {
                if let Some(c) = closed.get_mut(road as usize) {
                    *c = f;
                }
            }
            *link.shared.closure.lock().unwrap() = closed;
        }
    }
}

fn left(v: Vec2) -> Vec2 {
    Vec2::new(-v.y, v.x).normalize_or_zero()
}

/// `path` shifted sideways by `off` (mitred at bends).
pub fn offset_path(path: &[Vec2], off: f32) -> Vec<Vec2> {
    (0..path.len())
        .map(|i| {
            let a = if i > 0 { left(path[i] - path[i - 1]) } else { Vec2::ZERO };
            let b = if i + 1 < path.len() { left(path[i + 1] - path[i]) } else { Vec2::ZERO };
            path[i] + (a + b).normalize_or_zero() * off
        })
        .collect()
}

/// Point and unit heading at distance `d` along `path` (clamped to its ends).
pub fn point_at(path: &[Vec2], d: f32) -> (Vec2, Vec2) {
    let mut rest = d.max(0.0);
    let mut heading = Vec2::X;
    for w in path.windows(2) {
        let l = w[0].distance(w[1]);
        if l <= 1e-6 {
            continue;
        }
        heading = (w[1] - w[0]) / l;
        if rest <= l {
            return (w[0] + heading * rest, heading);
        }
        rest -= l;
    }
    (path.last().copied().unwrap_or(Vec2::ZERO), heading)
}

/// Lateral rail position (in `RAIL_PITCH` units, + = left of the route direction) for a train going `dir` in `lane`.
pub fn rail_offset(tracks: u8, dir: i8, lane: u8) -> f32 {
    match tracks {
        0 | 1 => 0.0,
        2 => if dir > 0 { -0.5 } else { 0.5 },
        _ => (if dir > 0 { lane as f32 } else { 2.0 + lane as f32 }) - 1.5,
    }
}

/// Track count per hop of `rec` from transit (2 until the line shows up there).
fn hop_tracks(net: &Network, rec: &LineRec) -> Vec<u8> {
    let hops = rec.cells.len().saturating_sub(1);
    net.lines
        .iter()
        .find(|l| l.id == LineId(rec.id as u32))
        .map(|l| l.segments.iter().map(|&s| net.edges[s].tracks).collect::<Vec<u8>>())
        .filter(|v| v.len() == hops)
        .unwrap_or_else(|| vec![2; hops])
}

fn stroke(commands: &mut Commands, vis: &Visuals, id: usize, path: &[Vec2], width: f32, y: f32, mat: &Handle<StandardMaterial>, detail: bool) {
    let tag = |e: &mut EntityCommands| {
        if detail {
            e.insert((Detail, Visibility::Hidden));
        } else {
            e.insert(Thick);
        }
    };
    for w in path.windows(2) {
        let mut e = commands.spawn((LineVisual(id), Mesh3d(vis.cuboid.clone()), MeshMaterial3d(mat.clone()), segment_transform(w[0], w[1], y, width, 0.3)));
        tag(&mut e);
    }
    for p in path {
        let mut e = commands.spawn((
            LineVisual(id),
            Mesh3d(vis.cylinder.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_xyz(p.x, y, p.y).with_scale(Vec3::new(width / 2.0, 0.3, width / 2.0)),
        ));
        tag(&mut e);
    }
}

fn spawn_line(commands: &mut Commands, vis: &Visuals, rec: &LineRec, tracks: &[u8], geoms: &RailGeoms) {
    let k = rec.id % vis.lines.len();
    let (path, idx) = line_geometry(&rec.cells, geoms);
    stroke(commands, vis, rec.id, &path, LINE_WIDTH, Y_LINE, &vis.lines[k], false);
    for (h, &n) in tracks.iter().enumerate() {
        let (i0, i1) = (idx[h], idx[h + 1].max(idx[h]));
        let sub = &path[i0..=i1.min(path.len() - 1)];
        let n = n.max(1);
        stroke(commands, vis, rec.id, sub, n as f32 * RAIL_PITCH + 0.9, Y_LINE + 0.05, &vis.beds[k], true);
        for r in 0..n {
            let off = (r as f32 - (n as f32 - 1.0) / 2.0) * RAIL_PITCH;
            let centre = offset_path(sub, off);
            // two rails per track, and sleepers across them
            for side in [-RAIL_GAP, RAIL_GAP] {
                stroke(commands, vis, rec.id, &offset_path(&centre, side), RAIL_W, Y_LINE + 0.1, &vis.lines[k], true);
            }
            let total = path_len(&centre);
            let mut d = SLEEPER_STEP / 2.0;
            while d < total {
                let (p, h) = point_at(&centre, d);
                let n = left(h) * RAIL_GAP * 1.5;
                let mut e = commands.spawn((LineVisual(rec.id), Mesh3d(vis.cuboid.clone()), MeshMaterial3d(vis.beds[k].clone()), segment_transform(p - n, p + n, Y_LINE + 0.07, SLEEPER_W, 0.1)));
                e.insert((Detail, Visibility::Hidden));
                d += SLEEPER_STEP;
            }
        }
    }
}

/// (Re)draw a line when its stations or any segment's track count change; drop visuals of deleted lines.
fn refresh_lines(
    lines: Res<Lines>,
    net: Res<TransitNet>,
    geoms: Res<RailGeoms>,
    vis: Option<Res<Visuals>>,
    mut paths: ResMut<LinePaths>,
    mut seen: Local<HashMap<usize, (Vec<IVec2>, Vec<u8>)>>,
    old: Query<(Entity, &LineVisual)>,
    mut commands: Commands,
) {
    let Some(vis) = vis else { return };
    if !(lines.is_changed() || net.is_changed()) {
        return;
    }
    let gone: Vec<usize> = seen.keys().copied().filter(|id| !lines.all.iter().any(|l| l.id == *id)).collect();
    for id in &gone {
        seen.remove(id);
        paths.0.remove(id);
    }
    let mut redraw: Vec<usize> = gone;
    for rec in &lines.all {
        let key = (rec.cells.clone(), hop_tracks(&net.0, rec));
        if seen.get(&rec.id) != Some(&key) {
            paths.0.insert(rec.id, line_geometry(&rec.cells, &geoms).0);
            redraw.push(rec.id);
            seen.insert(rec.id, key);
        }
    }
    if redraw.is_empty() {
        return;
    }
    for (e, v) in &old {
        if redraw.contains(&v.0) {
            commands.entity(e).despawn();
        }
    }
    for rec in lines.all.iter().filter(|l| redraw.contains(&l.id)) {
        let tracks = seen[&rec.id].1.clone();
        spawn_line(&mut commands, &vis, rec, &tracks, &geoms);
    }
}

/// Zoomed in: individual rails. Zoomed out: one thick stroke.
fn lod_lines(cam: Option<Res<MapCam>>, mut thick: Query<&mut Visibility, (With<Thick>, Without<Detail>)>, mut detail: Query<&mut Visibility, With<Detail>>) {
    let zoomed_in = cam.is_some_and(|c| c.view_h <= RAIL_VIEW);
    for (mut v, want) in thick.iter_mut().map(|v| (v, !zoomed_in)).chain(detail.iter_mut().map(|v| (v, zoomed_in))) {
        let w = if want { Visibility::Inherited } else { Visibility::Hidden };
        if *v != w {
            *v = w;
        }
    }
}

fn sync_trains(trains: Res<Trains>, vis: Option<Res<Visuals>>, q: Query<(Entity, &TrainVisual)>, mut commands: Commands, mut mats: ResMut<Assets<StandardMaterial>>) {
    let Some(vis) = vis else { return };
    for (e, t) in &q {
        if !trains.0.trains.iter().any(|s| s.id == t.id) {
            commands.entity(e).despawn();
        }
    }
    for s in trains.0.trains.iter().filter(|s| !q.iter().any(|(_, t)| t.id == s.id)) {
        commands.spawn((
            TrainVisual { id: s.id, lat: Vec2::ZERO, dwelling: false },
            Mesh3d(vis.capsule.clone()),
            // own material per train so its body colour can show its load
            MeshMaterial3d(mats.add(StandardMaterial { base_color: load_colour(0.0), unlit: true, ..default() })),
            Transform::from_xyz(0.0, -50.0, 0.0),
        ));
    }
}

/// Body colour by load share: white empty -> red full.
pub fn load_colour(share: f64) -> Color {
    let k = share.clamp(0.0, 1.0) as f32;
    Color::srgb(0.98, 0.98 - 0.72 * k, 0.98 - 0.76 * k)
}

#[allow(clippy::too_many_arguments)]
fn move_trains(
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    trains: Res<Trains>,
    net: Res<TransitNet>,
    paths: Res<LinePaths>,
    cam: Option<Res<MapCam>>,
    mut sound: MessageWriter<SoundEvent>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(&mut TrainVisual, &mut Transform, &MeshMaterial3d<StandardMaterial>)>,
) {
    let alpha = fixed.overstep_fraction() as f64;
    let blend = 1.0 - (-8.0 * time.delta_secs()).exp();
    for (mut tv, mut tf, mh) in &mut q {
        let Some(t) = trains.0.trains.iter().find(|s| s.id == tv.id) else { continue };
        if let Some(mut m) = mats.get_mut(&mh.0) {
            let c = load_colour(t.load / transit::trains::TRAIN_CAPACITY);
            if m.base_color != c {
                m.base_color = c;
            }
        }
        let (Some(path), Some(total)) = (paths.0.get(&(t.line.0 as usize)), trains.0.line_total(t.line)) else { continue };
        let scale = path_len(path) / total.max(1e-6) as f32;
        let front = trains.0.route_pos(t, alpha);
        let centre = (front - t.dir as f64 * t.len / 2.0) as f32 * scale;
        let (pos, fwd) = point_at(path, centre);
        let n = left(fwd);
        let target = match trains.0.on_platform(t) {
            Some((node, slot)) => {
                // platform bars are laid out along the folded heading
                let a = fwd.y.atan2(fwd.x).rem_euclid(std::f32::consts::PI);
                Vec2::new(-a.sin(), a.cos()) * platform_offset(slot, net.0.platform_count(node))
            }
            None => n * rail_offset(trains.0.tracks_under(&net.0, t), t.dir, t.lane) * RAIL_PITCH,
        };
        let lat = tv.lat + (target - tv.lat) * blend;
        tv.lat = lat;
        let p = pos + lat;
        let heading = fwd * t.dir as f32;
        tf.translation = Vec3::new(p.x, Y_TRAIN, p.y);
        tf.rotation = Quat::from_rotation_y(-heading.y.atan2(heading.x)) * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        tf.scale = Vec3::new(0.55, t.len as f32 / CAPSULE_LEN, 0.55);
        let dwelling = matches!(t.phase, Phase::Dwell(_));
        if dwelling && !tv.dwelling {
            let pan = cam.as_ref().map_or(0.0, |c| ((p.x - c.focus.x) / c.view_h).clamp(-1.0, 1.0));
            sound.write(SoundEvent { kind: SoundKind::TrainArrive, pitch: t.line.0 as i32 * 2 + 1, pan });
        }
        tv.dwelling = dwelling;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_at_walks_the_polyline_and_clamps() {
        let p = [Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)];
        assert_eq!(point_at(&p, 5.0).0, Vec2::new(5.0, 0.0));
        assert_eq!(point_at(&p, 15.0), (Vec2::new(10.0, 5.0), Vec2::Y));
        assert_eq!(point_at(&p, 99.0).0, Vec2::new(10.0, 10.0));
        assert_eq!(point_at(&p, -3.0).0, Vec2::ZERO);
    }

    #[test]
    fn rails_split_by_direction_and_lane() {
        assert_eq!(rail_offset(1, 1, 0), 0.0);
        assert_eq!((rail_offset(2, 1, 0), rail_offset(2, -1, 0)), (-0.5, 0.5));
        let four: Vec<f32> = [(1, 0), (1, 1), (-1, 0), (-1, 1)].iter().map(|&(d, l)| rail_offset(4, d, l)).collect();
        assert_eq!(four, vec![-1.5, -0.5, 0.5, 1.5]);
    }

    #[test]
    fn offset_path_shifts_left_of_travel() {
        let p = [Vec2::ZERO, Vec2::new(10.0, 0.0)];
        assert_eq!(offset_path(&p, 1.0), vec![Vec2::new(0.0, 1.0), Vec2::new(10.0, 1.0)]);
    }

    #[test]
    fn line_geometry_uses_arcs_and_orients_them() {
        let (a, b, c) = (IVec2::new(0, 0), IVec2::new(40, 0), IVec2::new(80, 0));
        let mut g = RailGeoms::default();
        g.0.insert((a, b), vec![Vec2::ZERO, Vec2::new(20.0, 8.0), Vec2::new(40.0, 0.0)]);
        let (p, idx) = line_geometry(&[c, b, a], &g);
        assert_eq!(idx.len(), 3);
        assert_eq!(p[idx[1]], Vec2::new(40.0, 0.0));
        assert_eq!(p[idx[1] + 1], Vec2::new(20.0, 8.0), "arc is walked b -> a");
        assert_eq!(p[idx[2]], Vec2::ZERO);
        assert_eq!(idx[0], 0);
    }

    #[test]
    fn full_trains_are_redder_than_empty_ones() {
        assert!(load_colour(1.0).to_srgba().green < load_colour(0.0).to_srgba().green);
    }
}
