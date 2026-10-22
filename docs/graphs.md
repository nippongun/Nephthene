# Graph runs

Notes on the planner's task graphs (`.claude/workflows/graph.js`): what each node did, how each executor scored, and what the planner
learned. Newest first.

## Scoring

Each executor scores out of 10, from the verifier's report and the planner's read of its diff:

- **Correct (4):** the change does what the node says, and the final verifier found nothing wrong in it.
- **In scope (2):** it touched only its listed files and did only the task. A justified `blocked` counts as in scope.
- **Check (2):** it ran the node's check and reported the decisive output verbatim. A `blocked` report must be honest and specific.
- **Lean (2):** tokens and tool calls in proportion to the node (2 = in line with the others, 1 = about 2x, 0 = far over).

Tokens are input (with cache reads) / output.

## 2026-10-22: climate field (branch `claude/climate`)

Goal: a second RGBA8 map fixed to the world (temperature offset, humidity, ground snow, ground wetness) that localises the air config
per place; the air field restores to it per cell; plugin and docs. Research first (docs/climate.md sources). Three runs.

### Run 1 (`wf_7eb58039-e02`): failed on a planning error

| Node | Files | Time | Tools | Tokens in / out | Result | Score |
|---|---|---|---|---|---|---|
| N1-config | config.rs, core lib.rs | 1m24 | 12 | 262k / 26k | `ClimateCfg`, `Config.climate`, `WeatherMap::load_named`; 75 tests pass | 10 |
| N2-climate | climate.rs, core lib.rs | 0m40 | 6 | 272k / 12k | the file as specified; 4 new tests pass | 10 |
| N3-air-field | air.rs | 2m52 | 18 | 1138k / 32k | `Field` reads the climate; 3 gates pass with the predicted numbers (-25.000 K; rh 0.20 dry vs 0.56; snow range 0.375 of bare). Its check passed, but it reported `blocked` because the root crate no longer compiled. That was correct and honest, and outside its files. | 9 (lean 1) |
| N4-plugin | src/ | - | - | - | skipped: N3 blocked | - |
| N5-docs | docs, README, config.reference.json | 7m49 | 10 | 500k / 22k | docs correct. It could not run `--print-config` (the root crate was broken), so it wrote the `climate` section of `config.reference.json` by hand. The result later matched the generator byte for byte, but a generated file should not be hand-edited. It reported `blocked` honestly. | 8 (scope 1, lean 1) |

**Planner errors (mine):**
1. N3 changed a core API that the root crate calls, and nothing told it the breakage was expected until N4. Executors read "the
   root crate fails" as a block, so N4 was skipped. Fix for next time: a node that changes a cross-crate signature either includes
   its call sites, or its task says outright that the other crate breaks until node X and that this is not a block.
2. N5's check runs the example (`--print-config`), so it needs the root crate to build, but N5 depended only on N1. Fix: any node
   whose check builds the root crate depends on every node that changes an API the root crate uses.

### Run 2 (`wf_2a6feee1-c86`): pass

| Node | Files | Time | Tools | Tokens in / out | Result | Score |
|---|---|---|---|---|---|---|
| N4-plugin | src/lib.rs, sky.rs, fog.rs, panel.rs | 3m19 | 23 | 1188k / 63k | all edits as specified; both builds pass; the shot prints `CLIMATE offset +10.0 K humidity -1.00` and `AIR temp 38.1 C dew 14.4 C rh 0.25` | 9 (lean 1) |
| N5b-reference | config.reference.json | 0m11 | 3 | 62k / 2k | found N5's hand edit already identical to the generator; changed nothing | 10 |
| Final verifier | (read-only) | 20m58 | 16 | 818k / 12k | pass. It re-ran the core tests, builds, config diff and fog gates (G16 0.0835 vs red arm 0.0318, unchanged), and took two shots (neutral 28.1 C dew 20.4; hot and dry 38.1 C dew 14.4). It found 2 minor issues (the fog rebake key ignored the climate; a stale signature in docs/air.md), both fixed in run 3. | - |

### Run 3 (`wf_04e7f557-756`): pass (optimisation and the verifier's two items)

| Node | Files | Time | Tools | Tokens in / out | Result | Score |
|---|---|---|---|---|---|---|
| N6-cadence | config.rs, sky.rs, docs/air.md, config.reference.json | 1m36 | 18 | 448k / 13k | `air.step_h` (default one game minute): the live field steps over the gathered game hours, from the hour they began, and the gathered time resets when the clock freezes. Reference regenerated. | 10 |
| N7-fog-key | src/fog.rs | 1m08 | 8 | 172k / 4k | fog rebake key holds `ac.mean_c`, `dew_below_c`, `heat_k_per_h` (`Key` is `[f32; 10]`) | 10 |
| Final verifier | (read-only) | 2m31 | 5 | 131k / 6k | pass. Builds, core tests, config diff, a `--live` shot (AIR 28.2 C), and a full diff read. With only 5 tool calls, this was a lighter pass than run 2's. | - |

**Optimisation (planner, measured in a scratch bench, release build):** at `field_px` 32, one `Field::step` took 48 µs with the uniform
climate and 79 µs with a climate map; `spin_up` took 1.6-2.2 ms (only on a frozen clock when the hour or config changes);
`Field::state` 60-100 ns. Stepping every frame cost about 79 µs per frame. With `step_h` one game minute at the default clock
(15 game s per real s), a step falls about every 4 s, about 0.3 µs per frame on average. Not optimised: `spin_up` (rare), and the
per-cell `AirCfg` clone in `inputs` (it now runs once a game minute).

**Executor average:** 9.5 / 10 over 8 scored nodes. Every deviation from the spec was reported honestly; none edited a test or a
budget. The lean deductions went to the three nodes over about 500k input tokens (N3, N4, N5).
