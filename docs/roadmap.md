# Nephthene roadmap

Nephthene is a complete, physically grounded, art-directable weather system for Bevy: sky, clouds, fog, rain, snow, dust, wind,
storms (thunder, snow, sand), lightning and weather sound cues, driven by a weather field, a climate field and one clock, every
knob in `Config`. A host game only consumes it.

## The target

"Rival a AAA weather system" is not a measurable claim, so it is kept here as a scoreboard with a denominator (the matrix at
**Scoreboard** below) and a stated bar:

- **80% of the matrix at `have`** - 20 of its 25 rows - each with a gate that is red without the feature, green with it, on both
  reference GPUs.
- **Inside the tier budget** (see **Performance**): the whole stack fits its tier's millisecond wall at its tier's resolution.
- **Judged by eye**: every row that changes a pixel ships renders of the fixed poses and the owner accepts them. Green gates do
  not prove a look.

Today the matrix reads 7 `have`, 6 `partial`, 12 `none`: about 40% weighted. The number is arbitrary; the denominator is not,
and neither is any individual row.

## The five commitments

Everything below serves one of these. A change that serves none of them needs a reason in `docs/decisions.md`.

### C1 Ergonomic for a designer *and* for an agent

A person with a paint program and a JSON file, or an agent with a shell, must be able to author and verify weather without
reading Rust.

- **One config, addressable by string.** `Config` is already one serde struct with ~250 defaulted fields and the example already
  parses `--set a.b=json`. Promote that path-setter to public API (`Config::set_path(&str, &str) -> Result`) so hosts, consoles,
  save files, the panel and agents all use one mechanism. Gate: set every leaf path by string and round-trip it.
- **A machine-readable schema.** Emit `config.schema.json` (JSON Schema, doc comments as `description`, defaults included) from
  the same struct. An editor validates a file, an agent discovers a knob without grepping. Gate: the schema validates
  `config.reference.json` and rejects a wrong type.
- **Named presets.** `presets/*.json` are partial configs merged over the defaults, addressable by name (`--preset overcast`,
  `Config::preset("storm")`). Ship a library: `clear`, `sparse`, `cloudy`, `overcast`, `storm`, `blizzard`, `sandstorm`,
  `dawn_fog`. A designer composes `--preset overcast --set clock.start_hour=6`. Gate: every shipped preset loads, validates and
  renders a pose without a panic.
- **A weather state the machine can read.** `Weather::describe() -> String`: one line, no GPU
  ("14:12 overcast 0.82 rain 0.31 wind 6.0 m/s from 240 deg snowline 2100 m storm 12 km SW"). An agent, a log line, a QA caption
  and a bug report all read the same text. Pure, unit-gated. This is what makes the system verifiable by an agent with no image.
- **Determinism as an interface.** Same seed plus same clock equals same weather on every machine. `Weather` saves and restores
  in full (today only `map_shift`). Gate: two runs from one seed agree bit for bit on the described state at 100 sampled hours.

Deliberately: no bespoke editor. The authoring surface is a PNG, a JSON file, the runtime panel and the shot harness.

### C2 Scalable: cost is O(pixels), never O(world)

An open world 100x larger must not cost one millisecond more.

- **The invariant.** Every per-frame GPU pass is screen-space and camera-local; every CPU bake has a stated extent around the
  camera, a refresh threshold and an asynchronous task. Nothing iterates the world.
- **Gate (new, cheap, automatic where a GPU exists).** Render a pose with the map extent, the terrain grid and the storm count
  scaled 100x; frame cost and the described state agree with the small world within run-to-run noise. Red arm: a bake whose
  extent follows the world instead of the camera.
- The fog bake is already the pattern to copy (`ground_fog.extent_m` 12000, `bake_px` 128, `rebake_m` 1500, async task).
- What must be fixed to hold the invariant: the weather map is one fixed rectangle (Phase 8 tiles and streams it); the cloud
  march pays for all of `clouds.max_distance_m` 90 km with no distant LOD (Phase 1); storm cells are capped at 16 by a uniform
  (Phase 3); the orographic FFT is global (Phase 2 tiles it around the camera).

### C3 Everything toggleable, and off costs nothing

- Every feature has an `enabled` in its config section. Today only `clouds`, `ground_fog`, `storms` and `storms.lightning` do:
  rain, shafts, stars, moon, dust and the sky driving have none.
- **Off means no pass, no system, no allocation.** `FogPlugin` is always built and idles; that is acceptable only while a gate
  proves the idle cost is zero.
- **Gate (new).** Everything off: `PASSMS` reports no spans, the frame time matches the same scene with the plugin absent within
  noise, and no weather texture is allocated. Red arm: one feature left on.
- A toggle is a feature, not a test switch. Test switches (`static_gpu`, `ignore_terrain`, `hash_noise`, `no_reproject`, ...)
  stay separate and stay documented as red arms.

### C4 Performant on a good GPU, by configuration

Two reference GPUs, four tiers, one budget rule. See **Performance**.

### C5 Well integrated: the weather belongs to the place and to the moment

