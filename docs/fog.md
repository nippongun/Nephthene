# Ground fog

Terrain-following radiation fog: cold-air pools that fill valleys and basins under a level top, shallow mist in hollows, formation on
clear calm nights, burn-off where (and only where) the sun reaches the fog. Off by default (`ground_fog.enabled`).

Code: model `crates/nephthene_core/src/fog.rs`, config `GroundFog` (`crates/nephthene_core/src/config.rs`), plugin `src/fog.rs` +
`src/fog/render.rs`, shader `src/shaders/fog.wesl`, gates `tests/fog.rs`, test terrain `nephthene_core::fog::test_valley` (example
`--terrain`).

## Physical basis

| Behaviour | Field value used as the starting point (all defaults UNTUNED) | Source |
|---|---|---|
| Forms on clear, calm nights by radiative cooling; cloud cover blocks the cooling | `cover_block` 0.8 | NWS fog guide, weather.gov/media/zhu/ZHU_Training_Page/fog_stuff/fog_guide/fog.pdf |
| Light wind needed; stronger wind mixes it out or lifts it to stratus | full below 2.5 m/s, none above 6 m/s | NWS snippets; threshold unsourced (textbook 5-8 kt) |
| Wet ground (after rain) and humid air favour it; dry, dusty air does not | `moisture_base`, `wet_gain`, `dust_dry` | NWS snippets |
| Cold air drains downslope and pools in valleys and basins; the pool is capped by an inversion, so its top is level | pool floor = lowest ground within `pool_radius_m` 2.5 km | en.wikipedia.org/wiki/Cold-air_pool; mountwashington.org cold-air pool study |
| Depth: radiation fog 10-300 m, valley fog can exceed 450 m | `depth_flat_m` 25, `depth_max_m` 160 | encyclopedia.com (fog); snippets |
| Burn-off after sunrise takes hours, ~3 h for 300 m of fog; often lifts into stratus first | `burn_per_h` 1.2 (full fog clears 3-4 h after sunrise in the sun), `lift_max_m` 80 | NWS forecasting notes (snippet); Bergot et al. 2015 QJRMS |
| Burn-off needs sunshine on the fog: terrain shadow delays it | per-texel burn gated by the baked sunlit altitude | radiative heating mechanism (Bergot et al. 2015) |
| Visibility V = 3.912 / extinction (Koschmieder, 2% contrast) | pool 150 m, mist 400 m | WMO definition via US patent 10672282 |
| Droplets: single-scattering albedo ~1, asymmetry g 0.7-0.85; multiple scattering makes it white and diffuse | `albedo` 0.99, `g` 0.75, 3 scattering octaves | DTIC ADA330527 |

Research was done from search snippets (page fetches are blocked in the build environment); every number above is a starting point to
check, not a measurement.

## Model (nephthene_core::fog)

- `evolve`: amount 0..1 per game hour. Formation = `form_per_h` x night (sun below `night_lo_deg..night_hi_deg`) x (1 - cover_block x
  cover) x calm x moisture x (1 - amount). Loss = `burn_per_h` x sun height factor x `lit` x (1 - cover_shade x cover) + `mix_per_h` x
  windiness.
- `field` (terrain only, once per camera position): ground on a `bake_px`^2 grid over `extent_m`; pool floor = square min filter over
  `pool_radius_m`, carrying the depth of that lowest point (depth grows with how enclosed it is: mean ground around minus its ground,
  up to `relief_m`); floor and depth averaged so the top is level. Shallow mist strength from hollowness (`hollow_radius_m`,
  `hollow_m`), drained on slopes (`slope_lo..hi`), `flat_share` on open flats.
- `sun_alt`: per texel, the altitude it must reach to see over the terrain toward the sun (march `shadow_range_m` in `shadow_step_m`).
- Per-texel amounts: each texel evolves on its own, `lit` from its fog top against its sunlit altitude, so a valley in a mountain's
  shadow keeps its fog until the sun clears the ridge. `spin_up_local` runs the last `spin_up_h` hours of this weather (frozen clock,
  first frame); a running clock steps the amounts every `rebake_s`.
- `map`: per texel (pool top, mist strength, sunlit altitude, amount); `density` is the vertical profile the shader mirrors: a soft
  level top, thinning at shallow margins, exponential mist, base lifted by burn-off only where the top is sunlit.

## Rendering (fog.wesl)

Full-screen pass after the main pass (where Bevy's volumetric fog runs), premultiplied over the view target, reading the scene depth.
The ray is clipped to the altitude band holding fog and marched in `steps` jittered steps. Light per sample: sun x atmosphere
transmittance x cloud shadow cookie x terrain shadow (`sun_alt`) x multiple-scattering octaves over the optical depth to the pool top;
moon the same without terrain shadow; sky light = mean sky-view radiance over the hemisphere x `ambient_scale`, half from above
(dimmed by `ambient_od` x optical depth above) and half reflected from below (two-stream: the fog layer under the sample over ground of
`ground_albedo`). Aerial perspective as the rain shafts.

## Verify locally

```
cargo test --release -p nephthene_core fog                                   # model (also runs in `cargo test --release`)
cargo run --release --example sky -- --shot fog_valley --terrain --hour 5.6 --out shots --set ground_fog.enabled=true --set wind.speed_mps=1.0
cargo run --release --example sky -- --shot fog_floor  --terrain --hour 6.0 --out shots --set ground_fog.enabled=true --set wind.speed_mps=1.0
cargo run --release --example sky -- --shot fog_basin  --terrain --hour 6.5 --out shots --set ground_fog.enabled=true --set wind.speed_mps=1.0
cargo test --release --test fog -- --ignored --nocapture --test-threads=1   # G15, G16
```

The default wind (6 m/s) is too strong for radiation fog; the model correctly forms none. Use `wind.speed_mps` <= 2.5, or
`ground_fog.force`. Interactive: `cargo run --release --example sky -- --terrain --set ground_fog.enabled=true --set wind.speed_mps=1`.

Gates: G15 (fog follows terrain; red arm `ground_fog.ignore_terrain`), G16 (mountain shadow holds fog; red arm
`ground_fog.ignore_shadow`). Thresholds are budgets with the reason in the test doc comments.

## Known limits / next steps

- Pool floor is a square min filter: blind to drainage outlets (a valley open downstream pools like a closed basin). Upgrade: a
  priority-flood depression fill on the field grid.
- No fog beyond `extent_m` / 2 (fades over `edge_fade`); far valleys rely on `DistanceFog`. Upgrade: a coarse far field.
- No bounce from sunlit slopes into shaded fog.
- Moonlight has no terrain shadow; trees and buildings neither shadow the fog nor drain it (terrain only).
- The march is full resolution with per-frame jitter and no temporal resolve; thin mist seen at grazing angles can be noisy.
  Upgrade: half resolution with the clouds' resolve.
- Formation uses one regional amount history per texel with camera-local weather (`Weather.here`); weather map variation across the
  field is not used.
- Advection and steam fog (over water) are not modelled.
