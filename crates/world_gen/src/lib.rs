//! Generative, infinitely scaling world: chunk addressing, terrain, linked cities.
//! Spec: docs/features/world_gen/SPEC.md. Skeleton only: no gameplay yet.
use bevy::prelude::*;

/// Integer chunk coordinate at a given level of detail (0 = finest).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkCoord { pub x: i64, pub z: i64, pub lod: u8 }

/// Identifier of a city within the shared logical world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CityId(pub u32);

/// Public entry point for this feature.
pub struct WorldGenPlugin;

impl Plugin for WorldGenPlugin {
    fn build(&self, _app: &mut App) {}
}