- **To the place.** A climate field (biome, mean temperature offset, humidity) plus terrain altitude through a lapse rate plus
  orographic uplift from the wind over the terrain. A blizzard in the mountains is then not scripted: it is what the air state
  says at 3000 m in winter. Phase 2.
- **To the moment.** A **director**: the host or a designer requests a state ("blizzard, start in 120 s, over 60 s"), the
  director schedules it, transitions it and reports it; plus a drama guarantee so the horizon is never empty. Phases 2 and 3.
- Gate for the place: the same world position at 0 m and at 3000 m in winter gives rain and snow respectively; red arm, lapse
  rate 0. Gate for the moment: a requested blizzard is fully established within its transition time and the described state says
  so; red arm, the request ignored.

## Rules (all phases)

- **Separate.** Nothing host-specific in the repo (AGENTS.md rule 7: no game names, assets or IDs). A host hooks in through
  markers (`NephtheneSun`, `WetGround`), resources (`Ground`, `Weather`, `FogState`, ...), events and config only.
- **No magic values.** Every tunable is a `Config` field with a documented default, labelled untuned until the owner tunes it.
- **Proof.** Every feature ships a gate that is red without it and green with it, and keeps its red arm (a config switch). A gate
  whose red arm cannot fail on any renderer is deleted and logged, not kept. Bounds come from a stated budget, never from a
  measurement.
