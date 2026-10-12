# HANDOFF — batch 2 (mgr-roads, /mnt/ssd/work/macrometro-wt/roads)
Done + pushed (workspace tests green on Mac before the main pull; rerun after):
- R43: reset dialog has Hamlet / Small town / Large town; choice stored as "preset seed" in save.preset (localStorage macrometro.preset on web); layout::plan_layout uses city::organic::plan_preset at the first city's core. Seed steps per reset. Test: persist reset_preset_marker_parses.
- organic.rs warnings fixed. Asphalt ramp roads::ASPHALT (edges + ring discs); test asphalt_darkens_strictly_by_class_and_markings_stay_readable. Interstate shade reserved (no such class yet).
Not visually checked: all of the above; far-zoom stroke width (queue item) NOT checked.
NEXT (priority order):
1. Queue 12 left-hand roundabouts (Jai live report on 1d9be02): rings clockwise when Hand.0 (left); entering cars yield to ring traffic from their right; entry/exit in left lane. Look at junctions::Ring ring_trip splice + lane_pt(left) in roads.rs; test: clockwise ring, entry/exit left of centreline, entrant yields to approaching ring car, plain junctions/straights keep left.
2. Road spacing item (min junction spacing per class, min length, merge close junctions, ring-clears-neighbours rule, spacing/overlap test) — roadgen.rs/organic.rs + junctions::Ring::at.
3. Disc as one opaque layer (roads.rs ring spawn) + far-zoom stroke check.
