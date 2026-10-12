# city STATUS
2026-10-12: A3a (generation, road growth) and A3b (traffic, mode choice, rendering) implemented. Tests green on Mac (`cargo test --workspace`). Not yet seen on screen.
Open: congestion does not feed growth costs (growth cost matrix uses free-flow road time); no junction delay model; transit time uses flat wait/no transfer penalty until C1 trains; inter-town trips impossible (separate road components).