- **Two reference GPUs.** **A: Apple Silicon / Metal** (the owner's review machine, the tightest ALU and bandwidth budget).
  **B: NVIDIA RTX 5070 Ti / Vulkan** (the headroom tier, where the high and ultra tiers are measured). Cloud sessions render on
  lavapipe (CPU Vulkan): it matches Metal within ~1% on image gates and is a good *relative* cost proxy (A/B wall time), but says
  nothing about absolute GPU cost or about texture vs ALU trade-offs. A look or cost change is accepted only after a run on A and
  a run on B.
- **Three golden packs, one bound.** `tests/golden/` holds a pack per renderer (`metal/`, `vulkan/`, and a lavapipe pack or a
  printed skip), selected by the detected adapter. This also closes the bug where `cargo test --release` is red in every cloud
  session because the Apple pack runs on lavapipe.
- **Cost is O(pixels).** C2's invariant, gated.
- **Off costs nothing.** C3's invariant, gated.
- **Look first, by eye.** Every visual change ships renders of the fixed poses (`examples/poses.json`); the owner judges them.
  Look values (exposure, amplitudes, colours) are the owner's call.
- **Determinism.** Systems that share state are ordered explicitly: an unordered pair in the shot harness moved the golden sway
  frames when unrelated systems were added.
- **Log.** Decisions in `docs/decisions.md`, bugs in `docs/bugs.md` (append rows, change status, never delete).
- **Roles** (AGENTS.md): a planner session designs and writes task graphs; Haiku executors do the nodes; a fresh verifier checks
  each one. One owner per file across parallel sessions (see **Order and ownership**).

## Scoreboard

The denominator for the 80% target. `have` requires a green gate with a kept red arm on both reference GPUs.

| # | Row | State | Where |
|---|---|---|---|
| 1 | Raymarched volumetric clouds, temporally resolved | have | G7-G14 |
| 2 | Cloud shadows on the world | have | shadow cookie, cost gate |
| 3 | Cloud types and presets (cumulus - stratus - cumulonimbus) | partial | map B channel exists, no preset library |
| 4 | Distant cloud LOD (cheap horizon layer, not the march) | none | Phase 1 |
| 5 | Physically based sky and aerial perspective | have | Bevy `Atmosphere`, `sky.sun_occlusion`, G1/G2 |
| 6 | Terrain-following volumetric ground fog | have | G15-G19, G21 |
| 7 | Light shafts through cloud gaps (god rays) | partial | rain shafts only |
| 8 | Rain particles and surface wetness | partial | no occlusion, density bug open |
| 9 | Snow particles and accumulation | partial | flakes and sleet in the rain pass from the air's `precip_kind`; G27 shape green, visibility red (look call); no accumulation yet |
| 10 | Precipitation occlusion (nothing falls indoors) | partial | `Shelter` roof grid from bounding boxes, G28; ground under roofs still wets |
| 11 | Lightning and thunder | partial | slice 1 in: channel, flash light, strike/thunder messages, G23/G24; on by default, Metal pending |
| 12 | Storm cells with a lifecycle | partial | analytic capsules, read as lenses |
| 13 | Weather field authoring (paint a region) | have | `WeatherMap::load`, panel map |
| 14 | Wind field driving clouds, rain, fog, foliage | none | one global vector; Phase 2 |
| 15 | Air state: temperature, humidity, freezing level | partial | `air::Field`, core gates; climate field open |
| 16 | Climate and altitude drive the weather | partial | altitude (gate C5-place) and climate (`climate.rs`) drive the air; the weather map's coverage and precip do not read the climate yet |
| 17 | Director: requested states and transitions | none | Phase 2/3 (C5) |
| 18 | Dust and sandstorm | none | Phase 5 |
| 19 | Blowing snow and whiteout | none | Phase 5/7 |
| 20 | Rainbows and halos | none | Phase 4 |
| 21 | Weather audio mapping | have | `core::audio` (mapping only, by design) |
| 22 | Quality tiers | none | Phase 1 (C4) |
| 23 | Upscaler (DLSS / TAA) integration | none | Phase 1 (C4) |
| 24 | Save, restore and seeded determinism | partial | `map_shift` only; Phase 2 (C1) |
| 25 | Heat haze and mirage | none | Phase 5 |

## Phase 0: finish ground fog - merged (PR 1); tail below

Done when: both reference GPUs run green (G15-G19, G21, goldens), the owner accepts the renders.
- Cost: gate G19 <= the tier budget's fog share with red arm `render_scale=1` + `hash_noise=true`. Reference-A number pending.
- Visible without forcing: fog follows the effective mean wind (panel override included); document that the default 6 m/s wind
  forms none and that a calm night does (>= 0.7 by 5 h at <= 2.5 m/s). Whether the *default* weather should ever be calm is a
  Phase 2 question, and once the director lands it is the director's answer.
- Sunlit pool reads as snow at 8 h: owner decides exposure (scene runs ~2 stops over sunny-16) and billow amplitude; then a
  surface-texture gate with billows off as red arm.
- Horizon seam inside fog at `fog_floor` 8 h: from the burn-off lift (a clear gap under the pool the camera stands in), not the
  terrain; owner's look call on the lift near amount 1 (`docs/bugs.md`).
- Reference-B (5070 Ti) run of the fog gates and renders: new, needed before the fog rows count as `have`.

## Phase 1: cost, tiers, toggles and the agent surface

Done when: every pass has a cost gate on both reference GPUs, the four tiers exist and hold their budgets, the toggle and scale
invariants are gated, and the config is addressable and schema'd. This is C1-C4's foundation; everything later lands with a cost
per tier instead of one number.

- **Quality tiers** (C4). `Config::tier(Tier)` sets every pass's `render_scale`, `steps`, `update_div`, shadow resolution and
  cadence. Four tiers, table under **Performance**. Starting values untuned; the first measurement on each reference GPU sets
  them. Gate: each tier holds its wall on its reference GPU, and each tier is measurably cheaper than the one above it.
- **Second reference GPU** (C4). Cost and golden gates run on the 5070 Ti; `tests/golden/` gains per-renderer packs; the
  always-red-in-cloud-sessions golden bug closes with a printed skip on software renderers.
- **Upscaler integration** (C4, new). Bevy ships DLSS in `bevy_anti_alias` (since 0.17, optional feature, `dlss_wgpu`); confirm
  what 0.20 exposes. The weather passes must render at the pre-upscale resolution and the cloud and fog temporal resolves must
  not fight the upscaler's own history: weather pixels need correct motion vectors, or clouds ghost twice. Gate: the cloud
  turning-ghost metric (G7) with the upscaler on stays within the no-upscaler budget. Red arm: weather motion vectors zeroed.
  Owner's call whether this is Phase 1 or deferred; a 5070 Ti user will run an upscaler, so it is treated as Phase 1 here.
- **Distant cloud LOD** (C2, scoreboard row 4). The march pays for `max_distance_m` 90 km. Beyond a crossover distance, composite
  a cheap layer (a shaded 2D field from the same weather map) instead of marching. Gate: image diff beyond the crossover within
  budget against a full march, and a measured cost drop. Red arm: crossover at infinity.
- **Toggle audit** (C3). An `enabled` in every section; the everything-off zero-cost gate; the idle-plugin cost gate for
  `FogPlugin`.
- **Scale invariant gate** (C2). The 100x-world gate above.
- **Agent and designer surface** (C1). `Config::set_path`, `config.schema.json`, the preset library, `Weather::describe()`. All
  pure, all unit-gated, no GPU: cheap and it makes every later phase verifiable by an agent from text.
- Per-pass GPU timing for every pass at each tier's resolution, built on `clouds.pass_timing` (`PASSMS`) and `FOGMS`; skip on
  software renderers with a printed reason. Rain already has a span (`transparent`).
- Re-add the checks the extraction dropped: sun disc and MSAA hairline (old G1-G4) are back; the panel pointer self-test
  (`--panel-test` is parsed but unused) and rain and storm shot scenes remain.
- Fix the golden pack's fixed 0.04-0.21 offset by strengthening the pack (more frames, the reference renderer), not by widening
  the bound.
- Audit system ordering (resources written by one system and read by another in the same set) and order them.
- Public panel root so a host can add widgets, font and camera binding.

## Phase 2: weather state, climate and topography (the foundation the rest reads)

Done when: rain vs snow, fog formation, dust lifting and cloud coverage all read one air state; that air state knows where it is
(climate, latitude, altitude) and what the terrain does to the wind; the weather field moves with the wind; and a host can ask for
a state by name. This phase is C5.

- **Air state in core**. Done: `nephthene_core::air` (force-restore surface temperature from clock, season, latitude and cloud
  cover; dew point, humidity, lapse rate, freezing level, snow line) exposed as `Weather.air`; fog formation reads the dew point
  spread (red arm `ground_fog.private_cooling`); `docs/air.md`. Done: the air is a temperature field (`air::Field`, owner's call:
  a full field from the start), a grid over the weather map in the map's frame, so it moves with the weather; each cell under its
  own cover (gate `the_field_follows_the_cover_under_it`, red arm `cloud_shade` and `cloud_block` 0) and restores to the climate under
  it (below). Open: a non-uniform wind advects it with the weather field (step 2 below).
- **Altitude is an input, not an afterthought.** Done: cells hold the temperature reduced to `ref_alt_m`; `Weather.air` is read at
  the `Ground` height under the camera through the lapse rates (dew point 1.8 K/km), so the same map texel is rain in the valley
  and snow on the ridge. Gate C5-place `rain_in_the_valley_snow_on_the_ridge`; red arm, lapse rate 0. Open: fog formation still
  reads the air at `ref_alt_m` (its spin-ups integrate there); the pool floor's altitude is the upgrade.
- **Climate field** (new, the second map the old roadmap reserved): climate class or biome, mean temperature offset, humidity,
  ground snow and ground wetness. This is how a designer makes a region a desert, a tundra or a rainforest with one paint pass,
  and it is what makes the weather field's coverage and precip *local* rather than global. The weather field's RGBA8 is full
  (coverage, precip, type, dust), so this is a second image with the same extent and sampling path.
  Done: R mean temperature offset, G humidity (signed, 128 neutral), B ground snow, A ground wetness, fixed to the world; it
  localises the air config per place (`climate.rs`, `docs/climate.md`; gates `a_painted_cold_place_is_colder`,
  `desert_air_is_dry_in_the_afternoon`, `snow_cover_cuts_the_days_warming`, red arm `climate.ignore`). Open: climate class or
  biome (a second image or presets), the climate shaping the weather field's coverage and precip, and the season's swing per place.
- **Precipitation type from the air state**: rain, snow, sleet, hail (`precip_kind` 0..1 blends, not a switch, so a front turns
  rain to snow over minutes). Done: `precip_kind` (0 rain, 1 snow, sleet between) from the temperature at the ground
  (`air.snow_c`..`air.rain_c`, Jennings et al. 2018), on `Weather.air` and in the panel. Open: hail (convective, with the storm
  fields in Phase 3); `describe()` (Phase 1 agent surface); rain particles reading it (Phase 4).
- **Orographic precipitation, wired** (scoreboard rows 15/16, C5). `crates/nephthene_core/src/orographic.rs` already implements
  Smith & Barstad (2004) linear orographic precipitation in one FFT pass, pure and unit-gated (windward > 1.5x lee, flips with the
  wind sign, flat terrain gives zero), and nothing calls it. Wire it:
  1. Bake `Ground`'s `HeightGrid` through `orographic::compute` into the precip and coverage contributions (`OroOut::precip` and
     `OroOut::cloud`), on the camera-local tile, asynchronously, cached per (tile, wind octant) - the FFT is O(N^2 log N) over a
     padded square, so it is a bake, never a frame cost (C2).
  2. Resolve the resolution mismatch: `HeightGrid.cell_m` is metres, a default weather texel is ~3.1 km. Either raise the field
     resolution or keep the orographic contribution as a separate finer field combined at sample time.
  3. **Frame conflict, decided** (owner, `docs/decisions.md`): two fields. The synoptic field advects; the orographic field is
     static and terrain-locked; the sampler combines them (proposed: a `max` for precip, a weighted add for coverage).
  4. Gate: windward-to-lee precipitation ratio across a ridge, with `oro.gain = 0` as the red arm; plus a render showing cap cloud
     on the crest and a dry lee.
