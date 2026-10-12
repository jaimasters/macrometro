# economy SPEC
Scope: Success metrics + build costs + land acquisition. Arch: land-acquisition; proposals/economy.
Crate: crates/economy (pub API = lib.rs only).
## Acceptance (machine-checkable)
- `cargo test -p economy` passes.
- TBD by manager in plan-feature.
## Model (D-6, D-8, D-9)
- AgencyId entity; per city per period: Budget{total, per_agency: weights by scorecard perf, conditions: Vec<SoftCondition{category, min_share}>, road_reserve}.
- Political standing per (city, agency); unmet condition -> standing penalty; standing + scorecard -> next budget.
- Favouritism strength rises with city car share (earmarks for road-adjacent/pet projects).
- Pure deterministic fn: next_budget(state, scorecards, car_share) -> Budget. Tests: same input same output; better-performing agency gets larger share; unmet condition lowers standing, never blocks spend.
