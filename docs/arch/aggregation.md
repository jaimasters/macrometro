# Aggregation / transitive closure
- Unit of sim out of focus = Zone (ZoneId), not agent. State: pop, jobs, demand vectors.
- Flows: OD matrix F[i][j] trips/tick between zones, solved by gravity model weighted by generalised cost C[i][j].
- C[i][j] from precomputed all-pairs reachability on the coarse transit+road graph (contracted graph; Dijkstra per changed source, incremental). Recompute only rows whose sources are within k hops of a network edit (dirty set).
- Hierarchy: zones nest (chunk LOD). Far cities = single super-zone; inter-city links summarise as one edge with capacity+cost.
- Focus region (camera radius R): zones spawn agents sampled from F (visual only, deterministic seed per zone+tick). Agents never feed back into F; F is authoritative.
- Cost: O(dirty_rows * E log V) per network edit; per tick O(nonzero F) flows. Budget: <2 ms/tick at 10k zones.
- Invariant (test): sum of agent trips in focus ~= F restricted to focus zones (+/-5% over 100 ticks).
- Links: sim-render-decoupling.md, parallelism.md, growth-drivers.md.