- **Weather field advects with the wind**. Step 1 done: under one global wind the field is translated, not resampled
  (`map.advect`, on by default; `map.wrap`; `map.start_shift_m` for save/restore; gates `map_tests` and G22;
  `docs/weather_map.md`). Step 2, with the wind field below: a non-uniform wind cannot be a translation, so the field becomes a
  ping-pong compute or fragment pass on the GPU (semi-Lagrangian) with a CPU copy for `Weather.here`. Owner: the default `advect`,
  and a tileable field before `wrap` goes on.
- **Wind field** (scoreboard row 14): replace the one global vector (`lib.rs` ponytail) with a coarse 2D field (mean + gusts +
  storm outflow + terrain channelling), sampled by clouds, rain angle, fog drift, dust lift and audio, and readable by the host
  (foliage, cloth, gameplay). Gusts must not turn the mean (the `map_wind` ponytail).
- **The director** (scoreboard row 17, C5-moment): weather states over time (calm clear nights, fronts, showers, cold snaps, dry
  windy spells) on the seeded clock, plus a host API to request one with a transition time, and a drama guarantee (a horizon that
  is never empty, whatever the local field says). Dawn fog then appears without forcing. Gate C5-moment above.
- **Determinism and save/restore** (C1): same seed plus same clock equals same weather everywhere; the whole state round-trips.

## Phase 3: storm cells as 2D fields (Schneider, SIGGRAPH 2022 course on 2D cloud fields)

Done when: storm cells come from field data, the lifecycle runs on the clock, the director can place one, and the renders read as
cumulonimbus, not capsules.

