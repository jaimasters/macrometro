# Multi-city world
- World = set of Cities (CityId) on one infinite chunk grid; each city owns a region (set of chunks) + its zone graph.
- Inter-city link = transit/road edge crossing region boundaries; aggregates to one super-edge (aggregation.md).
- Non-active cities simulate at coarse rate (1 tick per N active ticks) as super-zones; exports/imports flows only.
- "Move to new city" = change active city; old city keeps coarse sim. This is the normal reset (game-loop.md).
- Saves: one file per world; cities are sections (see features/save/SPEC.md).
