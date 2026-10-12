# macrometro (agent-optimised repo)
Rust+Bevy 0.20, native Mac + wasm/WebGPU. Code written for agents; humans don't read it.
- Route: read docs/INDEX.md first, then only the docs for your feature.
- One crate per feature in crates/; pub API = lib.rs; never edit another manager's crate.
- Every feature: docs/features/<f>/{SPEC,DECISIONS,STATUS}.md; acceptance = `cargo test -p <f>` tests named in SPEC.
- Workflow: /plan-feature -> /build-feature -> /verify -> /ship (.claude/commands).
- Hierarchy, ownership, spawn rules: docs/AGENTS.md (binding). Always pass `model` when spawning (frontmatter ignored).
- Not Norby: no Linear, no Norby Memory. Keep on /mnt/ssd. Full builds on Mac.
- Determinism: seeded RNG only; tests assert identical hashes.
