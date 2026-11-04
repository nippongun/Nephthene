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

- `evolve`: amount 0..1 per game hour. Formation = `form_per_h` x saturation (1 - smoothstep(`spread_full_c`, `spread_none_c`, air
  temperature - dew point), see docs/air.md) x calm x (1 - amount); the red arm `private_cooling` keeps the old formation (night by sun
  elevation `night_lo_deg..night_hi_deg` x (1 - cover_block x cover) x calm x moisture). Loss = `burn_per_h` x sun height factor x
  `lit` x (1 - cover_shade x cover) + `mix_per_h` x windiness.
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
The march runs at `render_scale` (default 0.25) into a fog target and a surface-distance target (the scene depth stops each ray at the
first surface); a `resolve` pass accumulates the march over frames at march resolution: the new march blended at `temporal_alpha` into last frame's resolved fog, reprojected through last frame's view at the texel's surface distance (`max_m` for sky) and clamped to the new march's neighbourhood on the same surface (3x3 while the view moves, 5x5 when still); the first frame, a resize, a skipped frame or a camera cut (`resolve.cut_deg`, `resolve.cut_m`) resets it. A full-resolution composite upsamples the resolved fog depth-aware.
The ray is clipped to the altitude band holding fog and marched in `steps` jittered steps. Light per sample: sun x atmosphere
transmittance x cloud shadow cookie x terrain shadow (`sun_alt`) x multiple-scattering octaves over the optical depth to the pool top;
moon the same without terrain shadow; sky light = mean sky-view radiance over the hemisphere x `ambient_scale`, half from above
(dimmed by `ambient_od` x optical depth above) and half reflected from below (two-stream: the fog layer under the sample over ground of
`ground_albedo`). Aerial perspective as the rain shafts.

Noise comes from a baked 64^3 tiling lattice texture (`nephthene_core::fog::noise_lattice`): one trilinear fetch per lookup.
`ground_fog.hash_noise` restores the old per-step hashing (about 70 hashes per step at full resolution), a test switch for the cost
gate. Timing: `clouds.pass_timing` prints `FOGMS` (march = march + resolve, composite and total; mean of 60 frames, ms).

## Knobs

Look and cost knobs (all defaults untuned):

- `top_billow_m`, `billow_tile_m`: the pool top billows over `billow_tile_m`, lit and self-shadowed by the local top; 0 = flat.
- `top_fade_m`: haze above the pool top, e-folding over this height; with `wisp_m` 0, the hard top.
- `wisp_m`, `wisp_erode`: the pool top within `wisp_m` below it is eroded by the breakup noise, up to `wisp_erode` at the top; `wisp_m` 0 = the hard top.
- `diffuse_sun`: the sun's two-stream diffuse light inside the fog; 0 = direct beam only.
- `cloud_diffuse`, `ignore_cloud_sky`: cloud-covered sky light moves toward the sun's diffuse light through cloud; `ignore_cloud_sky` = clear-sky light (red arm).
- `step_power`: march step spacing (i/steps)^power, dense near the eye; 1 = uniform.
- `render_scale`: march resolution as a share of the view (default 0.25 = a sixteenth of the pixels); 1 = full resolution.
- `temporal_alpha`: share of each new march blended into the reprojected history (0.1); 1 = no history (G21 red arm).
- `still_px`: reprojection motion (march texels) below which the history clamp box is 5x5 instead of 3x3 (0.5).
- `hash_noise`: test switch, per-step hashed noise (cost gate red arm).
- `sky.sun_occlusion`, `sky.occlusion_keep`, `sky.occlusion_steps`: with terrain, the share of a view ray in the terrain's shadow loses its aerial sun in-scatter except `occlusion_keep` (0.2), sampled at `occlusion_steps` (8); `sun_occlusion` false = Bevy's own (red arm).
- `sky.cloud_shafts` (true): the cloud shadow cookie shadows the same view ray as the terrain does, below `clouds.bottom_m`, so air under cloud gaps stays lit and air under clouds loses its sun in-scatter (physically right, but too faint to read as rays: the visible god rays are the screen-space pass below); sky pixels use a ray of `clouds.shadow_half_m`. Needs `sun_occlusion` and the fog pass running (ground fog on, or terrain). false = terrain only (red arm of G34). The correction is weighted by the share of the pixel the clouds leave uncovered (1 - cloud alpha): cloud colour has its own air, not Bevy's. `sky.cloud_shafts_ignore_clouds` true drops that weight (red arm).
- `rays` (src/rays.rs, shaders/rays.wesl): screen-space god rays (GPU Gems 3, ch. 13). `source` box-averages the scene's HDR light above `rays.threshold` (0.8), capped at `rays.source_max` (2.0), at `rays.source_scale` (1/8); `march` walks each pixel toward the sun's screen position through it (`samples` 64, `decay` 0.97, Gaussian mask `radius` 0.35 of the screen height) at `render_scale` 0.5; `composite` adds it times `strength` (12) to the scene. Fades out as the sun leaves the screen (`edge_fade`), goes behind the camera, or sets. Beams show where bright sky gaps sit next to cloud edges near the sun; a sun inside an even bright cloud gives a glow. Gate G35 (red arms: `rays.enabled=false`, `rays.flat_source=true`). All values untuned. In a game: on by default with `NephthenePlugin`, no extra component; it runs on every 3D camera (each `Core3d` view with a `ViewTarget`), so a second 3D camera (mirror, map) gets rays too; turn off with `rays.enabled=false`. It needs the `NephtheneSun` entity, which the plugin spawns.

