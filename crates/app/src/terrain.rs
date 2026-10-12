//! Chunk terrain as a flat 2D map (D-4): muted land and clean flat water, one unlit vertex-coloured material.
//! Chunks stream around the camera focus; everything sits at y = 0 (the camera looks straight down).
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use std::collections::HashMap;
use world_gen::{height, ChunkCoord, WorldSeed, CHUNK_SIZE, SEA_LEVEL};

/// Cells per terrain quad edge (2 => 16x16 quads per 32-cell chunk; shorelines resolve to 2 cells).
const STEP: u32 = 2;
/// Largest streaming radius in chunks (caps mesh count at extreme zoom-out).
pub const MAX_RADIUS: i64 = 12;
const SPAWNS_PER_FRAME: usize = 12;

pub const LAND: Color = Color::srgb(0.89, 0.87, 0.81);
pub const WATER: Color = Color::srgb(0.64, 0.77, 0.86);

/// The map is flat: ground is always y = 0.
pub fn ground_y(_seed: u64, _x: f32, _z: f32) -> f32 {
    0.0
}

fn linear(c: Color) -> [f32; 4] {
    c.to_linear().to_f32_array()
}

/// Unindexed flat triangles at y = 0 with one colour per triangle: land or water by mean terrain height.
pub fn chunk_triangles(seed: u64, c: ChunkCoord) -> (Vec<[f32; 3]>, Vec<[f32; 4]>) {
    let (ox, oz) = c.origin_cell();
    let n = CHUNK_SIZE / STEP;
    let s = STEP as i64;
    let sample = |i: u32, j: u32| {
        let (x, z) = ((ox + i as i64 * s) as f64, (oz + j as i64 * s) as f64);
        ([x as f32, 0.0, z as f32], height(seed, x, z))
    };
    let (land, water) = (linear(LAND), linear(WATER));
    let (mut pos, mut col) = (Vec::new(), Vec::new());
    for j in 0..n {
        for i in 0..n {
            let (a, ha) = sample(i, j);
            let (b, hb) = sample(i + 1, j);
            let (cc, hc) = sample(i, j + 1);
            let (d, hd) = sample(i + 1, j + 1);
            for (tri, hs) in [([a, cc, b], [ha, hc, hb]), ([b, cc, d], [hb, hc, hd])] {
                let k = if (hs[0] + hs[1] + hs[2]) / 3.0 >= SEA_LEVEL { land } else { water };
                pos.extend(tri);
                col.extend([k; 3]);
            }
        }
    }
    (pos, col)
}

fn chunk_mesh(seed: u64, c: ChunkCoord) -> Mesh {
    let (pos, col) = chunk_triangles(seed, c);
    let normals = vec![[0.0, 1.0, 0.0]; pos.len()];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, col)
}

/// Chunks within Chebyshev `r` of `center`, nearest first.
pub fn wanted_chunks(center: ChunkCoord, r: i64) -> Vec<ChunkCoord> {
    let mut v: Vec<ChunkCoord> = (-r..=r)
        .flat_map(|dz| (-r..=r).map(move |dx| ChunkCoord::new(center.x + dx, center.z + dz)))
        .collect();
    v.sort_by_key(|c| (c.x - center.x).abs().max((c.z - center.z).abs()));
    v
}

/// World focus point and chunk radius the terrain streams around (the camera sets both).
#[derive(Resource)]
pub struct StreamFocus {
    pub pos: Vec2,
    pub radius: i64,
}

impl Default for StreamFocus {
    fn default() -> Self {
        Self { pos: Vec2::ZERO, radius: 6 }
    }
}

#[derive(Resource, Default)]
struct Loaded(HashMap<ChunkCoord, Entity>);

#[derive(Resource)]
struct TerrainMaterial(Handle<StandardMaterial>);

pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StreamFocus>()
            .init_resource::<Loaded>()
            .insert_resource(ClearColor(WATER))
            .add_systems(Startup, setup_material)
            .add_systems(Update, stream_chunks);
    }
}

fn setup_material(mut commands: Commands, mut mats: ResMut<Assets<StandardMaterial>>) {
    commands.insert_resource(TerrainMaterial(mats.add(StandardMaterial {
        base_color: Color::WHITE,
        unlit: true,
        ..default()
    })));
}

fn stream_chunks(
    focus: Res<StreamFocus>,
    seed: Res<WorldSeed>,
    mat: Option<Res<TerrainMaterial>>,
    mut loaded: ResMut<Loaded>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
) {
    let Some(mat) = mat else { return };
    let r = focus.radius.clamp(1, MAX_RADIUS);
    let center = ChunkCoord::from_cell(focus.pos.x.floor() as i64, focus.pos.y.floor() as i64);
    let keep = |c: &ChunkCoord| (c.x - center.x).abs().max((c.z - center.z).abs()) <= r + 2;
    loaded.0.retain(|c, e| {
        let k = keep(c);
        if !k {
            commands.entity(*e).despawn();
        }
        k
    });
    let missing: Vec<ChunkCoord> =
        wanted_chunks(center, r).into_iter().filter(|c| !loaded.0.contains_key(c)).take(SPAWNS_PER_FRAME).collect();
    for c in missing {
        let e = commands.spawn((Mesh3d(meshes.add(chunk_mesh(seed.0, c))), MeshMaterial3d(mat.0.clone()))).id();
        loaded.0.insert(c, e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangles_flat_and_two_colours_only() {
        let (p, c) = chunk_triangles(1, ChunkCoord::new(0, 0));
        let q = (CHUNK_SIZE / STEP) as usize;
        assert_eq!(p.len(), q * q * 6);
        assert_eq!(c.len(), p.len());
        assert!(p.iter().all(|v| v[1] == 0.0));
        let mut distinct: Vec<[u32; 4]> = c.iter().map(|k| k.map(f32::to_bits)).collect();
        distinct.sort();
        distinct.dedup();
        assert!(distinct.len() <= 2, "no noise colouring: {}", distinct.len());
    }

    #[test]
    fn neighbour_borders_agree() {
        let w = |c: ChunkCoord| chunk_triangles(4, c).0;
        let (a, b) = (w(ChunkCoord::new(0, 0)), w(ChunkCoord::new(1, 0)));
        let edge_x = CHUNK_SIZE as f32;
        let has = |v: &Vec<[f32; 3]>, x: f32, z: f32| v.iter().any(|p| p[0] == x && p[2] == z);
        for z in (0..CHUNK_SIZE).step_by(STEP as usize) {
            assert!(has(&a, edge_x, z as f32) && has(&b, edge_x, z as f32));
        }
    }

    #[test]
    fn wanted_is_square_nearest_first() {
        let w = wanted_chunks(ChunkCoord::new(2, -3), 2);
        assert_eq!(w.len(), 25);
        assert_eq!(w[0], ChunkCoord::new(2, -3));
        assert!(w.iter().all(|c| (c.x - 2).abs() <= 2 && (c.z + 3).abs() <= 2));
    }
}
