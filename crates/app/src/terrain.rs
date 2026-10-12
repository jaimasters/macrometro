//! Chunk terrain as flat-shaded abstract 3D (D-4): one vertex-coloured material, chunks streamed near the camera.
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use std::collections::HashMap;
use world_gen::{height, ChunkCoord, WorldSeed, CHUNK_SIZE, SEA_LEVEL};

/// World units of height per unit of `world_gen::height` (0..1).
pub const HEIGHT_SCALE: f32 = 24.0;
/// Cells per terrain quad edge (2 => 16x16 quads per 32-cell chunk).
const STEP: u32 = 2;
/// Chunks kept around the camera focus (Chebyshev radius).
pub const VIEW_RADIUS: i64 = 5;
const SPAWNS_PER_FRAME: usize = 6;

/// Ground y at world (x,z); water is clamped flat at sea level.
pub fn ground_y(seed: u64, x: f32, z: f32) -> f32 {
    height(seed, x as f64, z as f64).max(SEA_LEVEL) * HEIGHT_SCALE
}

fn colour(h: f32) -> [f32; 4] {
    if h <= SEA_LEVEL + 0.001 {
        [0.16, 0.34, 0.55, 1.0]
    } else if h < SEA_LEVEL + 0.04 {
        [0.80, 0.75, 0.55, 1.0]
    } else if h < 0.62 {
        [0.36, 0.58, 0.32, 1.0]
    } else if h < 0.78 {
        [0.50, 0.46, 0.38, 1.0]
    } else {
        [0.88, 0.90, 0.92, 1.0]
    }
}

/// Unindexed triangles (3 verts each) with one colour per triangle, so shading is flat. Chunk edges
/// sample the world function, so neighbouring chunks share their border vertices exactly.
pub fn chunk_triangles(seed: u64, c: ChunkCoord) -> (Vec<[f32; 3]>, Vec<[f32; 4]>) {
    let (ox, oz) = c.origin_cell();
    let n = CHUNK_SIZE / STEP;
    let s = STEP as i64;
    let sample = |i: u32, j: u32| {
        let (x, z) = ((ox + i as i64 * s) as f64, (oz + j as i64 * s) as f64);
        let h = height(seed, x, z);
        ([x as f32, h.max(SEA_LEVEL) * HEIGHT_SCALE, z as f32], h)
    };
    let (mut pos, mut col) = (Vec::new(), Vec::new());
    for j in 0..n {
        for i in 0..n {
            let (a, ha) = sample(i, j);
            let (b, hb) = sample(i + 1, j);
            let (cc, hc) = sample(i, j + 1);
            let (d, hd) = sample(i + 1, j + 1);
            for (tri, hs) in [([a, cc, b], [ha, hc, hb]), ([b, cc, d], [hb, hc, hd])] {
                let k = colour((hs[0] + hs[1] + hs[2]) / 3.0);
                pos.extend(tri);
                col.extend([k; 3]);
            }
        }
    }
    (pos, col)
}

fn chunk_mesh(seed: u64, c: ChunkCoord) -> Mesh {
    let (pos, col) = chunk_triangles(seed, c);
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, col)
        .with_computed_flat_normals()
}

/// Chunks within Chebyshev `r` of `center`, nearest first.
pub fn wanted_chunks(center: ChunkCoord, r: i64) -> Vec<ChunkCoord> {
    let mut v: Vec<ChunkCoord> = (-r..=r)
        .flat_map(|dz| (-r..=r).map(move |dx| ChunkCoord::new(center.x + dx, center.z + dz)))
        .collect();
    v.sort_by_key(|c| (c.x - center.x).abs().max((c.z - center.z).abs()));
    v
}

/// World focus point the terrain streams around (the camera sets it).
#[derive(Resource, Default)]
pub struct StreamFocus(pub Vec2);

#[derive(Resource, Default)]
struct Loaded(HashMap<ChunkCoord, Entity>);

#[derive(Resource)]
struct TerrainMaterial(Handle<StandardMaterial>);

pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StreamFocus>()
            .init_resource::<Loaded>()
            .add_systems(Startup, setup_material)
            .add_systems(Update, stream_chunks);
    }
}

fn setup_material(mut commands: Commands, mut mats: ResMut<Assets<StandardMaterial>>) {
    commands.insert_resource(TerrainMaterial(mats.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 1.0,
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
    let center = ChunkCoord::from_cell(focus.0.x.floor() as i64, focus.0.y.floor() as i64);
    let want = wanted_chunks(center, VIEW_RADIUS);
    let keep = |c: &ChunkCoord| (c.x - center.x).abs().max((c.z - center.z).abs()) <= VIEW_RADIUS + 1;
    loaded.0.retain(|c, e| {
        let k = keep(c);
        if !k {
            commands.entity(*e).despawn();
        }
        k
    });
    let missing: Vec<ChunkCoord> = want.into_iter().filter(|c| !loaded.0.contains_key(c)).take(SPAWNS_PER_FRAME).collect();
    for c in missing {
        let e = commands.spawn((Mesh3d(meshes.add(chunk_mesh(seed.0, c))), MeshMaterial3d(mat.0.clone()))).id();
        loaded.0.insert(c, e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangles_count_and_range() {
        let (p, c) = chunk_triangles(1, ChunkCoord::new(0, 0));
        let q = (CHUNK_SIZE / STEP) as usize;
        assert_eq!(p.len(), q * q * 6);
        assert_eq!(c.len(), p.len());
        assert!(p.iter().all(|v| v[1] >= SEA_LEVEL * HEIGHT_SCALE - 1e-4 && v[1] <= HEIGHT_SCALE));
    }

    #[test]
    fn neighbour_borders_agree() {
        let w = |c: ChunkCoord| chunk_triangles(4, c).0;
        let (a, b) = (w(ChunkCoord::new(0, 0)), w(ChunkCoord::new(1, 0)));
        let (edge_x, ymax) = (CHUNK_SIZE as f32, |v: &Vec<[f32; 3]>, x: f32, z: f32| {
            v.iter().find(|p| p[0] == x && p[2] == z).map(|p| p[1])
        });
        for z in (0..CHUNK_SIZE).step_by(STEP as usize) {
            assert_eq!(ymax(&a, edge_x, z as f32), ymax(&b, edge_x, z as f32));
        }
    }

    #[test]
    fn wanted_is_square_nearest_first() {
        let w = wanted_chunks(ChunkCoord::new(2, -3), 2);
        assert_eq!(w.len(), 25);
        assert_eq!(w[0], ChunkCoord::new(2, -3));
        assert!(w.iter().all(|c| (c.x - 2).abs() <= 2 && (c.z + 3).abs() <= 2));
    }

    #[test]
    fn ground_matches_mesh_vertices() {
        let (p, _) = chunk_triangles(9, ChunkCoord::new(-1, 2));
        assert!(p.iter().all(|v| (ground_y(9, v[0], v[2]) - v[1]).abs() < 1e-3));
    }
}
