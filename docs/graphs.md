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

## 2026-10-23: cloud edge LOD (branch `claude/cloud-edge-lod`)

Goal: a toggle that reduces cloud march steps and detail erosion by screen radius, off by default. Two runs.

| Node | Files | Result |
|---|---|---|
| N1-config | config.rs | `EdgeLod` and its unit test |
| N2-uniform | render.rs, clouds_common.wesl, clouds.rs | six fields at the uniform tail, filled every frame; layout pins pass |
| N3-march | clouds_march.wesl | correct on run 1, but blocked by the planner's check: `grep error` matched the URL in Bevy's software-renderer WARN. Run 2 filtered that line out |
| N4-panel | panel.rs | the "Cloud edge LOD" toggle |
| N5-gate | tests/edge_lod.rs | G33 green on lavapipe: centre 0.000, edge 2.490, red arm 26.223 |
| N6-cost | tests/cost.rs | `cost_clouds_edge_lod` (Mac) |
| N7-reference | config.reference.json, README.md | regenerated; README rows |

Verifier: pass. The default render is byte-identical to main's (built from main, same shot). Notes: the edge half's red arm is
only the off pair; at the defaults the edges turn grainy rather than soft (the owner's look call).
Learned: a check that greps logs for "error" must exclude Bevy's software-renderer WARN.

## 2026-10-22: orographic precipitation, slice 2 (branch `claude/orographic-clouds`)

Goal: the cloud march, shadow and shafts read the orographic tile (a second texture, combined in `map_sample`), with a render gate.
Four runs. The GPU path was right from run 1. Runs 2-4 were spent on what the render gate exposed, and each step taught something
real.

### Run 1 (`wf_13d71757-59e`): GPU path done; the gate went red over the whole sky

| Node | Files | Time | Tools | Tokens in / out | Result | Score |
|---|---|---|---|---|---|---|
| S1-bytes | orographic.rs | 0m26 | 5 | 126k / 4k | `Tile::rgba8` and its test | 10 |
| S2-gpu | 4 shaders, render.rs, clouds.rs, src/orographic.rs | 5m28 | 32 | 2640k / 56k | bindings, uniform fields, `map_sample` combine, upload on a new tile; the layout pins pass, and both shots compile their shaders at run time with no error | 9 (lean 1) |
| S3-gate | tests/orographic.rs, poses.json | 4m46 | 14 | 515k / 15k | an honest red (windward 11.8 vs lee 28.7) with a correct frame description: the whole sky went overcast | 10 |

**What it exposed (a model flaw):** Smith & Barstad assume saturated air. In the afternoon air, about 9 K above its dew point, every
45 m hill made cloud. The fix: lift the air from the tile's lowest ground, and let it condense only above its lifting condensation
level (Espy, 125 m per K), with the terrain clipped there (`docs/decisions.md`).

### Run 2 (`wf_fb1b24fa-d21`): the LCL clip passes its gate; the render scene is now cloud-free

| Node | Files | Time | Tools | Tokens in / out | Result | Score |
|---|---|---|---|---|---|---|
| R1-core | config.rs, orographic.rs | 1m10 | 18 | 803k / 34k | the LCL clip, `dew_c` on the tile, gate `cloud_needs_the_lifting_condensation_level` (dry 0.0000, saturated 0.277) | 10 |
| R2-plugin-gate | src/orographic.rs, tests/orographic.rs | 5m01 | 15 | 511k / 12k | the air at the reference altitude passed in, the gate renamed to G31 (G28-G30 belong to another session); an honest red: at 14 h the air is 10 K above its dew point, so the ridge stays under its LCL (max cover 0.02) | 10 |

### Run 3 (`wf_1df5688d-758`): a cap appears, but the frame-wide mean cannot see it

| Node | Files | Time | Tools | Tokens in / out | Result | Score |
|---|---|---|---|---|---|---|
| T1-gate | tests/orographic.rs, poses.json | 5m53 | 13 | 384k / 14k | at 10 h (5 K spread) a denser band appears toward the ridge, but the mean is 2.14 against the 5 borrowed from G22, and the downwind view changed too (1.66); an honest red with a pixel description | 10 |

**What the planner then measured (scratch bake and renders):** the downwind change is real physics, not an artifact. Linear theory
lifts the air in lee waves about 2 pi U / N_m (12.6 km) downwind, and the tile has cover 0.1 at x 14-17 km. (Subtracting the minimum
instead of the mean in `compute`, tried as a fix, did not remove it and broke the slice-1 gate.) A camera at x 7 km sits between
the cap and the wave. The metric became the share of pixels changed by 20+ levels: west 6.5 %, east 0.72 %.

### Run 4 (`wf_2ca22ebc-ae8`): G31 passes; the verifier refuted two doc sentences

| Node | Files | Time | Tools | Tokens in / out | Result | Score |
|---|---|---|---|---|---|---|
| U1-gate | tests/orographic.rs | 5m08 | 8 | 223k / 11k | G31: west 6.52 % (min 1 %), east 0.72 % (west >= 1.5x east), red arm vs off 0.000 (budget 0.500) | 10 |
| U2-docs | docs, handover, config reference | 5m31 | 15 | 583k / 60k | wrote the planner's text as specified | 10 |
| Final verifier | (read-only) | 9m47 | 23 | 1513k / 23k | refuted 2 wordings of the planner's against its own CPU bake: the lee wave sits at x 14-17 km, which is 11-14 km past the ridge; at 14 h there is no cap, but a faint wave remains (0.05). Fixed by the planner (docs only). | - |

**Planner lessons:** (1) Before writing a render gate, bake the scene's tile on the CPU and render one on/red pair in scratch. Run 4
did this and passed first time; runs 1-3 each spent a run discovering the scene. (2) Do not borrow a budget from a gate whose
change covers the whole frame (G22) for a change that covers part of it; measure the visible area instead. (3) Docs sentences with
numbers come from a measurement, not from the shape of the result.

**Executor average:** 9.9 / 10 over 8 scored nodes. Every red was reported honestly, with the numbers and a frame description,
and no node touched a budget.

## 2026-10-22: orographic precipitation, slice 1 (branch `claude/orographic`)

Goal: bake Smith & Barstad (2004) over `Ground` around the camera into a tile fixed to the terrain and fold it into `Weather.here`
(CPU only, off by default). Research first: the paper's parameters, the AMS rain-intensity classes, and a scratch run of `compute`
for magnitudes. Three runs.

### Run 1 (`wf_6891d4d2-a4a`): every node passed; the verifier refuted the gate

| Node | Files | Time | Tools | Tokens in / out | Result | Score |
|---|---|---|---|---|---|---|
| N1-config | config.rs, core lib.rs | 0m42 | 10 | 225k / 12k | `OroCfg`, `Config.orographic` | 10 |
| N2-core | orographic.rs | 1m02 | 9 | 417k / 21k | Cw, `Tile`, `bake`, `sample`, `stale`, `combine` and the tests as specified. Cw(10 C) 0.00759. It noted unprompted that the lee read 0.000, so the 1.5x clause passed trivially, which was the first sign of the gate's flaw. | 10 |
| N3-plugin | src/orographic.rs, lib.rs, sky.rs, panel.rs | 4m29 | 11 | 626k / 23k | all as specified; the shot prints `OROGRAPHIC tile (-1500, 600) wind (4.2, 4.2) m/s air 27.0 C: max precip 0.57 cover 1.00` | 10 |
| N4-docs | docs, README, config.reference.json | 0m28 | 9 | 233k / 16k | docs as specified; the reference was regenerated and matched | 10 |
| Final verifier | (read-only) | 6m31 | 15 | 798k / 16k | **refuted**. It flipped the wind mapping in `bake` in a scratch copy, and the gate still passed. | - |

**Planner error (mine):** the gate `windward_slope_rains_lee_stays_drier` sampled 1.5 km before and 6 km past the crest of a
symmetric ridge. With the paper's 1000 s fallout delays, the rain peaks just past the crest, so a reversed wind still wets the
"windward" point. The gate could not tell the wind's direction. The fix: test properties that depend on direction. The cap cloud forms
on the ascent (cover 3 km upwind of the crest above the cover 3 km downwind), and the rain drifts downwind (4.5 km downwind at least
1.5x 4.5 km upwind, 3.1x measured). Lesson: when a gate claims to pin a sign or an axis, the planner mutates that sign in a scratch
copy before handing the gate out.

### Run 2 (`wf_5d5bb5bb-8eb`): the replacement gate passed, but the node reported `blocked`

| Node | Files | Time | Tools | Tokens in / out | Result | Score |
|---|---|---|---|---|---|---|
| G1-gate | orographic.rs | 1m29 | 9 | 339k / 13k | `rain_condenses_upwind_and_drifts_downwind` passes (0.187 upwind under cover 1.0; 0.156 vs 0.050 drift). It ran both axis mutations in a scratch copy, and both fail. It then reported `blocked` because the docs still named the old test, which was the next node's job, so G2 was skipped. | 8 (check 0: `blocked` with its check passing) |

**Planner error (mine, the second time):** the same failure mode as the climate run's N3. When the task leaves a mismatch for a
later node, the task must say so ("the docs are G2's job; this is not a block").

### Run 3 (`wf_294524d9-50c`): pass

| Node | Files | Time | Tools | Tokens in / out | Result | Score |
|---|---|---|---|---|---|---|
| G2-docs | docs/orographic.md, roadmap.md | 0m13 | 9 | 157k / 10k | the gate section, the research row and the limits updated; the old name gone | 10 |
| Final verifier | (read-only) | 5m27 | 11 | 491k / 11k | pass. It redid both axis mutations itself (both fail), checked that the red arm fails, ran the shots (one OROGRAPHIC line when enabled, none by default), and probed the profile independently (1.42 / 3.40 / 2.70 mm/h at -3 km / crest / +3 km; the downwind half 1.49x the upwind half). | - |

**Executor average:** 9.7 / 10 over 6 scored nodes. The two defects were in the planner's specs, not the executors' work: a gate that
could not fail on the property it claimed, and a node not told that a later node would fix the docs.

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

## 2026-10-22: orographic upwind lifting base (branch `claude/affectionate-pasteur-qtqvng`)

Goal: lift each cell's air from the lowest ground upwind of it (`orographic.fetch_m`) instead of the tile's lowest ground; gate
`cloud_lifts_from_the_upwind_ground`, red arm `orographic.tile_base`; no tile cache (measured). The planner prototyped the bake and the
gate in a scratch copy first (tile base 0.34 cover, upwind 0, flipped march 0.34; a 10 km fetch changed the old gate's lee rain, 25.6 km
did not). One run.

### Run 1 (`wf_5611d590-6fc`): pass

| Node | Files | Time | Tools | Tokens in / out | Result | Score |
|---|---|---|---|---|---|---|
| N1-core | config.rs, orographic.rs | - | - | - | as specified; both axes dry 0.0000, red arm 0.3433, humid 0.506; old gates unchanged; the flipped march fails in a scratch copy | 10 |
| N2-docs | config.reference.json, 5 docs | - | - | - | reference regenerated (two keys); G31 on lavapipe passes (west 5.91 %, east 0.50 %, red arm 0.000, 288 s); docs filled from measurements and flagged which numbers it did not re-measure | 10 |
| Final verifier | (read-only) | - | - | - | pass. Full `cargo test --release`; flipped x only, z only, both, and a disabled march (all fail); `--print-config` byte-identical; bake 44 ms vs 12 ms; G31 re-run, same numbers. Note: the bake time is measured on a `HeightGrid` only. | - |

Workflow total: 3 agents, 52 tool calls, 114k subagent tokens, 16.5 min.

## 2026-10-23: mountain test terrain (branch `claude/mountains`)

Goal: a mountain range fixture (`terrain::mountains`), `--mountains`, poses `mtn_*`, `--pose` for the live camera, docs. The planner
prototyped the heightmap (numpy hillshade, then the Rust function in a scratch crate) and handed it over as a file to copy.

| Run | Node | Result | Score |
|---|---|---|---|
| `wf_09505f39-8e1` | N1-core | copied, test passes (top 2860 m, crest band 1462 m, plains 60 m) | 10 |
| | N2-example | all edits right; reported `blocked` on a build warning that is already on main (`panel_test`, read only under the panel feature) | 9 (correct to report; the planner's check was too strict) |
| `wf_927f6095-77d` | N3-docs | README line, handover row | 10 |
| | Final verifier | pass, no shots: core tests, warning on HEAD, exit codes 0/4/5, spawn diff, pose heights 29/45/73 m under 250/250/4500, no game names | - |

**Planner error:** a "no warnings" check without first building HEAD for warnings already there. Build the base first.

## 2026-10-23: horizon speckle and clouds meet terrain (collected into `claude/mountains`, PR #24)

| Run | Node | Result | Score |
|---|---|---|---|
| `wf_d7feaa8b-535` (on main, moved to #24) | N1-config, N2-gpu, N3-docs | `air_segment` guard, `clouds.air_guard`; both settings compile; reference regenerated | 10, 10, 10 |
| | Final verifier | pass; argued the lower-bound maths per branch; noted the unused `sample_transmittance_lut_segment` imports | - |
| `wf_96d80c9c-645` | T1-config, T2-rust, T3-shaders, T4-plane, T5-docs | all as specified, each check passed | 10 each |
| | Final verifier | **refuted**: `layout_pins::march_flash_mirrors_rust` pins the composite's `return vec4(c.rgb + f.rgb, c.a);`, which T3's rewrite (planner-written code) dropped | - |
| fix + fresh verifier | planner: `with_flash` helper keeps the pinned form | pass: full `cargo test`, pins unedited, five compile shots, binding orders, config | - |

**Planner errors:** (1) the composite rewrite I specified broke a pin I had not read; read the pins on every file a node rewrites. (2) T2's check ran layout_pins before T3 changed the shader, so no node ran the full suite after the last edit; give the last code node `cargo test --release` as its check.
