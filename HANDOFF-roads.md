# Handoff: roads worker (mgr-roads), 2026-10-13

The last green commit is be0f6ab, pushed to mgr-roads. Checks at that commit, run on the Mac:
- `cargo test --workspace` passes.
- The release build succeeds.

Do not merge to main. mgr-city owns these files: trains.rs, segment_menu.rs and persist.rs.

## Done
- **Roundabouts:** 871e1dc
- **Markings declutter:** d9e2c59
- **R42 design note:** 785cc11, at docs/design/organic-growth.md
- **Car despawn fix:** 8c3e6ed. Cars only looked as if they vanished: sprites were indexed by Vec position, and that index shifts on retain. Each car now has a stable slot.
- **R43 preset generator:** be0f6ab. Details in the next section.

## R42 (a)/(b) and R43 state
- `crates/city/src/organic.rs` provides `plan_preset(seed, core, Preset) -> TownPlan`. The presets are `Preset::{Hamlet, SmallTown, LargeTown}`, and `shape()` sets roads, reach, streets and houses for each.
- **Skeleton:** one or two roads that cross at the core. Each grows until it is `reach` from the core. Bends are steered back toward the axis by `+off*0.01`; this sign was a bug, now fixed.
- **Curves:** a curve is a chain of short edges, 16 m (`STEP`) each, joined at degree-2 nodes. The REdge `ctrl` field from the design note was NOT added, so the save format is unchanged.
- **Side streets:** `G::grow` branches from existing degree-2 nodes only. A street stops when its next segment would cross another road or come within 10 m of one.
- **Houses:** set back 6 m (`SETBACK`) on both sides of each edge and sorted by distance from the core. The first `houses` of them are built (`start_pop` is 40).
- **Test:** `presets_are_connected_reachable_and_every_house_has_access` runs every preset on seeds 1, 7 and 42. It checks that the network is connected, that the skeleton reaches at least 0.9 × reach, that every house is within its setback of its access edge, and that no two edges cross.

## Not done
1. **Wire the reset menu.** Replace `plan_towns` with `plan_preset(new_seed, core, preset)`. Its caller is app/src/layout.rs:14, and the reset logic appears to live in persist.rs, which mgr-city owns. The orchestrator has been asked who does this.
2. **Remove the unused `order` variable** at organic.rs:94. It causes two compiler warnings.
3. **R42 (c) and (d)**, as described in the design note.
4. **Leftovers:** draw the overpass; make upgrades re-align roads and pay eminent domain.

## Car code map (crates/app/src/roads.rs)
- **Constants and types:**
  - `MAX_CARS` (420) is at line 21.
  - `struct Journey` (line 164) has the fields trip, seg, along, v, wait and slot.
  - `struct Journeys` (line 178) has a `rings` field of type `HashMap<(u32,u32), Ring>`.
- **`follow_step` (line 236):** the car-following step. In order, it:
  1. keys cars by lane;
  2. holds cars at junctions of degree 3 or more;
  3. checks there is room on the exit before a car enters;
  4. breaks gridlock;
  5. runs cars on ring segments at speed × 0.6.
- **`ring_trip` (line 313):** splices the roundabout arc into a trip's points. The Ring geometry is `Ring::at` and `path` in crates/city/src/junctions.rs. The ring radius is `2.6 + w*0.5`, clamped to 40% of the shortest leg.
- **Slots and spawning:**
  - At spawn (around line 571), each car takes the lowest free slot.
  - At draw (around line 585), cars are mapped by slot through `by_slot`.
  - Cars are removed only on arrival. A test asserts that every removed car had less than 0.5 left to travel.
- **Markings:**
  - `edge_marks` (line 67) draws an edge's markings. Lines are cut at the trim distance. A stop bar has `fat` 1.5.
  - `update_marks` (line 422) sets the trim for junctions of degree 3 or more: `half.max(1)+0.6` normally, or `r+1.2` at a ring.
  - Small marks are hidden when zoomed out (`w > 0.4`).
- **Tests:**
  - `junction_flow(ring)` (line 733) drives two tests: `heavy_junction_traffic_never_deadlocks` and `roundabout_traffic_flows_without_overlap`.
  - The throughput check is skipped for the first 400 steps (warmup).

## Checks
1. Spend about 5 seconds checking that the Mac answers over ssh. If it answers, run:
   `rsync -a --delete --exclude target --exclude .git ./ 100.104.143.66:mm-roads/ && ssh 100.104.143.66 'cd ~/mm-roads && export PATH=$HOME/.cargo/bin:$PATH && cargo test --workspace -q && cargo build --release -q'`
2. If the Mac does not answer, run `nice -n 10 cargo test` locally.
