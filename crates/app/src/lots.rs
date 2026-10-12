//! R40: every driveway ends at a building or at a dotted outline of the lot it serves. The city plan is fixed at
//! generation, so every lot is already "planned ahead": a lot whose frontage road is built but which has no
//! building yet (never developed, or razed by eminent domain) shows as a dotted outline in its zone colour.
use crate::zones::{is_built, GrowthLink, ZoneLayout, Y_ZONE};
use crate::zones::CityPlan;
use bevy::prelude::*;
use city::ZoneKind;

#[derive(Component)]
struct LotOutline(usize);

/// Dot positions (local, unit lot) around the perimeter: three per side.
pub fn dot_offsets() -> Vec<Vec2> {
    let s = [-0.5f32, 0.0, 0.5];
    let mut v = vec![];
    for a in s {
        v.extend([Vec2::new(a, -0.5), Vec2::new(a, 0.5)]);
        if a != -0.5 && a != 0.5 {
            v.extend([Vec2::new(-0.5, a), Vec2::new(0.5, a)]);
        }
    }
    v.extend([Vec2::new(-0.5, -0.5), Vec2::new(-0.5, 0.5), Vec2::new(0.5, -0.5), Vec2::new(0.5, 0.5)]);
    v.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    v.dedup();
    v
}

/// A lot shows its outline when its frontage road exists and nothing stands on it.
pub fn shows_outline(road_built: bool, pop: f32) -> bool {
    road_built && !is_built(pop)
}

pub struct LotsPlugin;

impl Plugin for LotsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_lots).add_systems(Update, update_lots);
    }
}

fn spawn_lots(layout: Res<ZoneLayout>, mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    let dot = meshes.add(Cuboid::new(1.0, 0.1, 1.0));
    let offs = dot_offsets();
    let kinds = [ZoneKind::Residential, ZoneKind::Commercial, ZoneKind::Industrial].map(|k| (k, mats.add(StandardMaterial { base_color: k.colour(), unlit: true, ..default() })));
    for (i, g) in layout.0.iter().enumerate() {
        let m = kinds.iter().find(|(k, _)| *k == g.kind).unwrap().1.clone();
        commands
            .spawn((
                LotOutline(i),
                Transform { translation: Vec3::new(g.center.x, Y_ZONE + 0.05, g.center.y), rotation: Quat::from_rotation_y(-g.angle), scale: Vec3::new(g.size.x, 1.0, g.size.y) },
                Visibility::Hidden,
            ))
            .with_children(|p| {
                for o in &offs {
                    // dots stay roughly square whatever the lot's size
                    p.spawn((Mesh3d(dot.clone()), MeshMaterial3d(m.clone()), Transform::from_xyz(o.x, 0.0, o.y).with_scale(Vec3::new(0.5 / g.size.x.max(1.0), 1.0, 0.5 / g.size.y.max(1.0)))));
                }
            });
    }
}

fn update_lots(link: Res<GrowthLink>, cp: Res<CityPlan>, mut last: Local<Option<u64>>, mut q: Query<(&LotOutline, &mut Visibility)>) {
    let s = link.snapshots.latest();
    if *last == Some(s.tick) {
        return;
    }
    *last = Some(s.tick);
    for (l, mut v) in &mut q {
        let road = s.built.get(cp.plan.parcels[l.0].access).copied().unwrap_or(false);
        let want = if shows_outline(road, s.pop.get(l.0).copied().unwrap_or(0.0)) { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undeveloped_lots_on_built_roads_show_an_outline_and_developed_ones_do_not() {
        assert!(shows_outline(true, 0.0), "driveway to an empty lot gets a dotted outline");
        assert!(!shows_outline(true, 500.0), "a building stands there");
        assert!(!shows_outline(false, 0.0), "no road, no driveway, no outline");
        assert_eq!(dot_offsets().len(), 8, "corners plus side midpoints");
    }
}
