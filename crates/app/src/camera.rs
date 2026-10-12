//! Straight-down orthographic map camera: drag to pan, scroll to zoom (ortho scale). No tilt, no orbit.
use crate::terrain::StreamFocus;
use bevy::camera::ScalingMode;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use ui_tools::ActiveTool;

/// Camera state: `focus` is the world (x,z) at screen centre, `view_h` the world height shown on screen.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct MapCam {
    pub focus: Vec2,
    pub view_h: f32,
}

pub const MIN_VIEW: f32 = 40.0;
pub const MAX_VIEW: f32 = 420.0;
const HEIGHT: f32 = 500.0;

impl MapCam {
    /// Frame a square region of `extent` world units around `focus` with 25% margin.
    pub fn framing(focus: Vec2, extent: f32) -> Self {
        Self { focus, view_h: (extent * 1.25).clamp(MIN_VIEW, MAX_VIEW) }
    }

    /// Move the focus so the map follows a cursor drag of `d` pixels (screen y grows downward = world +z).
    pub fn drag(&mut self, d: Vec2, window_h: f32) {
        self.focus -= d * (self.view_h / window_h.max(1.0));
    }

    /// Zoom about the screen centre; positive lines zoom in.
    pub fn zoom(&mut self, lines: f32) {
        self.view_h = (self.view_h * (1.0 - lines * 0.1)).clamp(MIN_VIEW, MAX_VIEW);
    }

    /// Terrain chunks needed to cover the view (assumes up to ~2:1 windows).
    pub fn chunk_radius(&self) -> i64 {
        (self.view_h / 32.0).ceil() as i64 + 1
    }
}

pub struct MapCameraPlugin;

impl Plugin for MapCameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_camera).add_systems(Update, drive_camera);
    }
}

fn spawn_camera(mut commands: Commands, cam: Option<Res<MapCam>>) {
    let c = cam.map(|c| *c).unwrap_or(MapCam { focus: Vec2::ZERO, view_h: 200.0 });
    commands.insert_resource(c);
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None, // flat map colours must render as authored
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical { viewport_height: c.view_h },
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(c.focus.x, HEIGHT, c.focus.y).looking_at(Vec3::new(c.focus.x, 0.0, c.focus.y), Vec3::NEG_Z),
    ));
}

#[allow(clippy::too_many_arguments)]
fn drive_camera(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    tool: Res<ActiveTool>,
    ui: Query<&Hovered>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    mut cam: ResMut<MapCam>,
    mut focus: ResMut<StreamFocus>,
    mut q: Query<(&mut Transform, &mut Projection), With<Camera3d>>,
) {
    let over_ui = ui.iter().any(|h| h.get());
    let h = window.map_or(720.0, |w| w.height());
    let axis = |neg: &[KeyCode], pos: &[KeyCode]| {
        keys.any_pressed(pos.iter().copied()) as i32 as f32 - keys.any_pressed(neg.iter().copied()) as i32 as f32
    };
    let (r, f) = (
        axis(&[KeyCode::KeyA, KeyCode::ArrowLeft], &[KeyCode::KeyD, KeyCode::ArrowRight]),
        axis(&[KeyCode::KeyS, KeyCode::ArrowDown], &[KeyCode::KeyW, KeyCode::ArrowUp]),
    );
    let step = cam.view_h * 0.01;
    cam.focus += Vec2::new(r, -f) * step;
    let dragging = mouse.pressed(MouseButton::Right)
        || mouse.pressed(MouseButton::Middle)
        || (mouse.pressed(MouseButton::Left) && *tool == ActiveTool::Pan && !over_ui);
    if dragging {
        cam.drag(motion.delta, h);
    }
    let lines = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
    };
    if lines != 0.0 && !over_ui {
        cam.zoom(lines);
    }
    focus.pos = cam.focus;
    focus.radius = cam.chunk_radius();
    for (mut t, mut p) in &mut q {
        *t = Transform::from_xyz(cam.focus.x, HEIGHT, cam.focus.y)
            .looking_at(Vec3::new(cam.focus.x, 0.0, cam.focus.y), Vec3::NEG_Z);
        if let Projection::Orthographic(o) = &mut *p {
            o.scaling_mode = ScalingMode::FixedVertical { viewport_height: cam.view_h };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_clamps_and_framing_fits() {
        let mut c = MapCam::framing(Vec2::new(10.0, 20.0), 160.0);
        assert_eq!(c.view_h, 200.0);
        for _ in 0..200 {
            c.zoom(1.0);
        }
        assert_eq!(c.view_h, MIN_VIEW);
        for _ in 0..200 {
            c.zoom(-1.0);
        }
        assert_eq!(c.view_h, MAX_VIEW);
    }

    #[test]
    fn drag_moves_map_with_cursor() {
        let mut c = MapCam { focus: Vec2::ZERO, view_h: 360.0 };
        c.drag(Vec2::new(10.0, -20.0), 720.0); // cursor right/up => map right/up => focus left/down(+z)
        assert_eq!(c.focus, Vec2::new(-5.0, 10.0));
    }
}
