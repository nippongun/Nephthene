# Nephthene roadmap

Nephthene is a complete, physically grounded, art-directable weather system for Bevy: sky, clouds, fog, precipitation, wind,
storms and weather sound cues, all driven by one weather map and one clock, every knob in `Config`. A host game only consumes it.

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

## Phase 0: finish ground fog (branch `claude/affectionate-pasteur-qtqvng`)
Done when: Metal runs green (G15-G19, goldens), the owner accepts the renders, then merge to main.
- Cost: half-resolution march + depth-aware composite, baked noise texture; gate G19 <= 0.8 ms at 1080p (proposed; the
  branch currently gates 1.5 ms) with red arm `render_scale=1` + `hash_noise=true`. Metal number pending.
- Visible without forcing: fog follows the effective mean wind (panel override included); document that the default 6 m/s wind
  forms none and that a calm night does (>= 0.7 by 5 h at <= 2.5 m/s). Whether the *default* weather should ever be calm is a
  Phase 2 weather-state question.
- Sunlit pool reads as snow at 8 h: owner decides exposure (scene runs ~2 stops over sunny-16) and billow amplitude; then a
  surface-texture gate with billows off as red arm.
- Horizon seam inside fog at `fog_floor` 8 h: not from the fog (present with fog, occlusion and clouds off); the 24 km test
  terrain ends in the valley notch. Close or enlarge the test terrain.

## Phase 1: cost and coverage gates
Done when: every pass has a Metal cost gate at 1080p, the dropped checks are back with red arms, the golden offset is fixed.
- Per-pass GPU timing gate (cloud shadow, march, resolve, shaft march/resolve, fog, rain) at 1080p and Retina on Metal, built on
  `clouds.pass_timing` (`PASSMS`) and `FOGMS`; skip on software renderers.
- Re-add the checks the extraction dropped: sun disc and MSAA hairline (old G1-G4), shafts do not lean with gusts (add a
  gust-lean red-arm switch), the panel pointer self-test (`--panel-test` is parsed but unused), rain and storm shot scenes.
- Fix the golden pack's fixed 0.04-0.21 offset by strengthening the pack (more frames, the reference renderer), not by widening
  the bound.
- Audit system ordering (resources written by one system and read by another in the same set) and order them.
- Public panel root so a host can add widgets, font and camera binding (after Phase 0 merges: `src/panel.rs` is Phase 0's).

## Phase 2: storms and weather as 2D fields (published approach: Schneider, SIGGRAPH 2022 course on 2D cloud fields)
Done when: storm cells come from map fields, lifecycle runs on the clock, and the renders read as cumulonimbus, not capsules.
- The weather map drives coverage, precipitation and cloud type, and **advects with the wind** (the panel map then moves with the
  weather; today it is static by design). Storm cells are fields painted into it, not analytic capsules (those read as mushrooms).
- Cumulonimbus: towers to the tropopause (default 14-16 km), anvils that spread and thin downwind, overshooting tops, a shelf
  cloud on the gust front.
- Dark bases: ambient occluded by the column above; ground bounce shadowed by the storm.
- Lifecycle on the clock (developing, mature, dissipating), squall lines and multicell systems; seeded timetable plus authored storms.
- Weather states over time (calm clear nights, fronts, showers) so dawn fog appears without forcing.
- Art-directed horizon: some distant drama always, whatever the local map says. Sky scale real by default, cloud base per region.

## Phase 3: towers from smooth fields
- Cumulus congestus grows from smooth 2D height and strength fields, not squashed noise: no slices, no streaks.

## Phase 4: precipitation
- Rain curtains and shafts under cells, virga with high bases, rain intensity from the map.
- Wet ground, puddles and ripples; snow and hail optional. Cloud shadows and precipitation darkening under cells.

## Phase 5: wind
- Gusts and gust fronts from storm outflow (`storm_fx::outflow_at` exists in core, unwired), dust and haboob wall option; wind
  drives clouds, rain angle, fog drift and audio.

## Phase 6: lightning and thunder
- Cloud-to-ground and in-cloud flashes lighting the clouds from inside (the flash schedule exists in core, unrendered).
- Thunder delayed by distance; events and audio cues for the host. The wind/rain audio mapping (`core::audio`) becomes host events.

## Phase 7: day and night
- Night sky (moon with inclination, stars, airglow), twilight colours, clouds and fog lit by the moon with terrain shadow.

## Later
- Seasons and climate presets; host terrain hooks (orographic rain is in core, unwired).
- A performance scaler that keeps quality inside the budget (update div, steps, render scale).

## Order and ownership
Phase 0 first. Phase 1 runs in parallel with Phase 0's tail, then before Phase 2, so cost and coverage regressions are caught
while storms are built. After Phase 2, phases run in parallel sessions, one owner per file.

| Owner | Files |
|---|---|
| Phase 0 (fog) | `src/fog.rs`, `src/fog/`, `src/shaders/fog.wesl`, `crates/nephthene_core/src/fog.rs`, `src/panel.rs`, `src/sky.rs`, `tests/fog.rs`, `docs/fog.md` |
| Phase 1 (gates) | `src/clouds/render.rs` (timing only), `src/rain.rs` (timing only), `tests/clouds.rs`, `tests/golden.rs`, `tests/golden/`, new `tests/*.rs`, `tools/` |
| Shared, append-only | `crates/nephthene_core/src/config.rs` (own section only), `examples/poses.json` (new poses only), `docs/bugs.md`, `docs/decisions.md`, `README.md` |
| Ask first | `examples/sky.rs`, `src/lib.rs`, shaders other than your own |
