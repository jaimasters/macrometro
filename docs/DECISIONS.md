# Global DECISIONS
- D-1 | 2026-10-12 | Cargo workspace, one crate per feature, each exposes one Bevy Plugin | isolation for agents | monolith.
- D-2 | 2026-10-12 | Zone flows authoritative; agents visual-only | cost scales with zones not pop | agent-based sim.
- D-arch-1 | 2026-10-12 | wasm sim time-sliced on main thread until wasm threads | WebGPU builds lack threads by default | block on threads.
- D-3 | 2026-10-12 | Saves store deltas over seed; RON | small, diffable, reproducible | full-state binary.
- D-4 | 2026-10-12 | Visual style abstract 3D diagram (Jai) | also caps material count -> fewer pipeline hitches.
- D-5 | 2026-10-12 | Endless sandbox; city move = reset; difficulty drift to LA sprawl (Jai).