## Why no fog by default

The formation model forms none above `wind_hi_mps` (6 m/s), and the default wind is 6 m/s: radiation fog needs light wind. At 2.5 m/s
or less the model reaches about 0.73 by 5 h on a clear night. To see fog, use the panel's wind speed or Fog controls, or a calm weather state.

## Verify locally

```
cargo test --release -p nephthene_core fog                                   # model (also runs in `cargo test --release`)
cargo run --release --example sky -- --shot fog_valley --terrain --hour 5.6 --out shots --set ground_fog.enabled=true --set wind.speed_mps=1.0
cargo run --release --example sky -- --shot fog_floor  --terrain --hour 6.0 --out shots --set ground_fog.enabled=true --set wind.speed_mps=1.0
cargo run --release --example sky -- --shot fog_basin  --terrain --hour 6.5 --out shots --set ground_fog.enabled=true --set wind.speed_mps=1.0
cargo test --release --test fog -- --ignored --nocapture --test-threads=1   # G15-G21
```

The default wind (6 m/s) is too strong for radiation fog; the model correctly forms none. Use `wind.speed_mps` <= 2.5, or
`ground_fog.force`. Interactive: `cargo run --release --example sky -- --terrain --set ground_fog.enabled=true --set wind.speed_mps=1`.

Gates (`tests/fog.rs`, manual):

- G15 fog follows the terrain (red arm `ground_fog.ignore_terrain`).
- G16 a mountain's shadow holds fog (red arm `ground_fog.ignore_shadow`).
- G17 terrain hides the sun glow (red arm `sky.sun_occlusion=false`).
- G18 the pool top is soft (red arm `ground_fog.wisp_m=0` + `top_fade_m=0`).
- G19 cost at most 1.5 ms per frame at 1080p on the reference GPU (red arm `render_scale=1` + `hash_noise=true`); skips on software renderers.
- G21 a still view shows no march grain: frame-to-frame flicker at render_scale 1 at most 0.05% of pixels (red arm `ground_fog.temporal_alpha=1`).

Thresholds are budgets with the reason in the test doc comments.

## Known limits / next steps

- Pool floor is a square min filter: blind to drainage outlets (a valley open downstream pools like a closed basin). Upgrade: a
  priority-flood depression fill on the field grid.
- No fog beyond `extent_m` / 2 (fades over `edge_fade`); far valleys rely on `DistanceFog`. Upgrade: a coarse far field.
- No bounce from sunlit slopes into shaded fog.
- Moonlight has no terrain shadow; trees and buildings neither shadow the fog nor drain it (terrain only).
- The march is quarter resolution (`render_scale`): the temporal resolve removes the jitter grain but not the resolution, so the pool top shows 4 px steps up close. Upgrade: jitter the march's sub-texel position per frame and accumulate at full resolution (the clouds' update division).
- The burn-off lift opens a clear gap under the pool (`lift_max_m` x sun factor x (1 - amount): 8 m at a forced 0.9 at 8 h). A camera inside the gap (pose fog_floor) sees along it to the bright horizon at the valley's end: a thin bright seam. Physically a lifting fog base does this; whether the lift should open at amounts near 1 is a look call.
- Formation uses one regional amount history per texel with camera-local weather (`Weather.here`); weather map variation across the
  field is not used.
- Advection and steam fog (over water) are not modelled.
