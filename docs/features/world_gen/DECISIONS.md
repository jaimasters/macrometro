# world_gen DECISIONS
(format: D-n | date | decision | why | rejected)
D-1 | 2026-10-12 | Own SplitMix64 hash + value noise, no rand/noise crate | output must be bit-stable forever (saves regenerate from seed); zero deps | `rand`, `noise` (version drift changes worlds)
D-2 | 2026-10-12 | chunk_hash_golden pins terrain output; change only with a save-format version bump | catches accidental world changes | none
D-3 | 2026-10-12 | Terrain is a function of world cell coords; chunks are samples | seamless borders, trivial LOD | per-chunk seeded noise (seams)
D-4 | 2026-10-12 | Cities: 1 candidate/24-chunk super-cell, 4-chunk margin, radius<=4 | disjointness by construction, no rejection loops | Poisson-disc (order-dependent)
D-5 | 2026-10-12 | Floats f64 add/mul/floor only, no fma/trig | IEEE determinism across arm64/x86/wasm | libm functions
