# Nephthene roadmap

Nephthene is a complete, physically grounded, art-directable weather system for Bevy: sky, clouds, fog, rain, snow, dust, wind,
storms (thunder, snow, sand), lightning and weather sound cues, all driven by one weather map and one clock, every knob in `Config`. A host game only consumes it.

## Rules (all phases)
- **Separate.** Nothing host-specific in the repo (AGENTS.md rule 7: no game names, assets or IDs). A host hooks in through
  markers (`NephtheneSun`, `WetGround`), resources (`Ground`, `Weather`, `FogState`, ...) and config only.
- **No magic values.** Every tunable is a `Config` field with a documented default, labelled untuned until the owner tunes it.
- **Proof.** Every feature ships a gate that is red without it and green with it, and keeps its red arm (a config switch). A
  gate whose red arm cannot fail on any renderer is deleted and logged, not kept. Bounds come from a stated budget, never from a
  measurement.
- **Reference GPU: Apple Silicon / Metal.** Cloud sessions render on lavapipe (CPU Vulkan). lavapipe matches Metal within ~1% on
  image gates and is a good *relative* cost proxy (A/B wall time), but says nothing about absolute GPU cost or about texture vs
  ALU trade-offs. Look and cost are accepted only after a Metal run; the owner's Mac reviews every push.
