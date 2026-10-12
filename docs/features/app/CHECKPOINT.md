# mgr-ui checkpoint (2026-10-12, HEAD after Stage B bc1e7b0 + this doc)
Rules: never launch the game on the Mac (hot-reload runner owns ~/macrometro). Verify only with `cargo test --workspace` + `cargo build` in ~/macrometro-wt-ui via rsync (script idea: rsync worktree -> ssh Mac -> source ~/.cargo/env -> cargo). No workers; stop on 502/429. Commit on mgr-ui; coordinator merges.

## Where things are (crates/app/src)
- layout.rs: thin adapter over crates/city (`plan_layout`, `build_layout`), `town_name`, `station_name`. Generation, road growth, traffic: crates/city, docs/arch/city-gen.md. roads.rs: road strokes by class, congestion tint (zoomed out), car dots (zoomed in).
- zones.rs: `GrowthSimulation` (sim thread; wasm Sim::pump) = growth + city RoadBuilder + Traffic; `GROWTH_RATE`; road-access-gated growth; `Shared` mutex carries Costs + transit-time matrix; mirrors built roads into transit; ghost roads; real CarShare/Scorecards.
- shell.rs: ToolCommand -> transit commands + SpendQueue, `Lines` (cells per player line), line/train visuals (trains are still visual ping-pong), sound events.
- stations.rs: platform stadium footprints striped in line colours, crowding gauge (stub), name labels when view_h<=150, hover/select/delete rings, `SelectedStation`.
- hud.rs: chips (Budget, Standing), palette (Pan/Station/Line/Delete/Sound), hints. camera.rs: top-down ortho, drag pan, scroll zoom. terrain.rs: flat land/water chunks.
- ui_tools: ActiveTool {Pan,PlaceStation,DrawLine,Delete}, one-shot tools, right-click delete, drag line with octilinear preview, `CursorWorld`, `Stations`, `GhostRoads`.
- transit (edited with coordinator approval): `TransitAction::RemoveNode`.

## Pending queue (coordinator order): A3 -> B2 -> C1 -> C2
A3 (roads/traffic) and B2 (station context menu + zone legend) are specified in the coordinator's latest message. 
C1 plan I had designed (not started, nothing uncommitted): new `crates/transit/src/trains.rs`, deterministic f64 fixed-step 1D train sim per line route:
- Edge.tracks (1 one-way a->b, 2 two-way, 4 = 2 per direction); Network.platforms per node; actions AddOneWayRail, UpgradeTracks(edge) (cost len*C*tracks_added), UpgradePlatform(node); AddLine: opposite direction on 1-track -> WrongWay; non-loop line needs all segments >=2 tracks (loop = first stop == last stop).
- Train {s, v, dir, lane, next stop, slot, dwell}; TrainType table per RailClass (vmax, accel, brake, length, dwell); brake profile v<=sqrt(2*b*rem) to the platform mark; leader cap v<=sqrt(2*b*(gap-MIN_GAP)) on same line/dir/lane; curve cap sqrt(Vc^2+2*b*d), Vc a function of bend radius (new requirement: arc bends, radius pickable/upgradable; larger radius = faster but demolishes zones, cost+standing, preview first).
- Platform slots per station: need a free slot to enter, else hold at throat (mark-THROAT); slot index = lane while in station zone; overtaking only on 4-track segments.
- Required tests: opposite-direction on 1-track rejected; 1-track loop runs one way; full station holds arrival; +platform raises throughput; stop within tolerance of mark; never overlaps train ahead; travel time matches analytic accel/cruise/brake profile; identical across runs.
- C2: render trains from sim state with interpolation (prev_s), N rails zoomed in with direction arrows, platform slots, +track/+platform UI with cost.
Cross-crate needs open: transit waiting passengers (replace crowding stub), sim_core Command payload. A3 done (city crate). C1 also needs the auto-routing track placer (R21): min-radius arcs, avoids buildings/roads, mouse steers side, eminent domain + level crossings priced.
