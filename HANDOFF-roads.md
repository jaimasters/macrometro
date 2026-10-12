# HANDOFF — batch 2 (mgr-roads, /mnt/ssd/work/macrometro-wt/roads)
Done + pushed (workspace tests green on Mac before the main pull; rerun after):
- R43: reset dialog has Hamlet / Small town / Large town; choice stored as "preset seed" in save.preset (localStorage macrometro.preset on web); layout::plan_layout uses city::organic::plan_preset at the first city's core. Seed steps per reset. Test: persist reset_preset_marker_parses.
- organic.rs warnings fixed. Asphalt ramp roads::ASPHALT (edges + ring discs); test asphalt_darkens_strictly_by_class_and_markings_stay_readable. Interstate shade reserved (no such class yet).
Not visually checked: all of the above; far-zoom stroke width (queue item) NOT checked.
Also done: left-hand roundabouts (445d4f3), turn speed planning (c9d6987), preset junction spacing + ring-clear test (organic::junction_gap).
Not visually checked: all of the above.
Also done: ring discs thin opaque slabs (4f16003); junctions::merge_close (clearance discs never overlap / never swallow a road end) run in BOTH fresh-city paths (roadgen::gen_town merge->smooth->merge, organic::plan_preset), tests assert_clear over presets + plan_towns seeds.
NEXT: queue row "Unify city generation" (Jai rule: seed + same growth step N times, presets = seed size + N; absorbs runtime-stub row) is the main next item. Then remaining B2 rows: LANE_W sizing, one-lane tier, far-zoom stroke check. Report to main chat (orchestrator out of context).
