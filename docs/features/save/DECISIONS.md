# save DECISIONS
(format: D-n | date | decision | why | rejected)
D-1 | 2026-10-12 | Migrations operate on untyped `ron::Value`, one fn per version step | old structs need not be kept compiled; matches spec fn(v)->v+1 | keeping every historical struct
D-2 | 2026-10-12 | No floats in save structs (mm ints, i64 funds) | byte-identical roundtrip and cross-platform determinism | f32 fields
D-3 | 2026-10-12 | Sim sections are minimal stub structs owned here for now | unblock format/loader/scenario; real types arrive from sim/transit/economy | opaque ron::Value blobs
D-4 | 2026-10-12 | gen-scenarios synthesises sim state from a seeded RNG | no headless sim runner exists yet; hook point is scenario::city() | blocking on sim_core
D-5 | 2026-10-12 | Seed->scenario stream: SplitMix64(mix(seed ^ (scenario+1)*0x5CE4A210)) | independent per scenario | shared stream
