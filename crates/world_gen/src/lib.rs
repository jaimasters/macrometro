//! Generative, infinitely scaling world: chunk addressing, terrain, linked cities.
//! Spec: docs/features/world_gen/SPEC.md. Pure-Rust core (rng/terrain/chunk/city) + thin Bevy plugin.
use bevy::prelude::*;

mod chunk;
mod city;
pub mod rng;
mod terrain;

pub use chunk::{chunk_hash, chunk_heights, ChunkCoord, CHUNK_SIZE};
pub use city::{generate_cities, CityId, CityRegion};
pub use terrain::{height, SEA_LEVEL};

/// World seed; everything generated derives from it. Insert before adding the plugin to override.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorldSeed(pub u64);

/// Cities of the current world, generated once from `WorldSeed`.
#[derive(Resource, Debug, Clone, Default)]
pub struct Cities(pub Vec<CityRegion>);

/// Cities generated at startup (more can be requested via `generate_cities`).
const STARTUP_CITIES: usize = 3;

/// Public entry point for this feature.
pub struct WorldGenPlugin;

impl Plugin for WorldGenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldSeed>();
        let seed = app.world().resource::<WorldSeed>().0;
        app.insert_resource(Cities(generate_cities(seed, STARTUP_CITIES)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_inserts_seeded_cities() {
        let mut app = App::new();
        app.insert_resource(WorldSeed(5)).add_plugins(WorldGenPlugin);
        assert_eq!(app.world().resource::<Cities>().0, generate_cities(5, STARTUP_CITIES));
    }
}