- **Budget.** The whole stack fits 3 ms at 1080p on the reference GPU (owner's call), with a per-pass cost gate. Proposed split
  (owner's call): clouds march+resolve 1.2, cloud shadow 0.2, rain shafts 0.3, ground fog 0.8, rain FX 0.3, slack 0.2 ms.
  Retina (~2.2x the pixels) is met through each pass's `render_scale`, not a second budget.
- **Look first, by eye.** Green gates do not prove a look. Every visual change ships renders of the fixed poses
  (`examples/poses.json`); the owner judges them. Look values (exposure, amplitudes, colours) are the owner's call.
- **Determinism.** Systems that share state are ordered explicitly: an unordered pair in the shot harness moved the golden sway
  frames when unrelated systems were added.
- **Log.** Decisions in `docs/decisions.md`, bugs in `docs/bugs.md` (append rows, change status, never delete).
- **Roles** (AGENTS.md): a planner session designs and writes task graphs; Haiku executors do the nodes; a fresh verifier checks
  each one. One owner per file across parallel sessions (see Ownership).

## Phase 0: finish ground fog (branch `claude/affectionate-pasteur-qtqvng`) — merged (PR 1); tail below
Done when: Metal runs green (G15-G19, goldens), the owner accepts the renders, then merge to main.
- Cost: half-resolution march + depth-aware composite, baked noise texture; gate G19 <= 0.8 ms at 1080p (proposed; the
  branch currently gates 1.5 ms) with red arm `render_scale=1` + `hash_noise=true`. Metal number pending.
- Visible without forcing: fog follows the effective mean wind (panel override included); document that the default 6 m/s wind
  forms none and that a calm night does (>= 0.7 by 5 h at <= 2.5 m/s). Whether the *default* weather should ever be calm is a
  Phase 2 weather-state question.
- Sunlit pool reads as snow at 8 h: owner decides exposure (scene runs ~2 stops over sunny-16) and billow amplitude; then a
  surface-texture gate with billows off as red arm.
- Horizon seam inside fog at `fog_floor` 8 h: from the burn-off lift (a clear gap under the pool the camera stands in), not the terrain; owner's look call on the lift near amount 1 (docs/bugs.md).
- Temporal resolve of the fog march (G21) done on lavapipe; Metal still and turning renders pending.

## Phase 1: cost and coverage gates
Done when: every pass has a Metal cost gate at 1080p, the dropped checks are back with red arms, the golden offset is fixed.
- Per-pass GPU timing gate (cloud shadow, march, resolve, shaft march/resolve, fog, rain) at 1080p and Retina on Metal, built on
  `clouds.pass_timing` (`PASSMS`) and `FOGMS`; skip on software renderers. Add rain to the timing (it has none today).
- Re-add the checks the extraction dropped: sun disc and MSAA hairline (old G1-G4), shafts do not lean with gusts (add a
  gust-lean red-arm switch), the panel pointer self-test (`--panel-test` is parsed but unused), rain and storm shot scenes.
- Fix the golden pack's fixed 0.04-0.21 offset by strengthening the pack (more frames, the reference renderer), not by widening
  the bound.
- Audit system ordering (resources written by one system and read by another in the same set) and order them.
- Public panel root so a host can add widgets, font and camera binding (after Phase 0 merges: `src/panel.rs` is Phase 0's).
- Quality tiers: `Config` presets (low / medium / high) that set each pass's `render_scale`, steps and update div, so every
  later feature lands with a cost per tier, not just one number. The auto scaler stays in Later.

## Phase 2: weather state (the foundation the storm kinds need)
Done when: rain vs snow, fog formation and dust lifting all read one air state, and the weather map moves with the wind.
- **Air state in core** (pure, unit-tested): surface temperature from clock, season, latitude and cloud cover; lapse rate with
  altitude (freezing level, snow line); humidity / dew point. Today nothing knows a temperature: fog formation has its own night
  cooling, and snow has no trigger. Fog formation moves onto the shared air state (red arm: its old private cooling).
- Precipitation type from the air state: rain, snow, sleet, hail (`precip_kind` 0..1 blends, not a switch, so a front can turn
  rain to snow over minutes). Exposed on `Weather` for the host.
- **Weather map advects with the wind**. Step 1 done: under today's one global wind the map is translated, not resampled
  (`map.advect`, off by default; `map.wrap`; `map.start_shift_m` for save/restore; gates `map_tests` and G22; `docs/weather_map.md`).
  Step 2, with the wind field below: a non-uniform wind cannot be a translation, so the map becomes a ping-pong compute or fragment
  pass on the GPU (semi-Lagrangian), with a CPU copy for `Weather.here`. Owner: the default `advect`, and a tileable map before wrap
  goes on. The map's RGBA8 is full (coverage, precip, type, dust): a second map holds temperature offset, humidity, ground snow and
  ground wetness.
- **Wind field**: replace the one global vector (`lib.rs` ponytail) with a coarse 2D field (mean + gusts + storm outflow), sampled
  by clouds, rain angle, fog drift, dust lift and audio, and readable by the host (foliage, cloth, gameplay).
- Weather states over time (calm clear nights, fronts, showers, cold snaps, dry windy spells) on the seeded clock, plus a host
  API to request one ("blizzard in 120 s") with a transition time. Dawn fog then appears without forcing.
- Determinism: same seed + same clock = same weather on every machine (multiplayer, replays). Save / restore of the state.

## Phase 3: storm cells as 2D fields (published approach: Schneider, SIGGRAPH 2022 course on 2D cloud fields)
Done when: storm cells come from map fields, lifecycle runs on the clock, and the renders read as cumulonimbus, not capsules.
- Storm cells are fields painted into the advecting map, not analytic capsules (those read as mushrooms). Lifts the 16-cell
  uniform cap (`storm::MAX_CELLS`), which a squall line or multicell system needs.
- Cumulonimbus: towers to the tropopause (default 14-16 km), anvils that spread and thin downwind, overshooting tops, a shelf
  cloud on the gust front. Cumulus congestus grows from smooth 2D height and strength fields, not squashed noise: no slices, no
  streaks (was Phase 3).
- Dark bases: ambient occluded by the column above; ground bounce shadowed by the storm.
- Lifecycle on the clock (developing, mature, dissipating), squall lines and multicell systems; seeded timetable plus authored storms.
- Art-directed horizon: some distant drama always, whatever the local map says. Sky scale real by default, cloud base per region.

## Phase 4: precipitation and surfaces
Done when: rain, snow and sleet fall from the map's precip and the air state's kind, stop under roofs, and leave the ground wet,
white or dusty; each has a red-armed gate.
- **One particle pass, several kinds.** Keep the stateless vertex-shader scheme (`rain.wesl`, mirrored in `core::rain`): a quad's
  kind picks its motion and look. Rain: streaks. Snow: slow flakes with flutter and wind drift, sprites not streaks. Sleet / hail:
  short bright pellets that bounce. Sand: grains near the ground (Phase 5). No second particle system.
- **Precipitation occlusion map** (replaces the `rain.rs` ponytail "rain also falls indoors"): an orthographic depth render of the
  scene from above around the camera, refreshed when the camera moves a fraction of its extent. Streaks, flakes and splashes stop
  on roofs and overhangs; wetness and snow cover read it too. Red arm: occlusion off.
- Rain curtains and shafts under cells, virga with high bases, rain intensity from the map.
- **Weather surface material**: an `ExtendedMaterial<StandardMaterial, _>` the host opts into, replacing the per-entity
  `WetGround` colour edits: wet darkening, puddles from a noise and slope mask, ripples, snow cover by slope and accumulated depth,
  dust film. Accumulation and melt per texel in a ground-state map (Phase 2's second map), on the GPU.
- Rainbows: analytic, sun behind the camera, rain ahead, sun elevation < 42 deg; nearly free.
- Optional: drops and frost on the camera lens (a host post-process toggle).

## Phase 5: wind, dust and blowing snow
Done when: a gust front lifts dust where the ground is dry and dusty, a haboob wall approaches and engulfs the camera, and
blowing snow reduces visibility near the ground; gates on wall visibility and on dust only where the ground is dry.
- Gusts and gust fronts from storm outflow (`storm_fx::outflow_at` exists in core, unwired) into the Phase 2 wind field.
- **Low-atmosphere media in the fog march.** Dust and blowing snow are media in the existing ground-fog march (one depth stop, one
  temporal resolve, one composite), not new passes: per medium a density from a map channel and an analytic height profile, its
  own albedo, phase and coloured extinction (dust warm and absorbing, snow white and forward scattering). Proposed; owner's call
  (decisions.md) against a separate pass.
- Haboob: a dust wall 1-3 km high riding the gust front at 10-25 m/s, sun dimmed and reddened, the sky's Mie dust term (map A)
  raised. Sand grains near the ground in the Phase 4 particle pass.
- Heat haze and mirage over hot ground (a screen-space refraction toggle), the dry-weather companion of dust.

## Phase 6: lightning and thunder
Can start after Phase 1, in parallel with Phases 2-5: the flash schedule (`storm_fx::flashes`) and the analytic cells exist.
Slice 1 built (lavapipe; Metal pending): bolt geometry in core (`core::lightning`, unit tests), channel ribbons, one point light per
live flash, `LightningStrike` / `Thunder` messages, gates G23 (channel reads as a line) and G24 (flash lights the ground). Off by
default (`storms.lightning.enabled`). Left for slice 2 (touches `clouds_march.wesl`, ask first): clouds lit from inside, the
resolve-vs-flash term, in-cloud flashes, bloom, fog lit by the flash.
Done when: cloud-to-ground and in-cloud flashes light the clouds, terrain and fog, thunder arrives late by distance, and the host
receives events; gates below.
- **Bolt geometry in core**: seeded stepped leader with branches (midpoint displacement), deterministic from the `Flash`; pure,
  unit-tested (connectivity, ends on the ground for CG, stays inside the cell for IC).
- **Bolt render**: emissive ribbons from a segment buffer in a vertex shader (the rain scheme), HDR so Bevy's bloom makes the glow;
  3-5 return strokes over ~0.5 s, channel afterglow.
- **Clouds lit from inside**: one point-light term in the cloud march from the flash position with a cheap transmittance
  (distance and local density, no light march), evaluated only while a flash is live.
- **Temporal resolves vs flashes**: the cloud and fog resolves blend history at 0.1 per frame, so a one-frame flash would smear over
  ~10 frames and its rise would be clamped. Flash light needs its own unresolved term (or the history re-weighted on flash frames).
  Gate: luma at flash frame N, and back within budget of the pre-flash level by N+k.
- **Scene light**: a short-lived Bevy light at the strike (no shadows by default) plus an ambient bump; fog lit by the flash.
- **Thunder**: `thunder_delay_s` by distance (exists); host events `LightningStrike { pos, cg, energy }` and
  `Thunder { delay_s, distance_m, energy }`. The wind / rain audio mapping (`core::audio`) becomes host events in the same change.
- Thundersnow (rare flashes in strong snow storms) falls out of the same code.

## Phase 7: day and night
- Night sky (moon with inclination, stars, airglow), twilight colours, clouds and fog lit by the moon with terrain shadow.
- Lighting under weather: overcast and storm darkening of the sun and ambient for the host's lights (one value on `Weather`),
  snow-cover ground bounce, whiteout (sky and ground the same white in a blizzard).

## Storm kinds (how the phases compose)
| Kind | Needs | Look | Distinct gate |
|---|---|---|---|
| Thunderstorm | 3 cells, 4 rain, 5 gust front, 6 lightning | dark-based Cb, curtains, shelf cloud, flashes | flash light + resolve recovery |
| Blizzard | 2 cold air, 4 snow + accumulation, 5 blowing snow, 7 whiteout | horizontal snow, visibility < 200 m, white ground | visibility at the camera vs red arm (blowing snow off) |
| Sandstorm / haboob | 2 dry air + wind field, 5 dust wall and grains | brown wall, red sun, near-zero contrast inside | dust only where ground is dry (red arm: ignore ground) |
| Rain shower | 3 cells, 4 rain | grey curtains under cumulus | exists (shafts) |

## Performance on the GPU (reference: Apple Silicon / Metal)
State today: every heavy pass already runs on the GPU as a fragment pass (cloud shadow, march, resolve, composite; shaft march and
resolve; fog march, resolve, composite), rain and splashes are vertex-generated quads with no CPU state, timing uses timestamps at
pass boundaries (Apple GPUs have no timestamps inside a pass, so keep it that way). CPU per frame: storm selection and the async
fog bake. More speed now comes from doing less work per pixel, not from moving work to the GPU.

| Item | Why | Status |
|---|---|---|
| Rain draws all 24000 streak and 6000 splash quads at any intensity > 0.01 | `rain.wesl` hides quads by a hash vs intensity, so drizzle costs as much vertex work as a downpour. Make visible quads a prefix (quad id < intensity x count) and draw that range, or a few mesh sizes. | open, measure on Metal first (docs/bugs.md) |
| Rain overdraw at Retina | Premultiplied streaks close to the camera fill large areas. Cap near-camera size, or draw near drops at lower res. | measure |
| `shader-f16` | Apple GPUs run f16 ALU at twice the rate and with half the registers (better occupancy). Candidates: lighting and phase maths in the cloud and fog marches, never positions or distances. Needs the wgpu feature check and an f32 fallback (lavapipe). Gate: image diff vs f32 within budget, Metal A/B cost. | proposal, owner's call |
| Tile memory (TBDR) | Each fullscreen pass stores and reloads its attachments. Merge passes where a pass only reads the previous one at the same pixel (fog resolve + composite is the candidate), keep formats small (Rg11b10 / Rgba16f). | proposal |
| Compute where it fits | Map advection (Phase 2), ground-state accumulation (Phase 4), precip occlusion refresh, fog bake if the CPU bake ever shows in a frame (measure first: it is async today). Fragment passes stay fragment where they write a screen image. | per phase |
| Flash cost spikes | The in-cloud flash light runs only on live-flash frames; budget it as a peak, not a mean. | Phase 6 |
| Auto scaler | Reads `PASSMS` / `FOGMS` and steps the quality tier to hold the budget. | Later |

Budget: the 3 ms split above has 0.2 ms slack and none of Phases 2-6. Proposed (owner's call): rain, snow and sand share the
rain FX 0.3 (one pass); dust and blowing snow share the fog's 0.8 (they replace fog cost and rarely peak together); new: precip
occlusion 0.05 mean, map advection and ground state 0.1, lightning 0.1 mean / 0.5 peak. Mean 3.05 ms, flash peak 3.45 ms:
either accept that, or take ~0.1 ms from clouds. Exclusive peaks (a blizzard has no rain shafts, a haboob no ground fog) are
what keep the mean near 3 ms.

## Later
- Seasons and climate presets; host terrain hooks (orographic rain is in core, unwired).
- A performance scaler that keeps quality inside the budget (update div, steps, render scale).
- Tornadoes and waterspouts; aurora; volcanic ash (a dark dust medium).

## Order and ownership
Phase 0 tail and Phase 1 first. Phase 6 (lightning) can start after Phase 1 against today's analytic cells. Phase 2 before 3, 4
and 5 (they read the air state, the wind field and the advecting map); snow particles can start in Phase 4 on a temperature
config knob before Phase 2 lands, then switch to the air state. After Phase 2, phases run in parallel sessions, one owner per file.

| Owner | Files |
|---|---|
| Phase 0 (fog) | `src/fog.rs`, `src/fog/`, `src/shaders/fog.wesl`, `crates/nephthene_core/src/fog.rs`, `src/panel.rs`, `src/sky.rs`, `tests/fog.rs`, `docs/fog.md` |
| Phase 1 (gates) | `src/clouds/render.rs` (timing only), `src/rain.rs` (timing only), `tests/clouds.rs`, `tests/golden.rs`, `tests/golden/`, new `tests/*.rs`, `tools/` |
| Phase 2 (weather state) | new `crates/nephthene_core/src/air.rs`, `crates/nephthene_core/src/lib.rs` (map, wind), new map-advection pass |
| Phase 4 (precipitation) | `src/rain.rs`, `src/shaders/rain.wesl`, `crates/nephthene_core/src/rain.rs`, new surface material |
| Phase 5 (dust) | `src/shaders/fog.wesl` and `src/fog/` after Phase 0 hands them over |
| Phase 6 (lightning) | new `crates/nephthene_core/src/lightning.rs`, `crates/nephthene_core/src/storm_fx.rs`, new bolt shader; `clouds_march.wesl` flash term (ask first) |
| Shared, append-only | `crates/nephthene_core/src/config.rs` (own section only), `examples/poses.json` (new poses only), `docs/bugs.md`, `docs/decisions.md`, `README.md` |
| Ask first | `examples/sky.rs`, `src/lib.rs`, shaders other than your own |

## Open questions for the owner
1. Budget: accept ~3.05 ms mean / 3.45 ms on flash frames, or take ~0.1 ms from clouds to stay at 3 ms?
2. Dust and blowing snow inside the fog march (proposed) or a separate pass?
3. Surfaces: ship an `ExtendedMaterial` the host opts into (proposed), or keep editing the host's `StandardMaterial`?
4. Air state: global surface temperature plus lapse rate (proposed, cheap), or a temperature map from the start?
5. `shader-f16` on Metal: try it behind a feature check with an f32 fallback?
