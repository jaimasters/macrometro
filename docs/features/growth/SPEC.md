# growth SPEC
Scope: Zones, OD flows, accessibility, growth. Arch: aggregation, growth-drivers.
Crate: crates/growth (pub API = lib.rs only). Pure-Rust core in src/core.rs (no bevy); plugin in lib.rs wraps it.
## API (core.rs)
- `Zone{id,pop,jobs,capacity,crowding}`; `Zones(Vec<Zone>)` sorted by ZoneId.
- `CostMatrix{n, c: Vec<f64>}`: C[i][j] generalised cost, `f64::INFINITY` = unreachable; `new(n)`, `set`, `get`, `from_fn`.
- `accessibility(zones,&C,beta)->Vec<f64>`: A_i = sum_j jobs_j*exp(-beta*C_ij) (unreachable contributes 0, self cost counts).
- `gravity_flows(zones,&C,beta,trip_rate)->Flows`: singly-constrained OD; origin i emits pop_i*trip_rate trips split over reachable j by jobs_j*exp(-beta*C_ij). Row sums == emitted; unreachable rows emit nothing. `Flows{n,f}` with `get`, `row_sum`, `total`.
- `growth_deltas(zones,&A,&GrowthParams)->Vec<ZoneDelta>`: per zone independent (parallel-safe): g = rate * A/(A+k) * land_avail * crowd_factor, land_avail=max(0,1-(pop)/capacity), crowd_factor=1/(1+max(0,crowding-1)); dpop=g*pop_seed term, djobs=jobs_ratio*dpop.
- `apply_deltas(&mut Zones,&[ZoneDelta])`: merge in ascending ZoneId order, clamp pop<=capacity.
- `GrowthSim{zones,params}::step(&C)->GrowthStep{accessibility,flows}`; `state_hash()` (FNV over f64 bits).
- Plugin: resources `Zones`, `CostMatrix`, `GrowthParams`; system in FixedUpdate runs step and applies deltas. transit supplies CostMatrix (cross-crate note, OPEN_QUESTIONS).
## Acceptance (`cargo test -p growth`)
- cost_matrix_get_set_infinity
- accessibility_decreases_with_cost, accessibility_ignores_unreachable, accessibility_scales_with_jobs
- gravity_row_sums_equal_emitted, gravity_zero_for_unreachable, gravity_closer_gets_more, gravity_total_conserved
- growth_zero_when_full_capacity, growth_reduced_by_crowding, growth_increases_with_accessibility
- apply_deltas_sorted_order_deterministic, apply_deltas_clamps_capacity
- same_inputs_same_hash (50 steps), growth_converges_toward_capacity
- plugin_step_changes_zones (bevy App with MinimalPlugins, one FixedUpdate run)
## City road agency (D-7)
- Zone underserved (transit share < s_min for T ticks) -> city adds road plan + increases road_reserve (consumed by economy). Plans emitted one cycle ahead as RoadPlan events for UI ghosts. Deterministic test required.
