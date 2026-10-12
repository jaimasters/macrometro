# app STATUS
state: playable shell (verified: tests + native launch; no screenshot, Mac ssh session has no display)
owner: mgr-ui
done: terrain chunks flat-shaded (terrain.rs), orbit/pan/zoom camera, zones from world_gen cities run as sim_core Simulation (zones.rs; thread native, Sim::pump on wasm, wasm unverified: target absent on Mac check), blocks by pop, RoadAgency ghosts, ui_tools commands -> TransitCommands + SpendQueue (shell.rs), trains ping-pong on drawn lines, HUD budget.
cross-crate needs: (1) sim_core Command lacks payload room, so costs/shares cross to the sim thread via Arc<Mutex> (zones::Shared); (2) growth::RoadPlan has no geometry, app draws zone->nearest-zone segment; (3) growth::Growth resource is left empty so growth_system does not double-step; transit weights default to 1.0 because of it; (4) Scorecards/CarShare are stubs (shell::stub_metrics); (5) save needs the app's zone layout (zones::build_zones) if saves should restore pops.
