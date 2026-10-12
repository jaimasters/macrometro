# world_gen STATUS
state: built (core + plugin); verified 14 tests + cargo check --workspace on Mac
owner: mgr-world
done: rng, terrain, chunk grid, city regions, plugin (WorldSeed, Cities resources)
next: sprawl placement (needs growth drivers); rendering mesh consumers use chunk_heights; golden hash only verified on aarch64 (macOS+Linux), not x86/wasm.
