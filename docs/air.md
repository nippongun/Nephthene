# Air state

A temperature field over the weather map (`air::Field`), and the air read from it at any place and altitude: temperature, dew point
and humidity, freezing level, snow line and precipitation type. The pure maths is `nephthene_core::air` (config `air`). The plugin
keeps the field as `Weather.air_field` and the air at the ground under the camera as `Weather.air`. `sky::tick` integrates the field
on the game clock, so it follows the sun, the season and the cloud cover over hours. The live field steps once `air.step_h` game hours (default one game minute) have gathered: one step of the 32 x 32 field costs about 50-80 us of CPU, a spin-up about 2 ms. When the clock is frozen
(`clock.minutes_per_second` 0) it is spun up for the hour instead, again only when the hour or the `air` or `clock` config changes.
The panel's Here section shows the air under the camera (`air ... precip rain|sleet|snow`).

## The field

- `air.field_px` x `air.field_px` cells (default 32: 6.25 km on the built-in 200 km map) over the weather map's extent, in map
  coordinates. It is in the map's frame, so it moves with the weather (`map.advect`, `Weather.map_shift`); read it at `map_xz`. It
  wraps when the map wraps.
- Each cell runs the force-restore model below under the map's cloud cover at its centre.
- A cell holds the temperature reduced to `ref_alt_m`. The altitude of the place read applies the lapse rates there:
  `Field::state(cfg, climate, shift, x, z, y, wetness, dust)`. The plugin reads it at the `Ground` height under the camera (0 m without a `Ground`). Air carried
  over a ridge therefore reads colder on the ridge, not with the valley's temperature.
- Each cell restores to the climate under it: the climate map sets its restore target, its dew point and its sun heating (`docs/climate.md`).

## Model

Surface temperature T (deg C) is a force-restore model (Deardorff 1978), integrated forward in game hours:

    dT/dt = heat_k_per_h x max(0, sin(sun elevation)) x (1 - cloud_shade x cover)
          - cool_k_per_h x (1 - cloud_block x cover)
          - restore_per_h x (T - season mean)

- Season mean = mean_c + season_c x sign(latitude) x declination(day - lag_days) / axial tilt. The equator takes the north's season.
- Air mass dew point = season mean - dew_below_c + wet_gain_c x wetness - dust_dry_c x dust, never above T.
- Relative humidity by Magnus (Alduchov & Eskridge 1996), over water at every temperature.
- Temperature at altitude y: T(y) = T - lapse_k_per_km x (y - ref_alt_m) / 1000. Dew point at y: Td - dew_lapse_k_per_km x
  (y - ref_alt_m) / 1000, never above T(y).
- Freezing level = ref_alt_m + 1000 x T / lapse_k_per_km. Snow line = freezing level - melt_m.
- Precipitation type at y: precip_kind = 1 - smoothstep(snow_c, rain_c, T(y)): 0 rain, 1 snow, sleet between. The panel calls it
  rain at or under 0.1 and snow at or over 0.9.

Spin-up starts from the season mean about `spin_up_h` hours before the hour and integrates in 0.25 h steps.

## Research

| claim | default | source |
|---|---|---|
| Clear-sky land diurnal range 8-15 K; overcast a third to a half of it | heat 4, cool 1.5, restore 0.25, cloud_shade 0.7, cloud_block 0.8: about 10.9 K clear and 4.3 K overcast at 35 N in July (printed by the gate) | Dai, Trenberth & Karl 1999 |
| Minimum near sunrise, maximum mid-afternoon | the same | observed land cycle; no citation |
| Standard lapse rate 6.5 K/km | `lapse_k_per_km` 6.5 | ICAO standard atmosphere |
| Snow reaches about 300 m below the freezing level (melting layer) | `melt_m` 300 | melting layer; no citation |
| Radiation fog forms when the dew point spread falls to about 0.5-2.5 K | `ground_fog.spread_full_c` 0.5, `spread_none_c` 2.5 | no citation; threshold to check |
| Seasonal lag about a month over land | `lag_days` 30 | surface heat storage; no citation |
| Dew point falls about 1.8 K/km in mixed air (with the dry adiabat, the 125 m per K of spread rule for the cloud base) | `dew_lapse_k_per_km` 1.8 | Espy's cloud base rule |
| Rain and snow equally likely at -0.4 to 2.4 C air temperature over Northern Hemisphere land, mean 1.0 C | `snow_c` -0.5, `rain_c` 2.5 (midpoint 1.0 C) | Jennings, Winchell, Livneh & Molotch 2018 |
| Field resolution | `field_px` 32 | none: a cost and detail choice, untuned |
| Field step cadence | `step_h` 1/60 h | none: a cost choice (measured 48-79 us per step at `field_px` 32, release build), untuned |

