// Placeholder starter scene; replaced once MVP scope is agreed (docs/OPEN_QUESTIONS.md).
use bevy::prelude::*;

#[derive(Component)]
struct Player;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, app::MacrometroPlugins))
        .add_systems(Startup, setup)
        .add_systems(Update, move_player)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Circle::new(6.0))),
        MeshMaterial3d(materials.add(Color::WHITE)),
        Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
    ));
    commands.spawn((
        Player,
        Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
        MeshMaterial3d(materials.add(Color::srgb_u8(124, 144, 255))),
        Transform::from_xyz(0.0, 0.5, 0.0),
    ));
    commands.spawn((PointLight { shadow_maps_enabled: true, ..default() }, Transform::from_xyz(4.0, 8.0, 4.0)));
    commands.spawn((Camera3d::default(), Transform::from_xyz(-2.5, 6.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y)));
}

fn move_player(keys: Res<ButtonInput<KeyCode>>, time: Res<Time>, mut q: Query<&mut Transform, With<Player>>) {
    let mut dir = Vec3::ZERO;
    if keys.any_pressed([KeyCode::KeyW, KeyCode::ArrowUp]) { dir.z -= 1.0; }
    if keys.any_pressed([KeyCode::KeyS, KeyCode::ArrowDown]) { dir.z += 1.0; }
    if keys.any_pressed([KeyCode::KeyA, KeyCode::ArrowLeft]) { dir.x -= 1.0; }
    if keys.any_pressed([KeyCode::KeyD, KeyCode::ArrowRight]) { dir.x += 1.0; }
    for mut t in &mut q {
        t.translation += dir.normalize_or_zero() * 4.0 * time.delta_secs();
    }
}
