# ui_tools STATUS
state: done for shell. owner: mgr-ui
- Tools: ActiveTool {Pan, PlaceStation, DrawLine}; keys 1/2/3 or the app palette. Line = press a station (or empty grid cell), drag, release on another; live translucent octilinear preview. Emits `ToolCommand{agency: sim_core::AgencyId, action}`.
- Pure helpers: snap/snap_station, finish_drag, octilinear/route/path_len, ray_terrain, segment_transform. `GhostRoads` (full set per message) drawn as faint strokes. `GroundHeight` for picking.
- UI hover guard: pointer ignores clicks while any bevy picking `Hovered` UI node is hovered.
- Tests: `cargo test -p ui_tools` (8).
- Not done: line policies (macro), stop/segment edits (micro), extending an existing line.
