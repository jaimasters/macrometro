# HANDOFF — roads (mgr-roads)

## Done
- cab31a4: queue row #10 (give-way signs at T-intersections) added.
- WIP (amended commit "wip: unify city generation", NOT green, NOT merged):
  - organic.rs rewritten: `Town { seed, step, run }`; `plan_preset(world, rng_seed, core, preset)`
    = seed + preset steps (order 0) + FUTURE_STEPS (orders 1..). Preset::shape() = (roads, reach, steps, houses).
  - roadgen.rs: old gen_town/Grid/Gen deleted; plan_towns calls plan_preset(SmallTown) per region; SEED_ORDER=0.
  - app/layout.rs uses the new signature.

## Failing (cargo test -p city on Mac via mt.sh)
- organic: Hamlet seed 1 has a 1-segment stub (node 14); multi-town clearance: node 971 3.73 from junction 457
  (merge_close only runs inside step; towns from different regions are never merged with each other → run
  merge_close / clearance check across the merged graph in plan_towns, or keep towns apart).
- roadgen_tests: 0 arterials (seed must emit arterials), parcel on road 1035, lot not aligned (d<=7 rule) — old
  generator-specific asserts; relax or rewrite against the new generator.
- roadbuild: builds only 2 in 120 s; "no underserved plans built"; stall/failsafe test → future edges with order>=1
  are too few or not adjacent to the built net. Check that FUTURE_STEPS edges attach to built nodes.
- traffic: a_line_serving_a_busy_origin…, congestion_raises_car_travel_time (depend on plan_towns(7) layout).

## Next
1. Fix the above, `cargo test --workspace` green, commit item 1 + queue rows 9 done / 8 absorbed.
2. Items 2–4 (queue rows 6, 7, 5): LANE_W, one-lane tier, far-zoom stroke check.
3. `git -C /mnt/ssd/work/macrometro checkout docs/REQUIREMENTS.md` then ff-merge mgr-roads. Never merge red.
Builds: scratchpad mt.sh amends WIP, pushes, runs cargo on Mac ~/macrometro-roads.
