//! Orbit / pan / zoom camera. Right-drag orbits, middle-drag or WASD pans, wheel zooms. Left click is left to tools.
use crate::terrain::StreamFocus;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;

#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct OrbitCam {
    pub focus: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
}

pub const MIN_DIST: f32 = 15.0;
pub const MAX_DIST: f32 = 600.0;
const MIN_PITCH: f32 = 0.15;
const MAX_PITCH: f32 = 1.5;

impl OrbitCam {
    pub fn looking_at(focus: Vec3) -> Self {
        Self { focus, yaw: 0.6, pitch: 0.9, dist: 160.0 }
    }

    pub fn eye(&self) -> Vec3 {
        let (sp, cp) = self.pitch.sin_cos();
        let (sy, cy) = self.yaw.sin_cos();
        self.focus + Vec3::new(sy * cp, sp, cy * cp) * self.dist
    }

    pub fn orbit(&mut self, d: Vec2) {
        self.yaw -= d.x * 0.005;
        self.pitch = (self.pitch + d.y * 0.005).clamp(MIN_PITCH, MAX_PITCH);
    }

    /// Pan on the ground plane in camera-yaw space; speed scales with distance.
    pub fn pan(&mut self, right: f32, forward: f32) {
        let (sy, cy) = self.yaw.sin_cos();
        let fwd = Vec3::new(-sy, 0.0, -cy);
        let rgt = Vec3::new(cy, 0.0, -sy);
        self.focus += (rgt * right + fwd * forward) * self.dist * 0.01;
    }

    pub fn zoom(&mut self, lines: f32) {
        self.dist = (self.dist * (1.0 - lines * 0.1)).clamp(MIN_DIST, MAX_DIST);
    }
}

pub struct OrbitCameraPlugin;

impl Plugin for OrbitCameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_camera).add_systems(Update, drive_camera);
    }
}

fn spawn_camera(mut commands: Commands, cam: Option<Res<OrbitCam>>) {
    let c = cam.map(|c| *c).unwrap_or(OrbitCam::looking_at(Vec3::ZERO));
    commands.insert_resource(c);
    commands.spawn((Camera3d::default(), Transform::from_translation(c.eye()).looking_at(c.focus, Vec3::Y)));
    commands.spawn((
        DirectionalLight { illuminance: 9_000.0, ..default() },
        Transform::from_xyz(0.4, 1.0, 0.3).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight { brightness: 350.0, ..default() });
}

fn drive_camera(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut cam: ResMut<OrbitCam>,
    mut focus: ResMut<StreamFocus>,
    mut q: Query<&mut Transform, With<Camera3d>>,
) {
    let axis = |neg: &[KeyCode], pos: &[KeyCode]| keys.any_pressed(pos.iter().copied()) as i32 as f32 - keys.any_pressed(neg.iter().copied()) as i32 as f32;
    let (r, f) = (axis(&[KeyCode::KeyA, KeyCode::ArrowLeft], &[KeyCode::KeyD, KeyCode::ArrowRight]), axis(&[KeyCode::KeyS, KeyCode::ArrowDown], &[KeyCode::KeyW, KeyCode::ArrowUp]));
    cam.pan(r * 0.5, f * 0.5);
    if mouse.pressed(MouseButton::Right) {
        cam.orbit(motion.delta);
    }
    if mouse.pressed(MouseButton::Middle) {
        cam.pan(-motion.delta.x * 0.1, motion.delta.y * 0.1);
    }
    let lines = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
    };
    if lines != 0.0 {
        cam.zoom(lines);
    }
    focus.0 = Vec2::new(cam.focus.x, cam.focus.z);
    for mut t in &mut q {
        *t = Transform::from_translation(cam.eye()).looking_at(cam.focus, Vec3::Y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eye_is_dist_from_focus_and_clamps() {
        let mut c = OrbitCam::looking_at(Vec3::new(10.0, 0.0, 20.0));
        assert!((c.eye().distance(c.focus) - c.dist).abs() < 1e-3);
        for _ in 0..200 {
            c.zoom(1.0);
        }
        assert_eq!(c.dist, MIN_DIST);
        for _ in 0..200 {
            c.zoom(-1.0);
        }
        assert_eq!(c.dist, MAX_DIST);
        c.orbit(Vec2::new(0.0, 1e6));
        assert_eq!(c.pitch, MAX_PITCH);
    }

    #[test]
    fn pan_forward_moves_away_from_eye_on_ground() {
        let mut c = OrbitCam::looking_at(Vec3::ZERO);
        let eye_xz = Vec2::new(c.eye().x, c.eye().z);
        c.pan(0.0, 1.0);
        assert_eq!(c.focus.y, 0.0);
        let f = Vec2::new(c.focus.x, c.focus.z);
        assert!(f.length() > 0.0 && f.distance(eye_xz) > eye_xz.length());
    }
}
