# transit SPEC
Scope: Rail+road graph, lines, RailClass specialisation and mismatch penalty, zone cost matrix. Arch: aggregation.
Crate: crates/transit (pub API = lib.rs). Pure core src/core.rs (no bevy); plugin in lib.rs. Depends on sim_core (AgencyId) and growth (CostMatrix, Costs; not edited).
## API
- `Network{nodes,edges,lines}`; `apply(&TransitCommand{agency,action})` with `TransitAction::{AddNode,AddEdge,AddLine,RemoveLine}`; every command carries AgencyId (D-8); only owner removes a line.
- `mismatch_penalty(class, spacing, alignment_mismatch)` in 0..=0.30 (D-9); `effective_speed/capacity`.
- `cost_matrix(&Network,&zone_nodes)->growth::CostMatrix` (best road/rail travel time; INFINITY unreachable; diag 0). Rail edge usable only if a line serves it, at the line's penalised speed.
- `transit_shares(&Network,&zone_nodes,&weights)->Vec<f64>` per-zone transit share for `growth::RoadAgency::observe`.
- Plugin: resources TransitNet, ZoneNodes, TransitCommands, TransitErrors, TransitShares; FixedUpdate applies commands, then (if dirty) writes `growth::Costs` and `TransitShares`.
## Acceptance (`cargo test -p transit`)
- penalty_zero_when_matched, penalty_capped_at_30_percent, penalty_monotone_in_spacing_mismatch, penalty_monotone_in_alignment_mismatch, penalty_scales_speed_and_capacity
- matched_spacing_gives_cheaper_trip_than_mismatched
- cost_matrix_zero_diag_and_symmetric, cost_matrix_unreachable_is_infinite, cost_matrix_prefers_faster_rail_over_road, unserved_rail_edge_is_unusable, cost_matrix_deterministic, cost_matrix_feeds_growth_accessibility
- commands_reject_unknown_node_bad_len_and_short_line, line_requires_rail_edges, commands_carry_agency_and_only_owner_removes_line
- transit_share_zero_without_rail_and_rises_with_it, plugin_publishes_costs_and_shares_to_growth
