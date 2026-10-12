# save STATUS
state: built (format, loader+migrations, scenario bin skeleton); verified 6 tests + cargo check --workspace on Mac
owner: mgr-world
done: format.rs, loader.rs (to_ron/from_ron/load_with), scenario.rs, bin gen-scenarios (byte-deterministic confirmed)
next: replace stub sim sections with real types; replace scenario::city() synthesis with headless sim run; wire SavePlugin (save/load commands) once app/sim_core need it; first real migration when format bumps.
