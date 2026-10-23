# Nephthene

A weather system for [Bevy](https://bevy.org) 0.20: a clock that drives sun, moon, stars and exposure; a Bevy `Atmosphere` sky with
dust; HZD-style raymarched volumetric clouds with a temporal resolve; cloud shadows; GPU rain with splashes and wet ground; lightning (branched channels, flash light, strike and thunder messages); distant rain
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
`NephtheneSun` marker. Insert `Ground(HeightGrid)` for terrain-aware rain, mark roofs `Shelter`. Read `Weather`
(hour, wind, local coverage/precip), `RainState`, `Storms`, and the ground's wetness and snow at any place with the `SurfaceWeather` system param.
The plugin never touches your materials: build your own wet and snowy look from that state, or enable feature `surface` and add
`SurfaceMaterialPlugin` for a ready-made ground material (`SurfaceMaterial`, or the `WetGround` marker on a `StandardMaterial`); it works without `NephthenePlugin` too. Order your systems with `NephtheneSet::{Tick, Drive}`. Everything else is `Config`.

## Run

- Example (a window, your terminal only): `cargo run --release --example sky`. Hold the left mouse button and move to look; W/S forward and back, A/D strafe, E up, Q down, Left Shift 5x, wheel changes speed; `` ` `` opens the panel.
- Headless shot (never opens a window): `cargo run --release --example sky -- --shot sky_west --hour 17.5 --out shots`. Options are listed at the top of `examples/sky.rs`; `--set clouds.update_div=1` overrides any config key, `--config file.json` loads a file, `--print-config` prints the full reference (`config.reference.json`). `--terrain` loads the test valley and `--mountains` a mountain range (poses `mtn_west`, `mtn_east`, `mtn_high`); in the live window `--pose <name>` starts the camera at a pose.
- Under an agent (`CLAUDECODE=1`) a windowed run refuses to start.
- Without the panel (feature `panel`, default on): `cargo build --release --example sky --no-default-features`.

## Tests (all headless)

- `cargo test --release` runs: core unit and integration tests, the golden still gate (`tests/golden.rs`, goldens in `tests/golden/`, Apple Silicon/Metal), the metric self-test.
- Manual storm gates G25-G26 (a storm overhead rains on the camera, rain shafts under a storm): `cargo test --release --test storms -- --ignored --nocapture --test-threads=1`.
- Manual lightning gates G23-G24 (channel reads as a line, flash lights the ground): `cargo test --release --test lightning -- --ignored --nocapture --test-threads=1`.
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
| `map` | world offset of the weather map, built-in procedural map values (tileable: `default_cycles`, `default_phase`, `default_blob`); advection with the wind (`advect`, on by default), `wrap` (on), `start_shift_m`, test switch `static_gpu`. See `docs/weather_map.md` |
| `air` | the temperature field over the weather map (force-restore per cell on the clock: sun, season, latitude, the cell's cloud cover; moves with the map), dew point and humidity, lapse rates, freezing level, snow line, precipitation type (`snow_c`..`rain_c`); `Weather.air` is the air at the ground under the camera, `Weather.air_field` the field. See `docs/air.md` |
| `climate` | what a place is like: a second map fixed to the world (`climate_map.png`; R temperature offset, G humidity, B ground snow, A ground wetness) or `uniform` without one; it sets each place's restore target, dew point and snow albedo for the air; test switch `ignore`. See `docs/climate.md` |
| `orographic` | rain and cloud from the wind over the terrain (Smith & Barstad 2004): a tile fixed to the terrain, baked around the camera from `Ground`, combined with the weather map where it is read (precip max, cover weighted add); CPU only so far, off by default via `enabled`; red arm `gain` 0. See `docs/orographic.md` |
| `clouds` (+ `look`, `noise`) | layer heights, march steps, light, shoulder, render scale, update div, shadow cookie, towers, probe view, coverage override |
| `resolve` | temporal resolve: reject gain, still box, fresh confidence, cut thresholds, jitter, the `wide_upsample` switch, and the test switches `no_reproject`, `light_jitter_shared`, `motion_half_px` |
| `shafts` | rain-shaft pass |
| `ground_fog` | terrain-following radiation fog: formation from the air's dew point spread (`air`) and wind; burn-off and base lift; valley pooling depth, shallow mist, optics, bake and march (`render_scale`, half resolution by default, depth-aware composite); test switches `ignore_terrain`, `ignore_shadow`, `private_cooling`, `hash_noise`; on by default (`enabled`; the panel's Fog section switches it at runtime). See `docs/fog.md` |
| `rain` | streaks, splashes, drop kinematics, wetness, rain light, `force` |
| `storms` (+ `render`, `fx`, `lightning`) | on by default: timetable (`rate_per_km2_hr`: storms form occasionally), cell shape, shader constants, flash schedule, authored storms, `force`; `couple` (storms bring rain, cover, gust front and darker sky light to the camera, and rain shafts); `summon_*` ("Storm now": a mature cell upwind that drifts over you; `summon_at_start` for shots); `lightning` (on): strike, channel shape, return strokes, channel look, flash light, `force_age_s` |
| `audio` | wind/rain bed mapping (the plugin does not play sound; use `core::audio` from your audio code) |
| `sim` | smoothing, `panel_open` |

## Notes

- Bevy 0.20 renders the sky; `sky.msaa_fix` swaps in a per-sample copy of Bevy's `render_sky.wesl` (guard tests in `src/sky/msaa.rs` fail when Bevy changes it).
- Linux needs the usual Bevy system packages (alsa, udev, wayland/x11); a GPU is required for the shots and gates.
