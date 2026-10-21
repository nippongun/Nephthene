# Nephthene

A weather system for [Bevy](https://bevy.org) 0.20: a clock that drives sun, moon, stars and exposure; a Bevy `Atmosphere` sky with
dust; HZD-style raymarched volumetric clouds with a temporal resolve; cloud shadows; GPU rain with splashes and wet ground; distant rain
shafts; optional storm cells; terrain-following ground fog (valley pools, shallow mist, burn-off); wind; a weather map; wind/rain audio mapping; and a mouse-driven weather panel.

Two crates:

| crate | what |
|---|---|
| `nephthene_core` (`crates/nephthene_core`) | Pure Rust, no Bevy: `Config`, clock/sun/moon, wind, weather map, ground fog, rain, storm timetable, audio mapping, panel state. Unit and integration tests. |
| `nephthene` (this package) | The Bevy plugin: render passes, shaders (embedded), systems, the panel. |

## Use

```rust
app.add_plugins(NephthenePlugin::default());                    // defaults
app.add_plugins(NephthenePlugin::new(Config::load(path)?));     // from JSON
```

The scene provides a camera with `AtmosphereSettings` (add `Exposure` / `DistanceFog` to have them driven) and one `DirectionalLight` with the
`NephtheneSun` marker. Mark a ground mesh `WetGround` to let rain wet it, insert `Ground(HeightGrid)` for terrain-aware rain. Read `Weather`
(hour, wind, local coverage/precip), `RainState`, `Storms`. Order your systems with `NephtheneSet::{Tick, Drive}`. Everything else is `Config`.

## Run

- Example (a window, your terminal only): `cargo run --release --example sky`. Hold the left mouse button and move to look; W/S forward and back, A/D strafe, E up, Q down, Left Shift 5x, wheel changes speed; `` ` `` opens the panel.
- Headless shot (never opens a window): `cargo run --release --example sky -- --shot sky_west --hour 17.5 --out shots`. Options are listed at the top of `examples/sky.rs`; `--set clouds.update_div=1` overrides any config key, `--config file.json` loads a file, `--print-config` prints the full reference (`config.reference.json`).
- Under an agent (`CLAUDECODE=1`) a windowed run refuses to start.

## Tests (all headless)

- `cargo test --release` runs: core unit and integration tests, the golden still gate (`tests/golden.rs`, goldens in `tests/golden/`, Apple Silicon/Metal), the metric self-test.
- Manual fog gates G15-G16 (terrain, mountain shadow): `cargo test --release --test fog -- --ignored --nocapture --test-threads=1`.
- Manual cloud gates G7-G14 (a few minutes of GPU): `cargo test --release --test clouds -- --ignored --nocapture --test-threads=1`. Each gate has a red arm made by a config switch, so a passing gate is proven able to fail.
- Regenerate goldens after an approved look change: `NEPHTHENE_UPDATE_GOLDEN=1 cargo test --release --test golden golden_cloud_sky`.
- Python metric tools: `tools/README.md`.

## Config

One struct, `Config` (`crates/nephthene_core/src/config.rs`), JSON, every field defaulted so a file names only what it changes. About 250 values. All defaults are
starting values and **untuned**. Sections:

| section | covers |
|---|---|
| `clock`, `exposure`, `daylight_ev100`, `ambient_day_brightness`, `fog_brightness` | time of day, latitude, exposure curve, ambient |
| `fog`, `moon`, `stars`, `sky`, `wind` | dust fog, moon, star field, Mie dust medium, `sky.msaa_fix`, wind and gusts |
| `map` | world offset of the weather map, built-in procedural map values |
| `clouds` (+ `look`, `noise`) | layer heights, march steps, light, shoulder, render scale, update div, shadow cookie, towers, probe view, coverage override |
| `resolve` | temporal resolve: reject gain, still box, fresh confidence, cut thresholds, jitter, the `wide_upsample` switch, and the test switches `no_reproject`, `light_jitter_shared`, `motion_half_px` |
| `shafts` | rain-shaft pass |
| `ground_fog` | terrain-following radiation fog: formation from night cooling, cloud cover, wind, wetness and dust; burn-off and base lift; valley pooling depth, shallow mist, optics, bake and march; test switches `ignore_terrain`, `ignore_shadow`; off by default via `enabled`. See `docs/fog.md` |
| `rain` | streaks, splashes, drop kinematics, wetness, rain light, `force` |
| `storms` (+ `render`, `fx`) | timetable, cell shape, shader constants, flashes, authored storms, `force` |
| `audio` | wind/rain bed mapping (the plugin does not play sound; use `core::audio` from your audio code) |
| `sim` | smoothing, `panel_open` |

## Notes

- Bevy 0.20 renders the sky; `sky.msaa_fix` swaps in a per-sample copy of Bevy's `render_sky.wesl` (guard tests in `src/sky/msaa.rs` fail when Bevy changes it).
- Linux needs the usual Bevy system packages (alsa, udev, wayland/x11); a GPU is required for the shots and gates.
