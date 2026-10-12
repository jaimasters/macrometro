# Agent hierarchy (Jai, 2026-10-12)
Coordinator (orchestrator, domain macrometro) -> Managers (one per area) -> Workers.
| Manager | Owns crates | Worktree/branch |
|---|---|---|
| mgr-sim | sim_core, growth | /mnt/ssd/work/macrometro-wt/sim  mgr-sim |
| mgr-render | render_budget | /mnt/ssd/work/macrometro-wt/render  mgr-render |
| mgr-world | world_gen, save | /mnt/ssd/work/macrometro-wt/world  mgr-world |
| mgr-transit | transit, economy | /mnt/ssd/work/macrometro-wt/transit  mgr-transit |
| mgr-ui | ui_tools, app | /mnt/ssd/work/macrometro-wt/ui  mgr-ui |
Rules:
- Manager: Agent subagent_type `orchestrator`, model "sonnet", effort "high". Edits only its crates + its docs/features/*.
- Workers: subagent_type `low-worker-claude`, model "haiku" ALWAYS. One function/system/test target; exact paths; acceptance test; stopping condition.
- Haiku model_not_found: retry once; if fails again report to coordinator and wait. No silent model switch.
- No concurrency cap; stagger spawns; back off on 429/burn warning.
- Never two agents in one worktree. Cross-crate API change: request via coordinator.
- Merge: manager rebases on main, `cargo check --workspace` + `cargo test -p <crates>` green, then coordinator merges.
- Heavy builds on Mac (ssh 100.104.143.66, no `timeout`); local `cargo check`/`cargo test` of small crates OK.
Efficiency (usage audit 2026-10-13):
- Context ceiling: any agent past ~150k context writes a 2-4 KB handoff and is replaced. mgr-city hit 465k, mgr-ui 364k; cache reads 53M/32M tokens = most of the session's spend.
- Hops: coordinator briefs managers once; no manager for a single-feature task (spawn one high worker directly, as R31 did). Managers do not relay status back up unless done or blocked.
- Edit with Edit tool or one script per file; never re-run 10+ inline python heredoc patches on the same file (mgr-city: 16x same app/src heredoc).
- Merge from a manager branch once per milestone, not per fix (mgr-city fetched+merged 11x).
- Behaviour gate: a feature Jai can see (roads, cars, stations) needs one headless sim test of the visible behaviour before merge, e.g. cars complete a lap of a roundabout, no car despawns before arrival, station footprint fits its platforms. R28/R31 merged with unit tests only; Jai then reported "roundabouts not working / wrong size" and "cars still randomly despawn" (22:56-22:57).
- Visual claims: say "built + tested, not visually checked" unless a screenshot was taken. Never report a visual feature as working from tests alone.
- Mac optional (Jai 2026-10-13): before each build/test run `ssh -o ConnectTimeout=5 100.104.143.66 true`; if it answers build/test there (faster), else `nice -n 10 cargo test --workspace` locally under /mnt/ssd/work/macrometro. A local pass is enough to merge; push to main is delivery (the runner pulls when the Mac is online). Never wait on, poll, or block on the Mac.
- Briefs quote Jai's words and the observable acceptance he will check. R28 brief tested "a car arrives" only; Jai then reported overlap/no kinematics (fix queued 22:07) and random despawns. Acceptance must cover the failure Jai would see, e.g. no overlaps, no mid-trip despawn.
- Main merges relaunch the Mac game (runner: pull ~1 min + build ~2-6 min per commit; 12 relaunches 09:17-10:08). Batch related slices into one main merge; don't merge main back into a manager branch after every main commit.
- Model 404: 14 real model_not_found errors this session (7 in main chat, 2 coordinator, 5 managers/workers). After one retry, report and wait; a respawn on the same model is not a retry.