Every default is UNTUNED. Each is a starting value from the source in its row, not a fit to a measurement.

## Ground fog on the air

Fog formation reads the air's dew point spread (air temperature - dew point) instead of its own night cooling:

    formation = form_per_h x (1 - smoothstep(spread_full_c, spread_none_c, T - Td)) x calm x (1 - amount)

Calm = 1 - smoothstep(wind_lo_mps, wind_hi_mps, wind). Cloud cover now acts through the temperature (clouds block the cooling in
`air`); moisture acts through the dew point (wetness and dust in the air mass dew point). Burn-off is unchanged. Fog spin-ups
integrate the air temperature alongside the fog, so each step's spread comes from the same air model.

Red arm `ground_fog.private_cooling` keeps the old formula: night by sun elevation (`night_lo_deg..night_hi_deg`) x (1 - cover_block x
cover) x calm x moisture. `cover_block`, `moisture_base`, `wet_gain`, `dust_dry` and the night band apply only to that arm.

## Gates

Core unit tests in `air::tests` (run by `cargo test --release`). Budgets are in the test comments.

- `the_day_warms_after_noon_and_cools_to_sunrise`: coldest hour within 1.5 h of sunrise, warmest 13-17 h, daily range 6-18 K.
  Red arm `heat_k_per_h` 0.
- `clouds_shrink_the_daily_range`: overcast range under half the clear range. Red arm `cloud_shade` 0 and `cloud_block` 0.
- `winter_is_colder_and_the_south_flips_it`: summer minus winter mean over 10 K each, north and south. Red arm `season_c` 0.
- `humidity_and_altitude`: rh(20 C, 10 C dew) = 52.5 % within 1 %; saturated air is 100 %; wet ground saturates the air; 15 C at 1 km
  is 8.5 C; the snow line is 300 m under the freezing level; without a lapse rate the freezing level is infinite. No red arm yet.
- `the_field_follows_the_cover_under_it`: a map overcast in the west half and clear in the east. Each cell equals the single-place
  model under its own cover (1e-4 K), and the overcast half's diurnal range is under half the clear half's. Red arm `cloud_shade` 0
  and `cloud_block` 0.
- `rain_in_the_valley_snow_on_the_ridge` (gate C5-place): one place in mid-January at 35 N, 14 h, read at 0 m and at 3000 m, gives
  rain (precip_kind <= 0.1) and snow (>= 0.9). Red arm `lapse_k_per_km` 0.
- `fog::tests::fog_waits_for_the_dew_point` (in `nephthene_core::fog`): fog is visibility under 1 km (WMO). Dawn visibility is under
  1 km in the humid default air mass (dew point 4 K under the mean) and at least 1 km in a dry one (12 K under). Red arm
  `ground_fog.private_cooling` fogs the dry air mass.

## Limits

- No latent heat: a saturated night keeps cooling at the clear rate.
- One linear lapse rate, no inversion near the ground.
- The equator takes the north's season (signum of 0 is 1); set `season_c` near 0 in the tropics.
- Wind does not mix the surface temperature yet. Fog still uses wind for mixing.
- One sun for the whole field (a 200 km map spans about 2 degrees of sun).
- The dew point comes from the climate at the place read (humidity, ground wetness); the rain's ground wetness and the dust are the
  camera's.
- Under a frozen clock, a host edit of `Weather.map` reaches the field only when the hour or the config changes.
- Fog formation reads the air at `ref_alt_m` (its spin-ups integrate there), not at the pool floor.
- Saturation is over water at every temperature (no frost point below 0 C).
- No hail (convective; Phase 3 storm fields). Rain particles do not read `precip_kind` yet (Phase 4).
