//! Player tools: line drawing, station placement, macro line policies, micro edits.
//! Spec: docs/features/ui_tools/SPEC.md. Pure logic in `core`; this file is the bevy plugin.
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

mod core;
pub use core::*;

/// Terrain height lookup (world x,z -> y) used for picking and ghost placement. App overrides it.
#[derive(Resource, Clone)]
pub struct GroundHeight(pub std::sync::Arc<dyn Fn(f32, f32) -> f32 + Send + Sync>);

impl Default for GroundHeight {
    fn default() -> Self {
        Self(std::sync::Arc::new(|_, _| 0.0))
    }
}

/// Agency the local player acts as (D-8). Single-player: AgencyId(0).
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct ActiveAgency(pub AgencyId);

/// Marker on spawned road-plan ghost meshes.
#[derive(Component)]
pub struct RoadGhost;

/// Ghost road width / thickness in world units.
const GHOST_WIDTH: f32 = 3.0;
const GHOST_THICKNESS: f32 = 0.3;

pub struct UiToolsPlugin;

impl Plugin for UiToolsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActiveTool>()
            .init_resource::<ActiveAgency>()
            .init_resource::<LineDraft>()
            .init_resource::<GroundHeight>()
            .add_message::<ToolCommand>()
            .add_message::<GhostRoads>()
            .add_systems(Update, (select_tool, finish_line_key, click_tool, spawn_road_ghosts).chain());
    }
}

fn select_tool(keys: Res<ButtonInput<KeyCode>>, mut tool: ResMut<ActiveTool>, mut draft: ResMut<LineDraft>) {
    let next = if keys.just_pressed(KeyCode::Digit1) {
        ActiveTool::PlaceStation
    } else if keys.just_pressed(KeyCode::Digit2) {
        ActiveTool::DrawLine
    } else if keys.just_pressed(KeyCode::Escape) {
        ActiveTool::None
    } else {
        return;
    };
    *tool = next;
    draft.0.clear();
}

fn finish_line_key(
    keys: Res<ButtonInput<KeyCode>>,
    agency: Res<ActiveAgency>,
    mut draft: ResMut<LineDraft>,
    mut out: MessageWriter<ToolCommand>,
) {
    if keys.just_pressed(KeyCode::Enter) {
        if let Some(c) = finish_line(agency.0, &mut draft) {
            out.write(c);
        }
    }
}

fn click_tool(
    mouse: Res<ButtonInput<MouseButton>>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    camera: Option<Single<(&Camera, &GlobalTransform)>>,
    tool: Res<ActiveTool>,
    agency: Res<ActiveAgency>,
    ground: Res<GroundHeight>,
    mut draft: ResMut<LineDraft>,
    mut out: MessageWriter<ToolCommand>,
) {
    if !mouse.just_pressed(MouseButton::Left) || *tool == ActiveTool::None {
        return;
    }
    let (Some(window), Some(camera)) = (window, camera) else { return };
    let (camera, cam_tf) = *camera;
    let Some(cursor) = window.cursor_position() else { return };
    let Ok(ray) = camera.viewport_to_world(cam_tf, cursor) else { return };
    let Some(hit) = ray_terrain(ray.origin, *ray.direction, &*ground.0) else { return };
    if let Some(c) = click_action(*tool, agency.0, &mut draft, snap(hit)) {
        out.write(c);
    }
}

/// Replace the ghost set with the latest GhostRoads. Faint, unlit, translucent (D-7: no popups).
fn spawn_road_ghosts(
    mut plans: MessageReader<GhostRoads>,
    old: Query<Entity, With<RoadGhost>>,
    ground: Res<GroundHeight>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(plan) = plans.read().last() else { return };
    for e in &old {
        commands.entity(e).despawn();
    }
    let mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let mat = materials.add(StandardMaterial {
        base_color: Color::srgba(0.9, 0.9, 0.7, 0.22),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });
    for &(a, b) in &plan.segments {
        let m = (a + b) * 0.5;
        let y = (ground.0)(m.x, m.y) + 0.4;
        commands.spawn((
            RoadGhost,
            Mesh3d(mesh.clone()),
            MeshMaterial3d(mat.clone()),
            segment_transform(a, b, y, GHOST_WIDTH, GHOST_THICKNESS),
        ));
    }
}

#[cfg(test)]
mod plugin_tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_plugins(UiToolsPlugin);
        app
    }

    #[test]
    fn keys_select_tool_and_enter_emits_line() {
        let mut app = app();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Digit2);
        app.update();
        assert_eq!(*app.world().resource::<ActiveTool>(), ActiveTool::DrawLine);
        app.world_mut().resource_mut::<LineDraft>().0 = vec![IVec2::ZERO, IVec2::new(5, 5)];
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Enter);
        app.update();
        let msgs = app.world().resource::<Messages<ToolCommand>>();
        let got: Vec<_> = msgs.iter_current_update_messages().collect();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].agency, AgencyId(0));
    }

    #[test]
    fn road_plan_spawns_then_replaces_ghosts() {
        let mut app = app();
        let seg = |n: usize| GhostRoads { segments: (0..n).map(|i| (Vec2::ZERO, Vec2::new(10.0 + i as f32, 0.0))).collect() };
        app.world_mut().write_message(seg(3));
        app.update();
        let count = |app: &mut App| app.world_mut().query::<&RoadGhost>().iter(app.world()).count();
        assert_eq!(count(&mut app), 3);
        app.world_mut().write_message(seg(1));
        app.update();
        assert_eq!(count(&mut app), 1);
    }
}
