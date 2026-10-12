# OPEN QUESTIONS (for Jai)
- Q1 Economy: P1/P2/P3 (proposals/economy.md). Default assumed: P1.
- Q2 City auto-roads: A vs B (proposals/city-agency.md). Default assumed: A.
- Q3 Scorecard metrics weighting / which are shown.
- Q4 Can old cities be revisited and edited, or only observed?
- Q5 Mismatch penalty magnitude for RailClass vs stop spacing/median running.
- (mgr-sim) transit must write `growth::Costs` (CostMatrix over zones, INFINITY = unreachable); app must feed `Growth` zones from world_gen and wire GrowthSim into a sim_core `Simulation` (sim_core::SimCorePlugin currently runs CoreSim placeholder). wasm: app drives `Sim::pump` with render_budget. Cross-crate; via coordinator.
