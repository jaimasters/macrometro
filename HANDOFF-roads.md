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

## Queued Jai items (2026-10-13), fold into next batch
1. Junction control (folds into item 1, and pulls queue row 12 forward): a 3/4-leg junction defaults to give-way on the lesser-used leg(s). Upgrade it to a roundabout only when queueing, volume or class is too high. Never put a roundabout on a node with 2 or fewer legs. Tests: give-way assignment and obedience, upgrade over the threshold, no ring at degree <= 2.
2. Growth falls off with distance from the centre or with density, so the outskirts stay sparse. Test: median plot distance from the centroid stays below a bound, and density decreases with distance.
3. No roads, plots or driveways on water in seed or growth; roads route around it. Test across seeds.
4. Very subtle city-limit underlay (dense zone) and region-limit underlay (sparse zone), drawn from the same values growth uses. Test: plots spawn inside their zone, and the underlay geometry matches.
5. Left-hand-driving setting doesn't persist: fix at the root in settings save/load. Test: set, save, reload, assert.
6. Cars are sized in world units only, with no pixel minimum; they fade or hide when too small. Use one world-to-screen scale shared with item 4 (far-zoom stroke). Test: zoom-out size <= zoom-in size, and car < lane at every zoom.
7. "Clear traffic" debug button that despawns all cars. Mark it with a `ponytail:` comment saying it is temporary. Test: count is 0, then cars respawn.