- Storm cells are fields painted into the advecting weather field, not analytic capsules (those read as mushrooms, and the
  authored-cell bug shows a 12 km tower rendering as a 10 px lens). Lifts the 16-cell uniform cap (`storm::MAX_CELLS`), which a
  squall line or a multicell system needs, and removes the per-cell uniform cost (C2).
- Cumulonimbus: towers to the tropopause (default 14-16 km), anvils that spread and thin downwind, overshooting tops, a shelf
  cloud on the gust front. Cumulus congestus grows from smooth 2D height and strength fields, not squashed noise: no slices, no
  streaks.
- Dark bases: ambient occluded by the column above; ground bounce shadowed by the storm.
- Lifecycle on the clock (developing, mature, dissipating), squall lines and multicell systems; the seeded timetable, authored
  storms, and **director placement** ("a storm on the horizon to the west in 90 s") in one mechanism.
- Genesis reads the *shifted* field (today the timetable samples the unshifted one: `docs/bugs.md`), and storms then advect with
  the weather they came from.
- Wire the authored `intensity`, which today changes nothing.
- Art-directed horizon: some distant drama always, whatever the local field says. Sky scale real by default, cloud base per
  climate region.

## Phase 4: precipitation and surfaces

Done when: rain, snow and sleet fall from the field's precip and the air state's kind, stop under roofs, and leave the ground wet,
white or dusty; each with a red-armed gate.

- **One particle pass, several kinds.** Keep the stateless vertex-shader scheme (`rain.wesl`, mirrored in `core::rain`): a quad's
  kind picks its motion and look. Rain: streaks. Snow: slow flakes with flutter and wind drift, sprites not streaks. Sleet and
  hail: short bright pellets that bounce. Sand: grains near the ground (Phase 5). No second particle system.
- Fix the open rain bugs first: forced full rain reads as almost no rain (0.12% of sky rows against a 1% budget - a look call),
  and every quad does vertex work at any intensity above 0.01 (make visible quads a prefix, `id < intensity x count`).
- **Precipitation occlusion map** (scoreboard row 10, replaces the `rain.rs` ponytail "rain also falls indoors"): an orthographic
  depth render from above around the camera, refreshed when the camera moves a fraction of its extent - camera-local by
  construction (C2). Streaks, flakes and splashes stop on roofs and overhangs; wetness and snow cover read it too. Red arm:
  occlusion off.
- Rain curtains and shafts under cells, virga with high bases, rain intensity from the field.
- **Weather surface material**: an `ExtendedMaterial<StandardMaterial, _>` the host opts into, replacing the per-entity
  `WetGround` colour edits: wet darkening, puddles from a noise and slope mask, ripples, snow cover by slope and accumulated
  depth, dust film. Accumulation and melt per texel in the Phase 2 ground-state field, on the GPU.
- **Light shafts through cloud gaps** (scoreboard row 7): the rain-shaft march already exists; drive it from cloud transmittance
  with no rain required. Gate: shaft contrast under a broken deck, red arm the shaft pass off.
- Rainbows and halos: analytic, sun behind the camera, rain ahead, sun elevation < 42 deg; nearly free.
- Optional: drops and frost on the camera lens (a host post-process toggle).

## Phase 5: wind, dust and blowing snow

Done when: a gust front lifts dust where the ground is dry and dusty, a haboob wall approaches and engulfs the camera, and blowing
snow reduces visibility near the ground; gates on wall visibility and on dust only where the ground is dry.

- Gusts and gust fronts from storm outflow (`storm_fx::outflow_at` exists in core, unwired) into the Phase 2 wind field.
- **Low-atmosphere media in the fog march.** Dust and blowing snow are media in the existing ground-fog march (one depth stop,
  one temporal resolve, one composite), not new passes: per medium a density from a field channel and an analytic height profile,
  its own albedo, phase and coloured extinction (dust warm and absorbing, snow white and forward scattering). Proposed; owner's
  call (`decisions.md`) against a separate pass. Costing them inside the fog's share is what keeps the budget (exclusive peaks: a
  blizzard has no rain shafts, a haboob no ground fog).
- Haboob: a dust wall 1-3 km high riding the gust front at 10-25 m/s, sun dimmed and reddened, the sky's Mie dust term (field A)
  raised. Sand grains near the ground in the Phase 4 particle pass.
- Heat haze and mirage over hot ground (a screen-space refraction toggle), the dry-weather companion of dust. Reads the air
  state's surface temperature, so it belongs to a desert climate without being scripted.

## Phase 6: lightning and thunder

Can start after Phase 1, in parallel with Phases 2-5: the flash schedule (`storm_fx::flashes`) and the analytic cells exist.
Slice 1 built (lavapipe; Metal pending): bolt geometry in core (`core::lightning`, unit tests), channel ribbons, one point light per
live flash, `LightningStrike` / `Thunder` messages, gates G23 (channel reads as a line) and G24 (flash lights the ground). Off by
default (`storms.lightning.enabled`). Left for slice 2 (touches `clouds_march.wesl`, ask first): clouds lit from inside, the
resolve-vs-flash term, in-cloud flashes, bloom, fog lit by the flash.
Done when: cloud-to-ground and in-cloud flashes light the clouds, terrain and fog, thunder arrives late by distance, and the host
receives events.

