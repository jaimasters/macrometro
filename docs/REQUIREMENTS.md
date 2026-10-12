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
| R10 | Real town and road patterns, visible traffic | Researched road growth; zones need road access; car dots and congestion tint | A3 / mgr-city | In progress | |
| R11 | Functional roads, mode choice | Car trips route on roads; mode choice by travel time replaces stub car share | A3 / mgr-city | In progress | |
| R12 | Station click menu | Menu: info, extend, new line, +platform, delete | B2 / mgr-city | Requested | |
| R13 | Zone legend ("what are green/orange/purple?") | Legend: green residential, orange commercial, purple industrial | B2 / mgr-city | Requested | |
| R14 | Track counts; one track is one-way | 1 one-way, 2 bidirectional, 4 with overtaking | C1+C2 / mgr-city | Requested | |
| R15 | Platform counts with holding | Platforms gate trains into stations | C1+C2 / mgr-city | Requested | |
| R16 | Kinematics | Accel, braking, headway spacing | C1+C2 / mgr-city | Requested | |
| R17 | Curve radius, eminent domain | Arc curves, radius upgrade demolishes at cost | C1+C2 / mgr-city | Requested | |
| R18 | Level crossings, bridges, underpasses cost money | Each crossing type priced | C1+C2 / mgr-city | Requested | |
| R19 | "game is quite laggy" | Frame time before/after reported from live session | mgr-perf | In progress (logging merged) | 1bd7846 |
| R21 | Track placer curves around roads/buildings; mouse steers (long way round); leftover building hits = eminent domain, road hits = level crossing (upgradeable to bridge) | Routing avoids obstacle when clear space exists; steering flips the side; unavoidable collisions priced; preview shows route, demolitions, crossings, total cost | C1+C2 / mgr-city | Requested | |
| R20 | Requirements tracker with delivery status | This file kept current on every merge | orchestrator | Merged (not seen on screen) | this commit |
