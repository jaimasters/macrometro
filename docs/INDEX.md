# INDEX (route from here; one read)
## arch
- docs/arch/aggregation.md — Out-of-focus sim via zone flows + precomputed reachability; detail only near camera.
- docs/arch/multi-city.md — Many cities in one logical world, linked by inter-city edges; each city is a save-able region.
- docs/arch/growth-drivers.md — How transit utilisation, accessibility and congestion drive zone growth and sprawl.
- docs/arch/land-acquisition.md — Eminent domain vs greenfield cost model rewarding foresight.
- docs/arch/parallelism.md — Independent sim systems and their parallel schedule.
- docs/arch/sim-render-decoupling.md — Fixed-step sim off the render thread; immutable snapshot handoff.
- docs/arch/render-budget.md — Monitor-locked framerate; priority-ordered, deferrable render work; residual risks.
- docs/arch/sound.md — Pure-Rust synth via bevy_audio Decodable; event->scale->voice limiter.
## features (SPEC/DECISIONS/STATUS each; crate crates/<name>)
- docs/features/sim_core/ — Fixed-step clock, sim thread, snapshot handoff, command queue. Arch: sim-render-decoupling, parallelism.
- docs/features/world_gen/ — Chunk grid, seeded terrain, city regions, sprawl placement. Arch: multi-city, growth-drivers.
- docs/features/transit/ — Rail+road graph, lines, RailClass specialisation and mismatch penalty. Arch: aggregation.
- docs/features/growth/ — Zones, OD flows, accessibility, growth. Arch: aggregation, growth-drivers.
- docs/features/economy/ — Success metrics + build costs + land acquisition. Arch: land-acquisition; proposals/economy.
- docs/features/render_budget/ — Frame pacing, priority work queue, pipeline precompile. Arch: render-budget.
- docs/features/save/ — Versioned world save format, loader, seeded scenario generator.
- docs/features/ui_tools/ — Player tools: draw line, place station, line policies (macro), stop/segment edits (micro).
- docs/features/sound/ — Procedural soundscape: SoundEvent -> quantised tones + ambient bed. Arch: sound.
- docs/features/app/ — Composes plugins; binary + wasm entry.
## other
- docs/game-loop.md — sandbox loop, failure, city reset, difficulty drift
- docs/proposals/economy.md — success/economy options (Jai to choose)
- docs/proposals/city-agency.md — game's own role (auto-roads)
- docs/OPEN_QUESTIONS.md — undecided items
- docs/DECISIONS.md — global decisions
- docs/GLOSSARY.md — terms
- docs/AGENTS.md — manager roster, ownership, spawn rules
