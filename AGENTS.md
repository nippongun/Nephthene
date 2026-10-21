# Nephthene: rules for coding agents

Weather library for Bevy 0.20 (Rust 2024). Layout: `crates/nephthene_core` (pure, no Bevy), `src/` (plugin, shaders in `src/shaders/`), `examples/sky.rs` (neutral scene + headless shot harness), `tests/` (gates), `tools/` (Python metrics).

## Run
- `cargo test --release` (everything automatic). Manual cloud gates: `cargo test --release --test clouds -- --ignored --nocapture --test-threads=1`. Goldens: `tests/golden/` (Apple Silicon); regenerate with `NEPHTHENE_UPDATE_GOLDEN=1`.
- Shot: `cargo run --release --example sky -- --shot <pose> --out <dir> [--set a.b=json]`; poses in `examples/poses.json`.

## Rules
1. **Headless only.** Never open a window. Shots and gates render to an image; the example refuses a window under `CLAUDECODE=1`.
2. **No magic values.** Every tunable lives in `Config` (`config.rs`), reaches shaders as a uniform, and defaults to a value labelled untuned. Do not tune look or feel values; add a knob, keep the old value as the default. A literal left in code or a shader needs a `ponytail:` comment saying why and the upgrade path.
3. **Rust uniform structs and WGSL structs must match field for field** (`tests` in `crates/nephthene_core/tests/layout_pins.rs` pin them). Change both sides together.
4. **Every gate needs a red arm** (a config switch that reproduces the bug). A gate that cannot fail is not a gate. Thresholds are budgets with a stated reason, never fitted just above a measurement.
5. **Verify with the gates; a fresh agent verifies, never the author.** Visual claims need the PNG read and described in pixels, plus the golden diff. Say "built, unverified" otherwise.
6. Shortest working diff. Stdlib and engine-native first. No speculative abstraction. Pure logic goes in `nephthene_core` with unit tests; the plugin stays a thin adapter.
7. No game-specific names, assets, bug IDs or feature IDs anywhere in this repo. Comments say the reason directly.
8. Bugs and decisions go in `docs/` (create `docs/bugs.md` and `docs/decisions.md` when the first entry exists); never delete a bug row, change its status.

## Roles (save tokens)
- **Planner** (main session, large model): research, design, decisions; writes the task graph. No mechanical edits itself.
- **Executors** (`.claude/agents/executor.md`, Haiku): one graph node each, only its files, run its check.
- **Verifier** (`.claude/agents/verifier.md`, Sonnet, fresh): re-runs checks and tries to refute; never edits (rule 5).
- Run a graph: workflow `graph` (`.claude/workflows/graph.js`), args `{goal, nodes: [{id, deps, files, task, check}], final_check}`. Nodes start when their deps pass; one retry on refutation. The planner commits after the final verdict.
- Node size: one file or one tight pair (Rust + WGSL struct), a check that runs in minutes. Write the task so an executor needs no other context.
- Cloud: `rust-toolchain.toml` pins rustc; the SessionStart hook installs Bevy's system packages and lavapipe (`VK_ICD_FILENAMES`), so shots and gates run on CPU (fog gates matched Metal within 1%; goldens stay Apple-only).
