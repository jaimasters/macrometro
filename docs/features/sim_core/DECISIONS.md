# sim_core DECISIONS
(format: D-n | date | decision | why | rejected)
- D-1 | 2026-10-12 | Pure-Rust core in src/core.rs, bevy only in plugin (lib.rs) | unit tests need no ECS; `rustc --test` loop; sim thread owns plain Rust state | sim inside bevy SubApp (heavier, bevy-coupled tests)
- D-2 | 2026-10-12 | Handoff = RwLock<Arc<T>> (std) | reader holds lock only to clone an Arc; no new dep | arc-swap/triple-buffer crate (add if contention measured)
- D-3 | 2026-10-12 | Generic `Simulation` trait; other crates (growth) implement it; wiring in app | sim_core must not depend on growth | sim_core depending on growth
- D-4 | 2026-10-12 | Commands carry no tick stamp; applied at next tick boundary in FIFO order | arch spec says so; determinism tests enqueue before stepping | tick-stamped commands (needed only for multiplayer/replay)
- D-5 | 2026-10-12 | Catch-up capped by max_ticks, excess time dropped | avoid spiral of death | unbounded catch-up
- D-6 | 2026-10-12 | wasm: no thread; `Sim::pump` time-sliced (D-arch-1) | no wasm threads | cfg-less thread spawn
- D-7 | 2026-10-12 | Command channel carries AgencyCommand; Pause/Resume/SetSpeed accepted from any agency | D-8 input-driven determinism | per-agency permissions (add with netcode)