- **Bolt geometry in core** (slice 1, in): seeded stepped leader with branches (midpoint displacement), deterministic from the
  `Flash`; pure, unit-tested (connectivity, ends on the ground for CG, stays inside the cell for IC). `core::lightning`.
- **Bolt render** (slice 1, in): emissive camera-facing ribbons, at least `min_px` wide with radiance scaled by width / drawn
  width, added over the scene; gate G23. Left for slice 2: HDR bloom, and whether 3-5 return strokes over ~0.5 s and channel
  afterglow read as one flash.
- **Clouds lit from inside** (slice 2, touches `clouds_march.wesl`: ask first): one point-light term in the cloud march from the flash position with a cheap transmittance
  (distance and local density, no light march), evaluated only while a flash is live.
- **Temporal resolves vs flashes** (slice 2): the cloud and fog resolves blend history at 0.1 per frame, so a one-frame flash would smear
  over ~10 frames and its rise would be clamped. Flash light needs its own unresolved term (or the history re-weighted on flash
  frames). Gate: luma at flash frame N, and back within budget of the pre-flash level by N+k.
- **Scene light** (slice 1, in for the strike): one Bevy `PointLight` per live flash from a pool of `max_live`, no shadows,
  placed along the channel at `light_at`; gate G24. Left for slice 2: the ambient bump and fog lit by the flash.
- **Thunder** (slice 1, in): `thunder_delay_s` by distance, and the `LightningStrike` / `Thunder` messages a host reads. Still
  open: the wind and rain audio mapping (`core::audio`) becomes messages too, so a host wires one stream rather than polling
  resources.
- Thundersnow (rare flashes in strong snow storms) falls out of the same code.

## Phase 7: day and night

- Night sky (moon with inclination, stars, airglow), twilight colours, clouds and fog lit by the moon with terrain shadow.
- Lighting under weather: overcast and storm darkening of the sun and ambient for the host's lights (one value on `Weather`),
  snow-cover ground bounce, whiteout (sky and ground the same white in a blizzard).

## Phase 8: scale - tiled and streamed weather (C2)

Done when the weather field is unbounded and nothing in a frame costs more because the world is bigger. Can start after Phase 2
step 2 (the field is a GPU pass by then).

- `WeatherMap` becomes a `WeatherField` trait: the current single rectangle is one implementation, a tiled streamed source is
  another. Resolution decouples from extent, so a 500 km world is not a 500 km texture.
- Tile residency around the camera with a stated budget in tiles and megabytes; a tile loads asynchronously and the sampler holds
  the last good value, never stalls.
- The same tiling for the climate field, the ground-state field and the orographic bake.
- `wrap` plus tiling gives an effectively infinite world; the tileability requirement on an authored field is documented, and the
  built-in procedural field is replaced with one that actually tiles (today it is a product of two sines and seams).
- Gate: the C2 100x gate at 1000x, plus a tile-thrash gate (a camera flying at 200 m/s holds its frame budget and never samples a
  missing tile).

## Phase 9: authoring and the agent API (C1 payoff)

Done when a designer or an agent can author, preview and verify a region without a Rust toolchain in the loop.

- A field authoring mode in the example: load a PNG, fly it, paint coverage, precip, type, dust, climate class, save. Mouse only,
  headless-safe, no new dependency.
- Region probe: click a point, get `describe()` for it at any hour, plus a 24 h strip of what that place does over a day. This is
  the fastest way to answer "is this region a desert" without rendering it.
- The command surface an agent drives: `--preset`, `--set`, `--describe`, `--probe x,z --hour h`, `--shot pose`, all stable, all
  documented in one place, all exit-coded. An agent then verifies weather from text and only renders when a look is in question.
- Optional, owner's call: the same surface over a socket so an external tool or an agent drives a live session.

## Storm kinds (how the phases compose)

| Kind | Needs | Look | Distinct gate |
|---|---|---|---|
| Thunderstorm | 3 cells, 4 rain, 5 gust front, 6 lightning | dark-based Cb, curtains, shelf cloud, flashes | flash light (G24, in) + resolve recovery (slice 2) |
| Blizzard | 2 cold air + altitude + climate, 4 snow + accumulation, 5 blowing snow, 7 whiteout | horizontal snow, visibility < 200 m, white ground | visibility at the camera vs red arm (blowing snow off); and the C5-place gate (same place, 0 m rain / 3000 m snow) |
| Sandstorm / haboob | 2 dry climate + wind field, 5 dust wall and grains | brown wall, red sun, near-zero contrast inside | dust only where the ground is dry (red arm: ignore ground state) |
| Rain shower | 3 cells, 4 rain | grey curtains under cumulus | exists (shafts) |
| Orographic rain | 2 orographic wiring + wind field | wet windward slope, cap cloud on the crest, dry lee | windward/lee ratio, red arm `oro.gain = 0` |

## Performance

