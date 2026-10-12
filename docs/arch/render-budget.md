# Render budget
- PresentMode::Fifo (vsync) => locked to monitor. Budget B = refresh period - margin (20% default).
- Each frame: work items (chunk mesh upload, LOD swap, agent instance update, label layout) in priority queue; executed until elapsed > B*0.7 of CPU budget; rest deferred, stale items cancelled.
- GPU side: cap draw count/instance updates by measured GPU time (timestamp queries where supported) with EWMA; adapt LOD bias.
- Residual risks (cannot eliminate): shader/pipeline compile hitches; OS preemption; GPU driver stalls; wasm GC/main-thread contention; asset IO.
- Mitigations: precompile all pipelines at load behind loading screen (warm every material x mesh layout); fixed small material set (abstract style helps); budget margin; async mesh building on tasks; frame-time telemetry with hitch counter.
- Test: headless pacing harness asserts p99 CPU frame work < B under synthetic load.
