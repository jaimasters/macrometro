# city DECISIONS
- C-1 | 2026-10-12 | Terrain-cost arterials + tensor-field local streets + access-gated growth (docs/arch/city-gen.md) | steerable by terrain, real patterns (grid core, curvilinear suburbs, cul-de-sacs, bridges) in ~900 lines | L-systems, full tensor-field placement, agent growth.
- C-2 | 2026-10-12 | Generated graph is built incrementally; zones need a built frontage road | "roads are real", D-7 | zones with parent links.
- C-3 | 2026-10-12 | Car routing/congestion lives in `city`, transit crate untouched; transit times computed from the Network by `city::transit_times` | no cross-manager edit; C1 swaps flat wait for train timetable | edit transit::cost_matrix.
- C-4 | 2026-10-12 | Zone-level logit mode choice on door-to-door minutes with CAR_BIAS; destination choice on car time | cheap, deterministic, gives the feedback loop | agent-level choice.
- C-5 | 2026-10-12 | Car dots are visual only (D-2), count from q*t per edge, capped pool, queue = bunching toward downstream end | no per-car sim cost | per-car queue sim.
- C-6 | 2026-10-12 | Transit/cost inputs reach the sim thread through a mutex (as before): timing vs ticks is not deterministic across runs of the whole app, only of the sim given identical inputs | sim_core Command has no payload; revisit for lockstep (D-8) | route through commands.