State today: every heavy pass already runs on the GPU as a fragment pass (cloud shadow, march, resolve, composite; shaft march
and resolve; fog march, resolve, composite), rain and splashes are vertex-generated quads with no CPU state, timing uses
timestamps at pass boundaries (Apple GPUs have no timestamps inside a pass, so keep it that way). CPU per frame: storm selection
and the async fog bake. More speed comes from doing less work per pixel, not from moving work to the GPU.

### Tiers (C4)

The budget is a *time* wall per tier; a bigger GPU buys a higher tier at the same wall, not a smaller number. Values are the
shape, not tuned numbers: the first measurement on each reference GPU sets them, and until then every one is untuned.

| Tier | Target | Reference GPU | Wall | Starting knobs (untuned) |
|---|---|---|---|---|
| `low` | 1080p, 120 fps headroom, iGPU and handheld | lavapipe relative, then A | 1.2 ms | clouds `render_scale` 0.25, `steps` 48, `update_div` 8, `shadow_res` 256, `shadow_every` 8; fog `render_scale` 0.25, `steps` 32 |
| `medium` | 1080p60 | A (Apple Silicon) | 3.0 ms | today's defaults: clouds 0.5 / 72 / 4, `shadow_res` 512, `shadow_every` 4; fog 0.25 / 48 |
| `high` | 1440p, 60-90 fps | B (5070 Ti) | 3.5 ms | clouds 0.5 / 96 / 2, `shadow_res` 1024, `shadow_every` 2; fog 0.5 / 64 |
| `ultra` | 2160p60, a weather showcase | B (5070 Ti) | 6.0 ms | clouds 1.0 / 128 / 1, `shadow_res` 2048, `shadow_every` 1; fog 0.5 / 96 |

Per-pass split inside the wall (proportional to the wall, owner's call; `medium` is today's split): clouds march + resolve 40%,
ground fog 27%, rain shafts 10%, rain FX 10%, cloud shadow 7%, slack 6%. New passes take a share, they do not add one: precip
occlusion and the field advection come out of slack and the cloud share; dust and blowing snow come out of the fog share; a
lightning flash is budgeted as a peak, not a mean.

`ultra`'s 6.0 ms is 36% of a 60 fps frame. That is defensible only for a weather-forward showcase; it is listed as an open
question rather than settled.

### Open performance items

| Item | Why | Status |
|---|---|---|
| Distant cloud LOD | The march pays for all 90 km of `max_distance_m`. A cheap composited layer past a crossover is the single biggest win available and it also serves C2. | Phase 1 |
| Rain draws all 24000 streak and 6000 splash quads at any intensity > 0.01 | `rain.wesl` hides quads by a hash vs intensity, so drizzle costs as much vertex work as a downpour. Make visible quads a prefix (quad id < intensity x count) and draw that range. | open, measure on A and B first (`docs/bugs.md`) |
| Rain overdraw at high resolution | Premultiplied streaks close to the camera fill large areas. Cap near-camera size, or draw near drops at lower res. | measure |
| Upscaler interaction | The cloud and fog resolves plus DLSS or TAA is double temporal filtering; without correct motion vectors for weather pixels, clouds ghost. | Phase 1 (C4) |
| `shader-f16` | Apple GPUs run f16 ALU at twice the rate and with half the registers (better occupancy); measure on B separately rather than assuming it carries. Candidates: lighting and phase maths in the cloud and fog marches, never positions or distances. Needs a wgpu feature check and an f32 fallback (lavapipe). Gate: image diff vs f32 within budget, A/B cost on both reference GPUs. | proposal, owner's call |
| Tile memory (TBDR) | On A each fullscreen pass stores and reloads its attachments; on B this matters much less. Merge passes where a pass only reads the previous one at the same pixel (fog resolve + composite is the candidate), keep formats small (`Rg11b10` / `Rgba16f`). A-specific, so it must not regress B. | proposal |
| Compute where it fits | Field advection (Phase 2), ground-state accumulation (Phase 4), precip occlusion refresh, the orographic FFT, fog bake if the CPU bake ever shows in a frame (measure first: it is async today). Fragment passes stay fragment where they write a screen image. | per phase |
| Auto scaler | Reads `PASSMS` / `FOGMS` and steps the tier to hold the wall. Needs the tiers first. | Later |

## Prior art and what we take from it

The repo cites its primary sources where the code uses them (`docs/weather_map.md`, `docs/fog.md`, `orographic.rs`). Published
primary sources are preferred over reading someone's implementation, for licensing as much as for clarity.

- A. Schneider, "Nubis: Authoring Real-Time Volumetric Cloudscapes with the Decima Engine", SIGGRAPH 2017; "Nubis, Evolved",
  SIGGRAPH 2022 - the cloud model, the static coverage signal, 2D cloud fields, scroll folded into the reprojection.
- S. Hillaire, "Physically Based Sky, Atmosphere and Cloud Rendering in Frostbite", SIGGRAPH 2016 - a world-space weather texture
  scaled and repeated over the world.
- R. B. Smith and I. Barstad, "A Linear Theory of Orographic Precipitation", J. Atmos. Sci. 61 (2004) - implemented in
  `orographic.rs`.
