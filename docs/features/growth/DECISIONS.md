# growth DECISIONS
(format: D-n | date | decision | why | rejected)
- D-1 | 2026-10-12 | Pure-Rust core in src/core.rs, bevy only in plugin | fast unit tests, parallelism later without ECS | systems-first design
- D-2 | 2026-10-12 | Dense f64 C and F matrices | <=few thousand zones now; 10k zones = 800MB dense, switch to sparse rows later (ponytail ceiling noted in code) | sparse from start
- D-3 | 2026-10-12 | Singly-constrained gravity (origin totals fixed, destination attraction = jobs) | row sums conserve trips, simple, matches aggregation.md | doubly-constrained IPF (add when jobs balance matters)
- D-4 | 2026-10-12 | Growth saturates: A/(A+k) * land_avail * crowd factor; pure per-zone fn, deltas merged sorted by ZoneId | growth-drivers.md; parallel-safe, deterministic | exponential growth unbounded
- D-5 | 2026-10-12 | Transit share, auto-roads trigger, chunk placement out of scope this pass | need transit/world_gen interfaces | stubbing them
- D-6 | 2026-10-12 | growth independent of sim_core; app wires GrowthSim into a sim_core Simulation | no cross-crate dependency | growth depends on sim_core
- D-7 | 2026-10-12 | RoadAgency takes transit shares as input slice; plan emitted at tick, built at tick+lead; road_reserve = cost of pending plans; zone with pending plan can't re-plan | decoupled from transit; one-cycle-ahead ghosts | duplicate plans per zone
