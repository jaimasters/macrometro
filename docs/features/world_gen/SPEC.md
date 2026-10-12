# world_gen SPEC
Scope: Chunk grid, seeded terrain, city regions. Arch: multi-city, growth-drivers. Crate: crates/world_gen (pub API = lib.rs).
Design: pure-Rust core (rng, terrain, chunk, city; no Bevy types) + thin `WorldGenPlugin` (inserts `WorldSeed`, `Cities`).
- Terrain `height(seed,x,z)->f32 in [0,1)`: 4-octave value-noise fBm over world cell coords; `SEA_LEVEL`=0.35. Chunks (32x32 cells, `ChunkCoord{x,z,lod}`) are views of it, so neighbours agree.
- Cities: one candidate per 24-chunk super-cell, spiral order from origin; square region radius 2..=4 chunks; always disjoint and on land. `generate_cities(seed,n)` prefix-stable.
- Out of scope now: sprawl placement (needs growth data), non-flat LOD pyramid, inter-city links (transit).
## Acceptance (`cargo test -p world_gen`)
rng::rng_golden_values; terrain::{terrain_deterministic, terrain_seed_sensitive, terrain_in_range_and_continuous, terrain_has_land_and_water}; chunk::{chunk_hash_deterministic_and_seeded, chunk_hash_golden, chunk_edges_match_neighbour, from_cell_negative}; city::{cities_deterministic, cities_prefix_stable, cities_disjoint_and_on_land, region_contains_matches_chunks}; tests::plugin_inserts_seeded_cities.
