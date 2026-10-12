//! C2 / R21 placement preview: while a line is dragged (or extended), run the placer on the live city and show the
//! real route (arcs), the buildings it would demolish, each crossing (level = orange bar, bridge/water = purple) and a
//! one-line total (cost, razed, crossings). Recomputed only when the drag changes.
use crate::hud::ui_font;
use crate::shell::{hop_key, steer_point, Razed, StationMap};
use crate::zones::{CityPlan, GrowthLink, ZoneLayout};
use bevy::prelude::*;
use city::placer::{place_track, Ctx, Placement, DEFAULT_RADIUS};
use transit::{CrossingKind, TransitNet};
use ui_tools::{segment_transform, LineDrag, Y_PREVIEW};

#[derive(Component)]
struct PlacementMark;
#[derive(Component)]
struct PlacementText;

/// One-line summary shown beside the preview.
pub fn summary(p: &Placement) -> String {
    let (level, bridge) = p.crossings.iter().fold((0, 0), |(l, b), c| if c.kind == CrossingKind::Level { (l + 1, b) } else { (l, b + 1) });
    format!(
        "Track {:.0}  (rails {:.0}, crossings {:.0}, eminent domain {:.0}) | razes {} | {} level, {} bridge | max {:.0} speed",
        p.cost.total, p.cost.track, p.cost.crossings, p.cost.eminent, p.demolition.zones.len(), level, bridge, p.vcap.min(99.0)
    )
}

pub struct PreviewPlugin;

impl Plugin for PreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_text).add_systems(Update, update_preview);
    }
}

fn spawn_text(mut commands: Commands) {
    commands.spawn((
        PlacementText,
        Text::new(""),
        TextFont { font: ui_font(), font_size: FontSize::Px(15.0), ..default() },
        TextColor(Color::WHITE),
        BackgroundColor(Color::srgba(0.1, 0.12, 0.18, 0.85)),
        Node { position_type: PositionType::Absolute, bottom: Val::Px(14.0), left: Val::Px(14.0), padding: UiRect::all(Val::Px(8.0)), ..default() },
        Visibility::Hidden,
    ));
}

#[allow(clippy::too_many_arguments)]
fn update_preview(
    drag: Res<LineDrag>,
    cp: Res<CityPlan>,
    link: Res<GrowthLink>,
    seed: Res<world_gen::WorldSeed>,
    razed: Res<Razed>,
    layout: Res<ZoneLayout>,
    map: Res<StationMap>,
    net: Res<TransitNet>,
    old: Query<Entity, With<PlacementMark>>,
    mut text: Query<(&mut Text, &mut Visibility), With<PlacementText>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut last: Local<Option<LineDrag>>,
) {
    if !drag.is_changed() || last.as_ref() == Some(&*drag) {
        return;
    }
    *last = Some(drag.clone());
    for e in &old {
        commands.entity(e).despawn();
    }
    let hide = |text: &mut Query<(&mut Text, &mut Visibility), With<PlacementText>>| {
        for (_, mut v) in text {
            *v = Visibility::Hidden;
        }
    };
    let Some(from) = drag.from.filter(|f| *f != drag.to) else { return hide(&mut text) };
    // an existing segment is simply reused: nothing to price
    if map.node(from).zip(map.node(drag.to)).and_then(|(a, b)| net.0.rail_edge(a, b)).is_some() {
        return hide(&mut text);
    }
    let _ = hop_key;
    let snap = link.snapshots.latest();
    let gone: Vec<bool> = (0..cp.plan.parcels.len()).map(|i| razed.0.get(i).copied().unwrap_or(false)).collect();
    let land = |p: Vec2| city::is_land(seed.0, p);
    let ctx = Ctx { plan: &cp.plan, built: &snap.built, pop: &snap.pop, gone: &gone, land: &land };
    let Some(p) = place_track(&ctx, from.as_vec2(), drag.to.as_vec2(), steer_point(from, drag.to), DEFAULT_RADIUS) else {
        for (mut t, mut v) in &mut text {
            t.0 = "Too much water to bridge here".into();
            *v = Visibility::Inherited;
        }
        return;
    };
    let mut flat = |c: Color| mats.add(StandardMaterial { base_color: c, unlit: true, alpha_mode: AlphaMode::Blend, ..default() });
    let (route_m, razed_m, level_m, bridge_m) = (
        flat(Color::srgba(0.15, 0.55, 0.95, 0.8)),
        flat(Color::srgba(0.95, 0.2, 0.2, 0.7)),
        flat(Color::srgba(1.0, 0.6, 0.1, 0.95)),
        flat(Color::srgba(0.6, 0.3, 0.9, 0.95)),
    );
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let ring = meshes.add(Cylinder::new(1.0, 1.0));
    for w in p.pts.windows(2) {
        commands.spawn((PlacementMark, Mesh3d(cube.clone()), MeshMaterial3d(route_m.clone()), segment_transform(w[0], w[1], Y_PREVIEW + 0.1, 3.0, 0.3)));
    }
    for &z in &p.demolition.zones {
        if let Some(g) = layout.0.get(z) {
            commands.spawn((
                PlacementMark,
                Mesh3d(ring.clone()),
                MeshMaterial3d(razed_m.clone()),
                Transform::from_xyz(g.center.x, Y_PREVIEW + 0.2, g.center.y).with_scale(Vec3::new(6.0, 0.3, 6.0)),
            ));
        }
    }
    // crossing marks sit on the route at their distance from `from`
    for c in &p.crossings {
        let (pos, dir) = crate::rails::point_at(&p.pts, c.at as f32);
        let m = if c.kind == CrossingKind::Level { level_m.clone() } else { bridge_m.clone() };
        let side = Vec2::new(-dir.y, dir.x);
        commands.spawn((PlacementMark, Mesh3d(cube.clone()), MeshMaterial3d(m), segment_transform(pos - side * 3.0, pos + side * 3.0, Y_PREVIEW + 0.3, 1.0, 0.4)));
    }
    for (mut t, mut v) in &mut text {
        t.0 = summary(&p);
        *v = Visibility::Inherited;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transit::Crossing;

    #[test]
    fn summary_reports_cost_razing_and_crossing_kinds() {
        let mut p = Placement {
            a: Vec2::ZERO, b: Vec2::X, waypoint: Vec2::ZERO, pts: vec![], len: 1.0, radius: 10.0, vcap: 6.0,
            crossings: vec![Crossing { kind: CrossingKind::Level, at: 1.0, road: 1 }, Crossing { kind: CrossingKind::Bridge, at: 2.0, road: u32::MAX }],
            crossing_costs: vec![], demolition: Default::default(), cost: Default::default(), offset: 0.0,
        };
        p.cost.total = 123.0;
        p.demolition.zones = vec![1, 2];
        let s = summary(&p);
        assert!(s.contains("123") && s.contains("razes 2") && s.contains("1 level, 1 bridge"), "{s}");
    }
}
