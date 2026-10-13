# HANDOFF — roads (mgr-roads)

## Done (workspace green: 222 passed, 0 failed, 1 ignored)
- 6932929: unified generator. organic.rs grow_town (centre falloff, arterial seed) + assemble (cross-town merge_close,
  prune_stubs, houses with dry driveways, capacity falls off with distance); plan_preset/plan_towns both use it.
  Relaxed asserts: roadgen seed-built <60% and parcels 40..=600; traffic congestion compares same-town car_time;
  zones 5-min growth <= 3x start and < n/4.
- 8a06de4: tests for one component per town (all presets + plan_towns, 6 seeds), nothing on water, centre-weighted density.
- 8d3a41f: junctions::control (give-way on lesser legs, roundabout over ROUNDABOUT_LOAD, Free at deg<=2) + entry_delay; tests.

## Next
1. Wire junctions::control into the app: render give-way signs/markings instead of Ring for GiveWay nodes, feed per-node
   load from traffic, apply entry_delay in car movement + traffic cost. Ring::at/clearance still assume a roundabout at deg>=3.
2. plan_towns towns from different regions are separate components (no inter-town link). Decide whether to link them.
3. Not visually checked: whole layout (large-town character, falloff, house angles). Run the game and look.
4. Item 2 (click a car → destination + route) not started. Then queue rows 6, 7, 5.
5. Merge only after a visual check: `git -C /mnt/ssd/work/macrometro checkout docs/REQUIREMENTS.md`, then ff-merge.
Builds: scratchpad mt.sh rsyncs to Mac ~/macrometro-roads-sync and runs cargo test there.

## Queued Jai items (2026-10-13), fold into next batch
1. Junction control (folds into item 1, and pulls queue row 12 forward): a 3/4-leg junction defaults to give-way on the lesser-used leg(s). Upgrade it to a roundabout only when queueing, volume or class is too high. Never put a roundabout on a node with 2 or fewer legs. Tests: give-way assignment and obedience, upgrade over the threshold, no ring at degree <= 2.
2. Growth falls off with distance from the centre or with density, so the outskirts stay sparse. Test: median plot distance from the centroid stays below a bound, and density decreases with distance.
3. No roads, plots or driveways on water in seed or growth; roads route around it. Test across seeds.
4. Very subtle city-limit underlay (dense zone) and region-limit underlay (sparse zone), drawn from the same values growth uses. Test: plots spawn inside their zone, and the underlay geometry matches.
5. Left-hand-driving setting doesn't persist: fix at the root in settings save/load. Test: set, save, reload, assert.
6. Cars are sized in world units only, with no pixel minimum; they fade or hide when too small. Use one world-to-screen scale shared with item 4 (far-zoom stroke). Test: zoom-out size <= zoom-in size, and car < lane at every zoom.
7. "Clear traffic" debug button that despawns all cars. Mark it with a `ponytail:` comment saying it is temporary. Test: count is 0, then cars respawn.

## Added by main chat 2026-10-13 (Jai): level crossings
Where a transit line crosses a road: cars stop while a train occupies or approaches the crossing (barrier/light state); draw a crossing marking at the intersection point. Headless test: no car is inside the crossing while a train is in it. Queue after the current items.

## Added by main chat 2026-10-13 (Jai): transit line zoom LOD
Transit lines are currently drawn thicker as you zoom out; they must do the opposite. Keep the line's real-world width, so it narrows on screen as you zoom out, and clamp it to a minimum pixel width so it stays visible. Transit vehicles (trains/trams) DO get a minimum on-screen size and scale up at far zoom so they stay visible. This is the opposite of road cars, which fade out (see item 6). Use the same shared world-to-screen scale. Headless test: line screen width never increases as zoom goes out, and never drops below the minimum; vehicle screen size never drops below its minimum.

## Added by main chat 2026-10-13 (Jai): transit rendering — end stations, rail texture, junctions
Screenshot: the end station renders as two overlapping blue pill shapes with a dark bar between them, poking past the line's end. Fixes:
- (a) An end station is one clean terminus shape aligned to the line, with the line ending at it and no overlap or doubling.
- (b) Rail lines need a rail texture: sleepers/ties or a twin-rail pattern, so rail reads differently from road.
- (c) With (b) in place, build proper rail junction layout and rendering: switches/turnouts where lines branch or merge, with tracks drawn through them correctly.
Headless tests: the terminus mesh has no self-overlap and stays within the line's end bounds; every rail segment carries the rail material; a branching rail node produces turnout geometry connecting each pair of legs it serves. Report as not visually checked unless verified on screen.

## Added by main chat 2026-10-13 (Jai): feedback on large-town layout
Jai: the large-town layout is "a good starting point" and works well; keep its character. The one defect is a disjoint network: several streets and road clusters are not connected to the rest. This is top priority within item 1 (connectivity enforced in the growth step). Every new road must attach to the existing network at creation, with no orphan segments. Headless test: across several seeds and sizes, the road graph has exactly one connected component.

## Added by main chat 2026-10-13 (Jai): click a car to see its destination
Clicking a car selects it and shows:
- its destination, highlighted on the map (the building/plot);
- its planned route, drawn as an overlay from the car to the destination;
- a small label with the trip purpose (e.g. home→work) if known.
Click empty space or press Esc to deselect; the overlay follows the car and clears on arrival or despawn. Pick by world-space hit test, which also works at far zoom given the car scaling rules. Doubles as a gridlock debugging tool. Headless test: selecting a car exposes its destination and route; the route ends at the destination; selection clears on despawn. Do this early, because it helps debug the traffic items.
