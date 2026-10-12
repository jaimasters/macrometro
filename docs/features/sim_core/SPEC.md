# sim_core SPEC
Scope: Fixed-step clock, sim thread, snapshot handoff, command queue. Arch: sim-render-decoupling, parallelism.
Crate: crates/sim_core (pub API = lib.rs only). Pure-Rust core in src/core.rs (no bevy); plugin in lib.rs wraps it.
## API (core.rs)
- `FixedClock{hz}`: `advance(dt:Duration, max_ticks:u32)->u32` ticks due; remainder kept; excess beyond max_ticks dropped (no spiral of death); `alpha()->f32` in [0,1).
- `Simulation` trait (Send+'static): `type Snapshot: Send+Sync`; `apply(&mut self,&Command)`; `step(&mut self,tick:u64)`; `snapshot(&self,tick:u64)->Snapshot`; `state_hash()->u64`.
- `Command` enum: Pause, Resume, SetSpeed(u8), Custom(u32,i64). `CommandQueue`/`CommandSender`: mpsc, drained at tick boundary in send order.
- `Handoff<T>`: publish(Arc<T>) / latest()->Arc<T>; reader never waits on sim work (lock held only to swap/clone an Arc).
- `Sim<S>`: owns S, tick, queue rx, handoff; `tick_once()` = drain cmds -> step (unless paused; speed = steps per tick) -> publish snapshot.
- `SimThread::spawn(sim, hz)` (non-wasm): worker thread driven by FixedClock; `stop()`/drop joins. wasm: `Sim::pump(dt, budget_ticks)` time-sliced on caller.
- `CoreSim`: reference Simulation (seeded xorshift state + tick, paused, speed) used by tests and the plugin; `CoreSnapshot{tick,hash,paused,speed}`.
- `fnv1a(&[u64])->u64`, `SeededRng` (xorshift64*).
## Acceptance (machine-checkable, `cargo test -p sim_core`)
- clock_accumulates_fractional_ticks, clock_caps_catchup, clock_alpha_in_range
- command_queue_fifo_applied_at_tick_boundary, pause_resume_speed_commands
- handoff_latest_never_blocks_during_publish, handoff_returns_newest
- same_seed_same_hash (100 ticks, same command script => identical hash), different_seed_different_hash
- thread_matches_direct_stepping (thread snapshot at tick N has hash == directly stepped hash at N), thread_stop_joins
- pump_budget_limits_ticks
- plugin_updates_simtick (bevy App update reads latest snapshot into SimTick)
## Agencies (D-8)
- `AgencyId(u32)` defined in sim_core; every command is an `AgencyCommand{agency,cmd}` (`cmd.by(agency)`); `Simulation::apply(&AgencyCommand)`. Test: identical_command_streams_identical_hashes.
