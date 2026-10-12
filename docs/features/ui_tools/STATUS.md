# ui_tools STATUS
state: done for shell. owner: mgr-ui
- Tools: [1] place station, [2] click stops + Enter = line, Esc none. Emit `ToolCommand{agency: sim_core::AgencyId, action}` messages; app translates to transit.
- `GhostRoads` message (full set) drawn as faint translucent cuboids. `GroundHeight` resource for picking.
- Tests: `cargo test -p ui_tools` (picking, click state machine, ghost replace, key->command).
- Not done: line policies (macro), stop/segment edits (micro).
