# city STATUS
2026-10-12: A3a (generation, road growth) and A3b (traffic, mode choice, rendering) implemented. Tests green on Mac (`cargo test --workspace`). Not yet seen on screen.
Open: congestion does not feed growth costs (growth cost matrix uses free-flow road time); no junction delay model; transit time uses flat wait/no transfer penalty until C1 trains; inter-town trips impossible (separate road components).
2026-10-12 B2: station menu (crates/app/src/station_menu.rs), zone legend chip (hud.rs). New transit actions ExtendLine, AddPlatform (+ Network.platforms, MAX_PLATFORMS) and ui_tools ToolAction::{ExtendLine, AddPlatform} + LineExtend click-to-finish mode. Platform count is stored and paid for; trains do not use it until C1.