- **jiaozi158/UnityVolumetricCloudsURP** (MIT): a port of Unity HDRP's volumetric clouds to URP. Useful as a *checklist* of the
  property surface a shipped system exposes (cloud presets, a cloud map with coverage / rain / type channels - which is exactly
  this repo's weather field, independently arrived at - altitude and erosion noise kept separate, a shadow cookie that overrides
  the main directional light, a cheap distant cloud layer) and as a readable worked example of the HDRP technique. **Do not copy
  code from it**: it is a derivative of Unity's HDRP implementation, so the MIT badge on the port does not settle the licence on
  what it was ported from, and it is HLSL and URP-shaped, so nothing transfers mechanically to WGSL and Bevy. What it reinforces
  for this roadmap: the preset library (Phase 1, C1) and distant cloud LOD (Phase 1, scoreboard row 4). Logged in
  `docs/decisions.md`.

## Later

- Seasons and multi-year climate; climate presets beyond the shipped library.
- Tornadoes and waterspouts; aurora; volcanic ash (a dark dust medium).
- A performance scaler that keeps quality inside the wall (tier stepping).
- Networked weather (one authority, clients replay from seed and state).

## Order and ownership

Phase 0 tail and Phase 1 first: the tiers, the second reference GPU and the agent surface make every later phase cheaper to land
and to verify. Phase 6 (lightning) can start after Phase 1 against today's analytic cells. Phase 2 before 3, 4 and 5 (they read
the air state, the climate field, the wind field and the advecting weather field); snow particles can start in Phase 4 on a
temperature config knob before Phase 2 lands, then switch to the air state. Phase 8 after Phase 2 step 2. Phase 9 any time after
Phase 1. After Phase 2, phases run in parallel sessions, one owner per file.

| Owner | Files |
|---|---|
| Phase 0 (fog) | `src/fog.rs`, `src/fog/`, `src/shaders/fog.wesl`, `crates/nephthene_core/src/fog.rs`, `src/panel.rs`, `src/sky.rs`, `tests/fog.rs`, `docs/fog.md` |
| Phase 1 (cost, tiers, toggles, agent surface) | `src/clouds/render.rs` (timing and LOD), `src/rain.rs` (timing only), `tests/clouds.rs`, `tests/cost.rs`, `tests/golden.rs`, `tests/golden/`, new `tests/*.rs`, `tools/`, new `crates/nephthene_core/src/schema.rs`, new `presets/` |
| Phase 2 (weather state, climate, topography) | new `crates/nephthene_core/src/air.rs`, new `climate.rs`, `crates/nephthene_core/src/orographic.rs`, `crates/nephthene_core/src/lib.rs` (field, wind), new field-advection pass, new `docs/climate.md` |
| Phase 3 (storms) | `crates/nephthene_core/src/storm.rs`, `src/storms.rs`, storm shaders |
| Phase 4 (precipitation) | `src/rain.rs`, `src/shaders/rain.wesl`, `crates/nephthene_core/src/rain.rs`, new surface material, new occlusion pass |
| Phase 5 (dust) | `src/shaders/fog.wesl` and `src/fog/` after Phase 0 hands them over |
| Phase 6 (lightning) | `crates/nephthene_core/src/lightning.rs`, `src/lightning.rs`, `src/shaders/lightning.wesl`, `tests/lightning.rs` (all slice 1), `crates/nephthene_core/src/storm_fx.rs`; `clouds_march.wesl` flash term (slice 2, ask first) |
| Phase 8 (scale) | `crates/nephthene_core/src/lib.rs` (the `WeatherField` trait), new tile source, the field passes |
| Phase 9 (authoring) | `examples/sky.rs` (by then it is the authoring tool: ask first until Phase 1 closes) |
| Shared, append-only | `crates/nephthene_core/src/config.rs` (own section only), `examples/poses.json` (new poses only), `docs/bugs.md`, `docs/decisions.md`, `README.md` |
| Ask first | `src/lib.rs`, shaders other than your own |

## Open questions for the owner

1. **Tier walls**: accept 1.2 / 3.0 / 3.5 / 6.0 ms, and in particular `ultra` at 36% of a 60 fps frame? Or cap `ultra` at 4.5 ms
   and spend the headroom on resolution instead of steps?
2. ~~**The orographic frame conflict**~~ Answered 2026-10-22: two fields. The weather field moves with the wind; the orographic
   field stays fixed to the terrain; they are combined at sample time (`docs/decisions.md`).
3. **Upscaler**: is DLSS and TAA integration Phase 1 (proposed, because a 5070 Ti user runs one) or deferred until the tiers are
   measured?
4. ~~**Air state**~~ Answered 2026-10-22: a full temperature field from the start (`air::Field`, `docs/decisions.md`).
5. **Dust and blowing snow** inside the fog march (proposed) or a separate pass?
6. **Surfaces**: ship an `ExtendedMaterial` the host opts into (proposed), or keep editing the host's `StandardMaterial`?
7. **`shader-f16`** behind a feature check with an f32 fallback - measure on both reference GPUs first, or skip it?
8. **The director's default**: should stock Nephthene ever be calm and clear for a whole game day, or does the drama guarantee
   always put something on the horizon?
