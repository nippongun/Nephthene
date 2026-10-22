# Weather map

The 2D field all the weather is read from: R coverage, G precipitation, B cloud type, A dust, over a world-space rectangle. The
clouds (march, shadow cookie, rain shafts) sample it on the GPU; the camera's local weather (`Weather.here`, which drives rain, fog
formation and the dust fog) samples the same image on the CPU. With `map.advect` above 0 the map moves downwind with the mean wind.
Advection is on by default (`map.advect` 1, with `map.wrap`; owner, 2026-10-22); `map.advect` 0 keeps a fixed map.

Code: map and sampling `crates/nephthene_core/src/lib.rs` (`WeatherMap`, `map_xz`, `advect`), config `MapCfg`
(`crates/nephthene_core/src/config.rs`), shift state `Weather.map_shift` (`src/sky.rs`), GPU feed `src/clouds.rs` (`drive`),
sampler `src/clouds/render.rs`, panel marker `src/panel.rs` + `dash::map_uv`, gates `map_tests` (core) and G22 (`tests/weather.rs`).

## Frames

- **World**: Bevy XZ, metres; north = +Z.
- **Map**: world + `map.world_offset_m` - `Weather.map_shift`. Rows run north to south (`max_z` is row 0).
- `map_shift` is how far the weather has moved downwind since the map was laid down. It starts at `map.start_shift_m`, so a host
  that saves a game stores `Weather.map_shift` and restores it there.
- The GPU gets the same frame through the `world_offset` uniform (`world_offset_m - map_shift`), so the shaders and the cloud
  uniform struct are unchanged. Every consumer reads the one `Weather.map_shift`: `here` (in `tick`) and the clouds (in
  `clouds::drive`, Drive set) see the same shift in a frame.

## Advection

| Behaviour | Model | Source |
|---|---|---|
| Weather systems are carried by the wind | the map moves at `map.advect` x the mean wind | Nubis 2015 prototype; Schneider 2022 scrolls the cloud field along a 2D wind direction (`scroll_direction_2D * dt`) |
| A uniform wind moves a field without changing its shape | translation, not resampling: `shift += advect x wind x dt` (`core::advect`) | Advection equation with constant velocity: the exact solution is a translation |
| Gusts do not move weather systems | the shift follows the mean wind (`core::map_wind`): the gusting wind's direction (gusts only scale its speed while `wind.gust` x the gust weights stays under 1) at `Weather.wind_mean_mps`, so the panel's wind override counts | Same rule as the rain shafts (`shafts.gust_lean` gate) |
| Weather moves on the weather clock | game seconds (`dt x clock rate x 3600`), so a front crosses the map in game hours whatever the time-lapse; a paused clock (headless shots) freezes it. The cloud *noise* drift stays on real time, so detail motion reads as wind on screen. | Storms already run on game time (`storms.clock_scale`) |
| Art-directed skies want fixed weather | `map.advect` 0 (the default until 2026-10-22): only the noise drifts through a fixed coverage | Schneider 2017: "while the noise in the density model is animated, the coverage signal is not ... we only want them to appear to be changing without altering the larger structure of the cloudscape" |

`map.advect` 1 is physical (the weather rides the wind at its speed). Values between 0 and 1 are an art call, as are values above 1
for a faster sky. The value is the owner's call (look and feel); the default keeps the old look.

## Edges and wrap

An advected map runs out: after `size / (advect x wind)` game seconds the camera sees the upwind edge texels held forever (default
map 200 km at 6 m/s: the edge reaches a camera at the centre after about 4.6 game hours). `map.wrap` repeats the map instead. Frostbite works this way:
"That texture is scaled and repeated all over the world if necessary" (Hillaire 2016). With wrap:

- The CPU bilinear read (`WeatherMap::rgba`) and the GPU sampler (`AddressMode::Repeat`) both wrap. The CPU wrap is unit-gated;
  CPU/GPU agreement at the seam is built, not render-gated (G22 runs unwrapped).
- The shift is kept within one map period (`core::advect`), so f32 keeps its precision over long sessions.
- `clouds.background` (the weather outside the map) never applies, so the plugin refuses the pair at start.
- The map must tile. The built-in procedural map (`map.default_*`, a product of two sines) tiles while `map.default_cycles` are
  whole numbers (the default [1, 1]; gate `built_in_map_tiles`, red arm the old 1.13 and 0.84 cycles). A PNG must tile itself.
- The panel folds the camera marker back onto the map (`dash::map_uv`).

## Gates

| Gate | Claim | Red arm | Budget |
|---|---|---|---|
| `map_tests::map_rides_the_mean_wind` (core, automatic) | After 600 game seconds of gusting wind, taken through the plugin's own path (`map_wind`), the weather seen at a point is seen where the mean wind carried it (a sign flip in `map_wind` fails it) | `map.advect` 0 | 1 m (one texel of the built-in map is 3.1 km) |
| `map_tests::wrap_tiles_the_map` (core, automatic) | One period away reads the same weather; the seam reads the mean of the edge texels; a wrapped shift stays in one period | `map.wrap` false | 1e-4 (f32 rounding) |
| G22 (`tests/weather.rs`, manual render gate) | The clouds and the weather here ride the same shifted map: shifting the map by S renders the same sky as moving the world offset by -S, and both differ from the unshifted sky | `map.static_gpu` (the clouds keep the unshifted map) | shift vs offset <= run-to-run noise + 0.5 levels; shift vs unshifted >= 5 levels (a visible cover change); CPU coverage equal within print rounding |

G22 on lavapipe (2026-10-22, sky_up 480x270, 14 h, shift (67, 36) km into the built-in map's clear trough):
- Run-to-run noise was 0.453 levels, and shift vs offset measured 0.301, inside the 0.953 budget.
- Shift vs unshifted measured 12.77 levels, above the 5-level minimum.
- The red arm (shift with `static_gpu`) read 12.40 levels from the offset sky, so the gate catches a static GPU map.
- CPU coverage at the camera fell from 0.532 unshifted to 0.251, the same in the shift, offset and red-arm runs.

Metal run pending.

## Open

- Storm genesis (`storms.rs`, the seeded timetable) still samples the unshifted map. Sampling the shifted one would make storms
  appear and vanish at each timetable rebuild, because the shift is integrated state, not a function of storm time. Phase 2 paints
  storms into the map as fields, and then they advect with it. See docs/bugs.md.
- The temporal cloud resolve reprojects history for camera motion only. Coverage moving at `advect x wind x clock rate` (90 m/s at
  the default 15x clock and 6 m/s) shifts cloud edges about 1 px per frame at 5 km. That is inside what the history clamp absorbs,
  but not measured. Nubis 2022 folds the scroll into the reprojection; that is the upgrade if the edges ghost.
- Weather states over time (fronts, calm nights) will write into the map. Advection only moves what is there.

## Sources

- A. Schneider, "Nubis: Authoring Real-Time Volumetric Cloudscapes with the Decima Engine", SIGGRAPH 2017 Advances in Real-Time
  Rendering: wind offset and skew, the coverage signal kept static for art direction, weather states and the scheduler.
  advances.realtimerendering.com/s2017
- A. Schneider, "Nubis, Evolved", SIGGRAPH 2022 Advances: 2D cloud fields, density scrolled along a 2D direction, the scroll folded into the
  reprojection. advances.realtimerendering.com/s2022
- S. Hillaire, "Physically Based Sky, Atmosphere and Cloud Rendering in Frostbite", SIGGRAPH 2016 PBS course: a world-space weather
  texture, scaled and repeated over the world. ea.com/frostbite
