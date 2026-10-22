# Air state

The surface air at the camera: temperature, dew point and humidity, and temperature with altitude (freezing level, snow line). The
pure maths is `nephthene_core::air`; the plugin exposes the state as `Weather.air` (config `air`). `sky::tick` integrates the surface
temperature on the game clock, so it follows the sun, the season and the cloud cover over hours. When the clock is frozen
(`clock.minutes_per_second` 0) it is spun up for the hour instead.

## Model

Surface temperature T (deg C) is a force-restore model (Deardorff 1978), integrated forward in game hours:

    dT/dt = heat_k_per_h x max(0, sin(sun elevation)) x (1 - cloud_shade x cover)
          - cool_k_per_h x (1 - cloud_block x cover)
          - restore_per_h x (T - season mean)

- Season mean = mean_c + season_c x sign(latitude) x declination(day - lag_days) / axial tilt. The equator takes the north's season.
- Air mass dew point = season mean - dew_below_c + wet_gain_c x wetness - dust_dry_c x dust, never above T.
- Relative humidity by Magnus (Alduchov & Eskridge 1996), over water at every temperature.
- Temperature at altitude y: T(y) = T - lapse_k_per_km x (y - ref_alt_m) / 1000.
- Freezing level = ref_alt_m + 1000 x T / lapse_k_per_km. Snow line = freezing level - melt_m.

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
- `fog::tests::fog_waits_for_the_dew_point` (in `nephthene_core::fog`): fog is visibility under 1 km (WMO). Dawn visibility is under
  1 km in the humid default air mass (dew point 4 K under the mean) and at least 1 km in a dry one (12 K under). Red arm
  `ground_fog.private_cooling` fogs the dry air mass.

## Limits

- No latent heat: a saturated night keeps cooling at the clear rate.
- One linear lapse rate, no inversion near the ground.
- The equator takes the north's season (signum of 0 is 1); set `season_c` near 0 in the tropics.
- Wind does not mix the surface temperature yet. Fog still uses wind for mixing.
- The air is camera-local: one state, not a field. Dew point and temperature do not vary across the map.
- Saturation is over water at every temperature (no frost point below 0 C).
- Precipitation type is the next step. It reads `freezing_m` and `snow_line_m`.
