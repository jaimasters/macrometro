# sim_core STATUS
state: verified
owner: mgr-sim
done: core.rs (clock, queue, handoff, Sim, SimThread, pump, CoreSim, hash) + plugin; 13 tests green (Mac, cargo test -p sim_core)
next: app wires real Simulation (growth) + wasm pump (needs app crate; see OPEN_QUESTIONS)
