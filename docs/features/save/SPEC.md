# save SPEC
Crate: crates/save.
- Format: RON (serde), header {magic:"MMSAVE", version:u32, seed:u64, tick:u64}, sections: world(seed, chunk overrides), cities[](id, region, zones, network, lines, metrics).
- Generated content NOT stored: regenerated from seed; only player/sim deltas saved.
- Loader: version-gated migrations fn(v)->v+1.
- Scenario generator: `cargo run -p save --bin gen-scenarios -- --seed N` writes saves/{fresh,mid_growth,sprawl_crisis}.ron by running headless sim with scripted player policies for fixed ticks. Reproducible: same seed+version => byte-identical file (test).
## Acceptance
- roundtrip test: save->load->save byte-identical.
- gen-scenarios determinism test.
- loader rejects unknown future version with error.
