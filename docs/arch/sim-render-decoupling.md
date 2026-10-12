# Sim/render decoupling
- Sim runs fixed SIM_HZ (10) in its own World on a worker thread (bevy SubApp-like, not FixedUpdate in render world).
- Output: Arc<SimSnapshot> (tick, zone stats, line states, chunk deltas) published via triple-buffer/arc-swap; render reads latest, never blocks.
- Render interpolates vehicle positions between last two snapshots by alpha = time since snapshot / tick period.
- Input: player commands queued (channel) and applied at next tick boundary.
- wasm: no threads by default -> sim runs time-sliced on main thread with a hard per-frame budget (render_budget.md) until wasm threads available. DECISION D-arch-1.
- Test: render frame time unaffected (+/-0.5ms) by doubling zone count in headless bench.
