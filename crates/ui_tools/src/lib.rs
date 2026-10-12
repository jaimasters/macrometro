//! Player tools: line drawing, station placement, macro line policies, micro edits.
//! Spec: docs/features/ui_tools/SPEC.md. Pure logic in `core`; this file is the bevy plugin.
use bevy::picking::hover::Hovered;
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

/// World (x,z) under the cursor this frame, if over the map and not over UI. Written by `pointer`.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct CursorWorld(pub Option<Vec2>);

/// Marker on spawned road-plan ghost meshes.
#[derive(Component)]
pub struct RoadGhost(pub [u32; 4]);

/// Marker on the live line-drag preview meshes.
#[derive(Component)]
pub struct DragPreview;

const GHOST_WIDTH: f32 = 1.6;
const GHOST_THICKNESS: f32 = 0.3;
/// Heights (world y) for flat layering, bottom to top. Shared with the app so layers never z-fight.
pub const Y_GHOST: f32 = 0.5;
pub const Y_PREVIEW: f32 = 2.6;

pub struct UiToolsPlugin;

impl Plugin for UiToolsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActiveTool>()
            .init_resource::<ActiveAgency>()
            .init_resource::<LineDrag>()
            .init_resource::<Stations>()
            .init_resource::<CursorWorld>()
            .init_resource::<GroundHeight>()
            .add_message::<ToolCommand>()
            .add_message::<GhostRoads>()
            .add_systems(Update, (select_tool, pointer, draw_preview, spawn_road_ghosts).chain());
    }
}

fn select_tool(keys: Res<ButtonInput<KeyCode>>, mut tool: ResMut<ActiveTool>, mut drag: ResMut<LineDrag>) {
    let next = if keys.just_pressed(KeyCode::Digit1) {
        ActiveTool::Pan
    } else if keys.just_pressed(KeyCode::Digit2) {
        ActiveTool::PlaceStation
    } else if keys.just_pressed(KeyCode::Digit3) {
        ActiveTool::DrawLine
    } else if keys.just_pressed(KeyCode::Digit4) {
        ActiveTool::Delete
    } else {
        return;
    };
    *tool = next;
    drag.from = None;
}

/// Left button: PlaceStation clicks, DrawLine press-drag-release. Pan is handled by the camera.
#[allow(clippy::too_many_arguments)]
fn pointer(
    mouse: Res<ButtonInput<MouseButton>>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    camera: Option<Single<(&Camera, &GlobalTransform)>>,
    mut tool: ResMut<ActiveTool>,
    agency: Res<ActiveAgency>,
    ground: Res<GroundHeight>,
    ui: Query<&Hovered>,
    mut cursor_world: ResMut<CursorWorld>,
    mut stations: ResMut<Stations>,
    mut drag: ResMut<LineDrag>,
    mut out: MessageWriter<ToolCommand>,
) {
    let over_ui = ui.iter().any(|h| h.get());
    cursor_world.0 = None;
    if let (Some(window), Some(camera)) = (window, camera) {
        let (camera, cam_tf) = *camera;
        cursor_world.0 = window
            .cursor_position()
            .and_then(|c| camera.viewport_to_world(cam_tf, c).ok())
            .and_then(|ray| ray_terrain(ray.origin, *ray.direction, &*ground.0))
            .filter(|_| !over_ui);
    }
    let hit = cursor_world.0;
    // Right-click a station: delete it, whatever tool is active.
    if mouse.just_pressed(MouseButton::Right) {
        if let Some(c) = hit.and_then(|p| delete_at(&mut stations, agency.0, p)) {
            out.write(c);
        }
    }
    if mouse.just_pressed(MouseButton::Left) {
        if let Some(p) = hit {
            match *tool {
                ActiveTool::Pan => {}
                ActiveTool::Delete => {
                    if let Some(c) = delete_at(&mut stations, agency.0, p) {
                        out.write(c);
                        *tool = ActiveTool::Pan; // one-shot
                    }
                }
                ActiveTool::PlaceStation => {
                    let cell = snap_station(&stations.0, p);
                    if !stations.0.contains(&cell) {
                        stations.0.push(cell);
                        out.write(ToolCommand { agency: agency.0, action: ToolAction::PlaceStation { cell } });
                        *tool = ActiveTool::Pan; // one-shot
                    }
                }
                ActiveTool::DrawLine => {
                    let from = snap_station(&stations.0, p);
                    *drag = LineDrag { from: Some(from), to: from };
                }
            }
        }
    }
    if let (Some(_), Some(p)) = (drag.from, hit) {
        let to = snap_station(&stations.0, p);
        if drag.to != to {
            drag.to = to;
        }
    }
    if mouse.just_released(MouseButton::Left) {
        if let Some(from) = drag.from.take() {
            if let Some(c) = finish_drag(&mut stations, agency.0, from, drag.to) {
                out.write(c);
                *tool = ActiveTool::Pan; // one-shot
            }
        }
    }
}

