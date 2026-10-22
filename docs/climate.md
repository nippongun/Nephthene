# Climate

What a place is like, as opposed to what the weather is doing there. The pure maths is `nephthene_core::climate` (config `climate`).
The plugin keeps the map as `Weather.climate` and the climate at the camera as `Weather.climate_here`; the panel shows it under Here
(`climate ...`) and shots print a `CLIMATE` line.

## The map

- A second RGBA8 image in the weather map's format: `<dir>/climate_map.png` with `<dir>/climate_map.json` (the same keys as
  `weather_map.json`), loaded with `WeatherMap::load_named(dir, "climate_map")` and given as `NephthenePlugin { climate: Some(..), .. }`.
- Fixed to the world: read at world + `map.world_offset_m`, never shifted by `map.advect` (the weather moves over it). Held at its edges.
- R: mean temperature offset, signed. Byte 128 is 0, 0 is -`temp_range_k`, 255 is +`temp_range_k`.
- G: humidity, signed. 0 is full dryness (dew point depression `dry_dew_below_c`), 128 keeps `air.dew_below_c`, 255 is full humidity
  (`wet_dew_below_c`).
- B: ground snow 0..1 (albedo).
- A: ground wetness 0..1: raises the dew point by `air.wet_gain_c` x wetness, taking the wetter of this and the rain's.
- Without a map every place is `climate.uniform` = [offset K, humidity -1..1, snow 0..1, wetness 0..1], default all 0: the air as
  before. Try a hot desert with `--set climate.uniform=[10,-1,0,0]`.

## Model

Each place gets its own air config (`climate::air`), and the air model (`docs/air.md`) runs unchanged on it:

    mean_c       = air.mean_c + offset                                  (the target the place restores to)
    dew_below_c  = lerp(air.dew_below_c, dry_dew_below_c, -humidity)    (humidity < 0)
                 = lerp(air.dew_below_c, wet_dew_below_c, humidity)     (humidity >= 0)
    heat_k_per_h = air.heat_k_per_h x (1 - albedo) / (1 - bare_albedo),  albedo = lerp(bare_albedo, snow_albedo, snow)

The temperature field (`air::Field`) moves with the weather over the climate. Each step a cell restores toward the climate under it
now, so cold air carried over a warm region warms toward it. A neutral place leaves the air config exactly as it was.

## Research

| claim | default | source |
|---|---|---|
| Annual means of inhabited places span about -9 C (Yakutsk, -8.0 to -8.8 C) to 34.6 C (Dallol, 1960-66): -24 to +20 K around the default 15 C | `temp_range_k` 25 | Urban Resilience Hub and Wikipedia (Yakutsk); Wikipedia (Dallol) |
| Desert: Phoenix (Sky Harbor, 1992-2021) dew point about 3.9 C (39 F) and humidity 30 % year-round at an annual mean near 24 C, a depression of about 20 K; June afternoons about 10-14 % | `dry_dew_below_c` 20 | timeanddate.com (Sky Harbor climate); Encyclopedia.com and Arizona Highways (June afternoons) |
| Humid tropics: Singapore mean about 28 C with a dew point about 24.4 C (76 F), a depression of about 3.5 K; nights often saturated | `wet_dew_below_c` 3.5 | timeanddate.com and weatherspark.com (Paya Lebar 2012-2021) |
| Albedo: grass 0.2-0.25, bare soil 0.17, bare land 0.23 measured; fresh snow 0.8-0.9, old snow 0.46-0.70 | `bare_albedo` 0.2, `snow_albedo` 0.7 (aged snow, the kind that lies) | Wikipedia albedo table; University of Arizona NATS 101; Yu et al. 2010 (Xianghe) |
| Wet soil shrinks the diurnal range (heat capacity and conductivity rise with moisture) | not modelled: wetness acts through the dew point only | NASA technical reports 1976 and 1977 (NTRS 19760018548, 19770012620) |

Every default is UNTUNED. The default air mass (`air.dew_below_c` 4 K) is already near the humid end, so painting humidity matters
mostly toward dry.

## Gates

Core tests (run by `cargo test --release`). Budgets are in the test comments.

- `air::tests::a_painted_cold_place_is_colder`: a map painted -25 K in its west half; the daily mean there is 25 K under the east,
  within 0.25 K. Red arm `climate.ignore`.
- `air::tests::desert_air_is_dry_in_the_afternoon`: full dryness, 15 h in mid-July at 35 N: relative humidity at most 0.25 (Phoenix
  June afternoons about 10 %). Red arm `climate.ignore`.
- `air::tests::snow_cover_cuts_the_days_warming`: full snow cover: the diurnal range is at most half the bare one (snow absorbs 0.375
  of the sunlight). Red arm `climate.ignore`.
- `climate::tests`: bytes decode with 128 neutral, a neutral place keeps the air config exactly, the mappings, and `ignore`.
- `air::tests::the_field_follows_the_cover_under_it` still holds under the default (neutral) climate: each cell is the single-place
  model to 1e-4 K.

## Limits

- The season's swing (`air.season_c`) is global: continentality is not a channel (a second climate image is the upgrade).
- Wet ground does not shrink the diurnal range; it only raises the dew point.
- Snow does not hold the surface at 0 C while it melts, and snowfall does not add to it (Phase 4).
- The dew point belongs to the place read, not to the air: humidity does not move with the weather.
- The climate does not shape the weather map's coverage or precipitation yet, and there are no biome presets (Phase 1 preset library).
