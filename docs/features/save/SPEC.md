# save SPEC
Crate: crates/save. Depends on world_gen (city regions, rng).
- Format: RON (serde), `SaveFile{header{magic:"MMSAVE",version:u32,seed,tick}, world{seed,chunk_overrides}, cities[]{id,region,zones,network,lines,metrics}}`. Canonical text via `to_ron` (fixed pretty config, trailing newline).
- zones/network/lines/metrics are STUB schemas (format.rs) until growth/transit/economy publish types.
- Generated content NOT stored: regenerates from seed; only deltas saved.
- Loader `from_ron`: probe header -> magic check -> reject version > CURRENT with `FutureVersion` -> apply `MIGRATIONS[v-1]` (ron::Value -> Value) for v..CURRENT, loader bumps header.version. CURRENT_VERSION == MIGRATIONS.len()+1.
- `cargo run -p save --bin gen-scenarios -- --seed N [--out DIR]` writes DIR/{fresh,mid_growth,sprawl_crisis}.ron (default `saves/`). world_gen data real; sim run STUBBED (seeded synthetic zones/lines/metrics).
## Acceptance (`cargo test -p save`)
loader::{roundtrip_byte_identical, loader_rejects_future_version, loader_rejects_bad_magic_and_garbage, migrations_chain_applies_in_order}; scenario::{gen_scenarios_deterministic, scenarios_shape_and_roundtrip}.