/// Live preview of the line being dragged: translucent octilinear strokes.
fn draw_preview(
    drag: Res<LineDrag>,
    old: Query<Entity, With<DragPreview>>,
    mut mat: Local<Option<Handle<StandardMaterial>>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !drag.is_changed() {
        return;
    }
    for e in &old {
        commands.entity(e).despawn();
    }
    let Some(from) = drag.from.filter(|f| *f != drag.to) else { return };
    let mat = mat
        .get_or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: Color::srgba(0.15, 0.45, 0.90, 0.55),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                ..default()
            })
        })
        .clone();
    let mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    for w in route(&[from, drag.to]).windows(2) {
        commands.spawn((
            DragPreview,
            Mesh3d(mesh.clone()),
            MeshMaterial3d(mat.clone()),
            segment_transform(w[0], w[1], Y_PREVIEW, 3.0, 0.3),
        ));
    }
}

/// Diff the ghost set against the latest GhostRoads: keep unchanged segments, spawn only new ones,
/// despawn only those no longer planned. Respawning the whole set on every change made planned
/// roads flash in and out (R22).
fn spawn_road_ghosts(
    mut plans: MessageReader<GhostRoads>,
    old: Query<(Entity, &RoadGhost)>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut handles: Local<Option<(Handle<Mesh>, Handle<StandardMaterial>)>>,
) {
    let Some(plan) = plans.read().last() else { return };
    let key = |a: Vec2, b: Vec2| [a.x.to_bits(), a.y.to_bits(), b.x.to_bits(), b.y.to_bits()];
    let wanted: std::collections::HashSet<_> = plan.segments.iter().map(|&(a, b)| key(a, b)).collect();
    let mut have = std::collections::HashSet::new();
    for (e, g) in &old {
        if wanted.contains(&g.0) {
            have.insert(g.0);
        } else {
            commands.entity(e).despawn();
        }
    }
    let (mesh, mat) = handles
        .get_or_insert_with(|| {
            let mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
            let mat = materials.add(StandardMaterial {
                base_color: Color::srgba(0.25, 0.25, 0.30, 0.10),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                ..default()
            });
            (mesh, mat)
        })
        .clone();
    for &(a, b) in &plan.segments {
        if have.insert(key(a, b)) {
            commands.spawn((
                RoadGhost(key(a, b)),
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat.clone()),
                segment_transform(a, b, Y_GHOST, GHOST_WIDTH, GHOST_THICKNESS),
            ));
        }
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
    fn number_keys_select_tool() {
        let mut app = app();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Digit3);
        app.update();
        assert_eq!(*app.world().resource::<ActiveTool>(), ActiveTool::DrawLine);
    }

    #[test]
    fn delete_tool_and_one_shot_revert_logic() {
        let mut st = Stations(vec![IVec2::new(0, 0)]);
        // finish_drag/delete_at are the units the one-shot hooks hang off; a no-op drag emits nothing.
        assert!(finish_drag(&mut st, AgencyId(1), IVec2::ZERO, IVec2::ZERO).is_none());
        assert!(delete_at(&mut st, AgencyId(1), Vec2::ZERO).is_some());
        assert!(st.0.is_empty());
    }

    #[test]
    fn drag_preview_spawns_and_clears() {
        let mut app = app();
        *app.world_mut().resource_mut::<LineDrag>() = LineDrag { from: Some(IVec2::ZERO), to: IVec2::new(40, 12) };
        app.update();
        let count = |app: &mut App| app.world_mut().query::<&DragPreview>().iter(app.world()).count();
        assert_eq!(count(&mut app), 2); // one bend -> two strokes
        app.world_mut().resource_mut::<LineDrag>().from = None;
        app.update();
        assert_eq!(count(&mut app), 0);
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
