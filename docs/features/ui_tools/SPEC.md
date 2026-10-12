# ui_tools SPEC
Scope: Player tools: draw line, place station, line policies (macro), stop/segment edits (micro).
Crate: crates/ui_tools (pub API = lib.rs only).
## Acceptance (machine-checkable)
- `cargo test -p ui_tools` passes.
- TBD by manager in plan-feature.
## Budget/roads surfacing (D-7)
- Faint ghost meshes for RoadPlan; budget breakdown shows road_reserve as one quiet line. No popups.
- All player actions are commands tagged with AgencyId (D-8).
