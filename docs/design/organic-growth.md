# R42 Organic growth: design note

## Problem
Towns grow from a pre-planned straight-edge grid (`roadgen::plan_towns`), built outward by graph order (`REdge.order`).
The result reads as a lattice: straight segments, junctions wherever segments meet, houses spread evenly from day one.
The target is towns that look grown: curved roads, junctions only where roads actually cross or branch, sparse
outskirts that infill over time, and roads that get built or upgraded because traffic needs them.

## Data model: curved edges, same indices
- Keep `RoadGraph { nodes, edges }` and every node and edge index exactly as they are. Saves, `RoadClasses`,
  `built[]`, parcels' `access` and router keys all refer to edge indices, so nothing that is persisted changes shape.
- Add `ctrl: Vec<Vec2>` to `REdge`: interior control points, empty for a straight edge. The polyline is
  `[a, ctrl.., b]`, rendered and routed as segments. `len` becomes the polyline length.
- Old saves load because the plan is regenerated from the seed. Curvature only applies to plans made with the new
  generator version. Saved state is per edge index, and slice (a) keeps the same index order for an unchanged seed.
  If index order would change, the generator version is bumped and old saves keep the old generator path.
- `RoadGraph::pos(e)` keeps returning the end nodes. A new `RoadGraph::poly(e)` returns the full polyline, and each
  caller that draws or routes (road strokes, markings, `Trip.pts`) switches to it one at a time.
- Junctions are the nodes with 3 or more incident edges. A node of degree 2 is a bend, and `junctions::smooth` already
  merges collinear ones. Roundabouts and stop bars already key on degree 3+, so curved edges need nothing new there.

## Growth state drives placement
- Each town tracks `frontier` (edges with unbuilt parcels or a dead end), its population, and its demand per zone.
- Every growth tick picks one action, weighted by demand:
  1. **Outskirts.** Pick a new long, curving road away from the frontier, or a long driveway to an isolated house.
  2. **Infill.** Pick a short cul-de-sac or extension between existing roads where the parcel gaps are largest.
  3. **Frontage.** Place houses along a built edge in order of distance from the junction, so a street fills in.
- Built parcels and populations stay where they live now (`growth::Zones`). Only the choice of which edge or parcel
  comes next changes, so `RoadBuilder` keeps owning the build timing (LEAD_TICKS ghosts, budget).

## Slices, each with a headless test
- **(a) Curves and real junctions.** `ctrl` points from a seeded smooth noise. Test: every degree-3+ node is an actual
  crossing or branch, no two edges cross without a shared node, and the seed is deterministic.
- **(b) Sparse to dense.** Outskirts first with long driveways, then infill cul-de-sacs, with houses in frontage order.
  Test: house density near the core goes up over time, and frontage order is monotonic on each street.
- **(c) Traffic-driven roads.** Edges with sustained high volume over capacity are upgraded through the existing Widen
  path, and missing links between busy zones are added. Test: a forced overload leads to a widen or a new road within N ticks.
- **(d) Freeway corridors.** Long-range links between towns become freeway-class with interchanges (overpasses), not
  junctions. Test: a freeway edge never has an at-grade degree-3 junction.

Order: (a), then (b). If headroom is tight, stop after (b) and leave (c) and (d) as follow-ups.
