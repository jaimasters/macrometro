# economy SPEC
Scope: Success metrics + build costs + land acquisition. Arch: land-acquisition; proposals/economy.
Crate: crates/economy (pub API = lib.rs). Pure core src/core.rs (no bevy). Uses `sim_core::AgencyId` (D-8), re-exported.
## Model (D-6, D-7, D-8, D-9)
- `next_budget(&EconomyState, &[(AgencyId,Scorecard)], car_share, road_demand)->Budget{total,road_reserve,per_agency}`. Pure, BTreeMap-ordered.
- total = base*(0.5+0.5*mean_perf+0.5*mean_standing). road_reserve = min(road_demand, total*max_road_reserve); `road_demand` = growth `AgencyOutput::road_reserve`. Agency pool = total - road_reserve (competes, D-7).
- Split weight = (perf+eps)^(1+car_share)*(0.5+standing): favouritism rises with car share; RoadAdjacent earmark condition = earmark_k*car_share.
- Scorecard = 6 metrics, equal weights (D-9). Soft conditions: shortfall lowers standing in `close_period`; `record_spend` never blocked by conditions.
- Land: `land_cost`, `build_cost`, `reserve`, `land_value` per land-acquisition.md.
- Plugin: resources Economy, Scorecards, CarShare, RoadDemand, CurrentBudget, SpendQueue (SpendCommand carries AgencyId), SpendErrors, PeriodClock.
## Acceptance (`cargo test -p economy`)
- next_budget_deterministic, better_performer_larger_share, favouritism_concentrates_with_car_share, budget_conserves_total, road_reserve_competes_with_agency_pool, earmark_rises_with_car_share
- unmet_condition_lowers_standing, unmet_condition_never_blocks_spend, met_condition_keeps_standing, standing_clamped_and_feeds_next_budget, spend_unknown_agency_rejected
- greenfield_cheaper_than_developed, reserved_corridor_cheaper_than_later_acquisition, land_value_rises_with_accessibility, build_cost_sums_base_and_land
- plugin_period_issues_budget
