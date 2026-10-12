# transit DECISIONS
(format: D-n | date | decision | why | rejected)
- D-1 | 2026-10-12 | Penalty = 0.30 * mean(spacing_mismatch, alignment_mismatch); spacing mismatch = |ln(spacing/ideal)|/ln4 capped 1 | D-9 provisional cap, symmetric for too-short/too-long | per-class tables
- D-2 | 2026-10-12 | Cost matrix = shortest travel time over road+rail; no transfers/waits yet | smallest model feeding growth; deterministic Dijkstra | incremental dirty rows (ponytail, add when Z large)
- D-3 | 2026-10-12 | Transit share = logit on rail-only vs road-only time, jobs-weighted over destinations | gives RoadAgency a per-zone slice | full mode-choice sim
- D-4 | 2026-10-12 | Class profiles (speed, ideal spacing, capacity) are provisional tuning constants | no spec numbers | none
