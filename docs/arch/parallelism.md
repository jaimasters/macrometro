# Parallelism
| System | Reads | Writes | Parallel with |
|---|---|---|---|
| reachability rows | network graph | C rows (dirty) | per-row parallel |
| flow solve | C, zones | F | per-origin parallel |
| line loading | F, lines | loads | per-line parallel |
| growth | A, C, loads | zone deltas | per-zone parallel |
| world_gen chunks | zone deltas | chunk meshes data | per-chunk parallel |
| economy/score | loads, F | metrics | anything read-only |
- Pipeline per tick: reachability -> flows -> loading -> {growth, economy} -> world_gen. Stages double-buffered so stage k of tick t overlaps stage k-1 of t+1 where data-independent.
- Use bevy_tasks ComputeTaskPool par_iter; no shared mutable state; deltas merged at stage end.
- Determinism: merge in sorted key order (test: same seed => identical snapshot hash, any thread count).
