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
