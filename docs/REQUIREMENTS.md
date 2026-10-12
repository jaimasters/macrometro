# Requirements tracker

One row per request from Jai. Statuses: Requested, In progress, Merged (not seen on screen), Verified on screen, Blocked.
Rule: update a row in the same commit as the work that changes it. Managers own their rows; the macrometro orchestrator reconciles on merge.

| ID | Jai's request (short) | Acceptance check | Stage / owner | Status | Commit |
|---|---|---|---|---|---|
| R1 | Playable Mac app first; pause web | Release build launches on Mac and is playable | all | In progress | c63fd4e |
| R2 | Usable Mini Metro-style UI | Drag-to-build lines, HUD, tools usable | mgr-ui | Merged (not seen on screen) | c63fd4e |
| R3 | "look straight down" | Ortho top-down camera | mgr-ui | Merged (not seen on screen) | c63fd4e |
| R4 | Sound | Procedural soundscape, mute and volume | mgr-sound | Merged (not seen on screen) | 4884f81 |
| R5 | Hot reload | Mac runner rebuilds on new main commit and relaunches only after a good build | orchestrator | Verified on screen (runner log) | e9798c7 |
| R6 | Delete stations | Delete tool and right-click remove station; line gaps bridged | Stage A / mgr-ui | Merged (not seen on screen) | 5d06f0e |
| R7 | One-shot tools | Tool reverts after one use | Stage A / mgr-ui | Merged (not seen on screen) | 5d06f0e |
| R8 | No grid; organic layout, slower growth | Organic towns, growth gated by transit | Stage A2 / mgr-ui | Merged (not seen on screen) | 356326d |
| R9 | Station visuals, not shapes (platform bars) | Platform bars, crowding gauge, labels | Stage B / mgr-ui | Merged (not seen on screen) | bc1e7b0 |
| R10 | Real town and road patterns, visible traffic | Researched road growth (docs/arch/city-gen.md); zones need road access; car dots (zoomed in) and congestion tint (zoomed out) | A3 / mgr-city | Built, awaiting merge (not seen on screen) | see A3 report |
| R11 | Functional roads, mode choice | Car trips route on the generated road graph; logit mode choice on car vs transit minutes replaces stub CarShare/Scorecards | A3 / mgr-city | Built, awaiting merge (not seen on screen) | see A3 report |
| R12 | Station click menu | Menu: info (name, waiting, lines, platforms), extend line from terminus, new line, +platform, delete; palette tools stay as shortcuts | B2 / mgr-city | Merged (not seen on screen) | d2b122a |
| R13 | Zone legend ("what are green/orange/purple?") | Top-left legend chip: green residential, orange commercial, purple industrial | B2 / mgr-city | Merged (not seen on screen) | d2b122a |
| R14 | Track counts; one track is one-way | 1 one-way, 2 bidirectional, 4 with overtaking | C1+C2 / mgr-city | Merged (not seen on screen; C2 visuals pending) | 267fc82 |
| R15 | Platform counts with holding | Platforms gate trains into stations | C1+C2 / mgr-city | Merged (not seen on screen; C2 visuals pending) | 267fc82 |
| R16 | Kinematics | Accel, braking, headway spacing | C1+C2 / mgr-city | Merged (not seen on screen; C2 visuals pending) | 267fc82 |
| R17 | Curve radius, eminent domain | Arc curves, radius upgrade demolishes at cost | C1+C2 / mgr-city | Merged (not seen on screen; C2 visuals pending) | 267fc82 |
| R18 | Level crossings, bridges, underpasses cost money | Each crossing type priced | C1+C2 / mgr-city | Merged (not seen on screen; C2 visuals pending) | 267fc82 |
| R19 | "game is quite laggy" | Frame time before/after reported from live session | mgr-perf | In progress (logging merged) | 1bd7846 |
| R21 | Track placer curves around roads/buildings; mouse steers (long way round); leftover building hits = eminent domain, road hits = level crossing (upgradeable to bridge) | Routing avoids obstacle when clear space exists; steering flips the side; unavoidable collisions priced; preview shows route, demolitions, crossings, total cost | C1+C2 / mgr-city | Merged (not seen on screen; C2 visuals pending) | 267fc82 |
| R22 | Future roads flash in and out ("disjoint and silly") | Planned roads appear once and persist until built; no full respawn on plan change | orchestrator | Merged (not seen on screen) | 52103b6+ |
| R23 | Houses are nodes; should sit on the sides of roads | No building centre on a road node; every lot adjacent to a road segment; density = more/bigger lots | A3 / mgr-city | Merged (not seen on screen) | cce4717 |
| R24 | Road classes with lanes: street (1 shared lane, slow passing), 2-lane low, 2-lane high, 4-lane high, 6/8/10-lane freeway (wide corridor) | Each class sets sim capacity, speed and width; width scales with lanes, subtle lane markings, freeway median; upgrades demolish at eminent-domain cost | after C1 / mgr-city | In progress (core: lane classes, upgrade pricing via eminent_domain, traffic hook; UI, render, per-edge state and save pending) | |
| R29 | Car kinematics: cars keep lane, accelerate/brake, queue, never overlap | Car-following like trains over the routed road polyline | mgr-city | In progress (car-following + tests merged-ready; not seen on screen) | |
| R30 | Click a road or track segment (pan mode) for a menu; train load indicator | Road lane upgrade, track +track/curve/bridge; occupancy bar | mgr-city | In progress (road lane-class upgrade + save, segment menu, estimated load in menu; train body tint not done; not seen on screen) | |
| R35 | Track realism: sleepers and two rails per track; new lines single track both ways with passing loops; stations default to island platforms | Segment token on single track; 2 platforms default | mgr-city | In progress (sim rule, defaults, tests merged-ready; two rails + sleepers drawn on track, station sprites unchanged; not seen on screen) | |
| R33 | Crossing bug: a road built after track was laid gets no level crossing | Crossings re-found when built roads change | mgr-city | In progress (fix + test; not seen on screen) | |
| R34 | Left-hand driving checkbox next to Reset city, saved | Cars drive on the left, queue side flips | mgr-city | In progress (checkbox, lane side, save + test; not seen on screen) | |
| R32 | Real train occupancy: passengers board/alight in the sim, per-train load shown, waiting counts at stations | Replaces the estimate in R30 | mgr-city | Queued (after slice 3) | |
| R31 | Road geometry by class: min curve radius for fast roads; every at-grade junction is a roundabout by default (sized by class, entering cars give way, circulation follows left-hand setting); freeways grade-separated; track straight through stations | Planner and lane upgrades | mgr-city | Queued | |
| R25 | "why is it green" (congestion tint) | Free-flowing roads neutral; amber->red only above ~0.6 v/c; no green; congestion legend entry | orchestrator | Merged (not seen on screen) | this commit |
| R26 | "no visual of the amount of platforms nor where the trains are on them also no way of adding tracks to lines" | N parallel platform bars per station; dwelling train drawn on its platform; priced +Track action on a line segment, drawn as N rails; hint hidden after first pan, labels clear of toolbar | C1 first slice / mgr-city | Merged (not seen on screen) | cce4717 |
| R27 | "save state persist across sessions with a reset button" | Autosave (periodic + on exit) of city, roads, stations, lines, money; load on launch; survives runner relaunch; versioned format resets cleanly on change; in-game Reset with confirmation | after R26 / mgr-city | Merged (autosave verified live on Mac; Reset not seen) | 52776eb |
| R28 | "roads need actual journeys not just repeat animations" | Each car is a trip: origin home -> destination job/shop, routed over road graph, despawns/parks on arrival; trips from the same demand/mode-choice model | after R27 / mgr-city | Merged (not seen on screen) | 4fa4b7a |
| R20 | Requirements tracker with delivery status | This file kept current on every merge | orchestrator | Merged (not seen on screen) | this commit |
