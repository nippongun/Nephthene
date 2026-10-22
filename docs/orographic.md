# Orographic precipitation

Rain and cloud made by the terrain: air forced up a slope cools, condenses and rains on the windward side, and the lee stays drier.
The pure maths is `nephthene_core::orographic` (Smith & Barstad 2004, one FFT pass), config `orographic`; the plugin's `Orographic`
resource holds the latest tile. Slice 1 runs on the CPU only and is off by default.

## How it runs

- Bake: a tile of `tile_px` x `tile_px` cells of `cell_m` (default 128 x 200 m = 25.6 km) centred on the camera (snapped to whole
  cells), heights from `Ground`, for the mean wind and the air temperature at the camera. The first bake runs at once; later ones run
  in the background when the tile goes stale (`rebake_m`, `rebake_deg`, `rebake_frac`, `rebake_k`). With `enabled` false or no
  `Ground` there is no tile and no cost.
- Fixed to the terrain (world frame): the weather map moves with the wind, this field does not (`docs/decisions.md`: two fields).
- Combined at the camera (`orographic::combine`), before the storms': precipitation = max(map, tile), cover = min(1, map +
  `cover_weight` x tile). `Weather.here` then feeds rain, fog and the panel (the `terrain uplift` line). Shots print an
  `OROGRAPHIC` line per bake.
- Not yet: the cloud shader, the cloud shadow and the rain shafts read only the weather map (slice 2). Until then rain can fall
  where the clouds draw a clear sky, which is why it is off by default.

## Model

Smith & Barstad (2004), per wavenumber (k, l) with sigma = U k + V l:

    P^(k, l) = Cw i sigma h^ / ((1 - i m Hw)(1 + i sigma tau_c)(1 + i sigma tau_f)),   m^2 = (N_m^2 - sigma^2)(k^2 + l^2) / sigma^2

- Cw = rho_s Gamma_m / gamma (`uplift_sensitivity`): the saturation vapour density of the air at the camera times the moist
  adiabatic lapse rate over the environmental one (`air.lapse_k_per_km`), times `gain`. Warm air rains more over the same ridge.
- The output is kg/m^2/s; x 3600 it is mm/h, and `full_mm_h` maps it to the map's 0..1. The cloud field is the condensation source
  before fallout, mapped the same way.
- The tile's grid runs east (+x) by south (-z), so the wind's (u, v) is (x, -z).

## Research

| claim | default | source |
|---|---|---|
| Example parameters: U 15 m/s, N_m 0.005 1/s, H_w 2500 m, tau_c = tau_f = 1000 s | `n_moist` 0.005, `h_w` 2500, `tau_c` and `tau_f` 1000 | Smith & Barstad 2004, J. Atmos. Sci. 61:1377 |
| Published tau estimates run 200-2000 s; a Pacific Northwest study constrains them only to about 500-5000 s | same | studies of the model in J. Hydrometeorology (2005, 2011) |
| Cw = rho_s Gamma_m / gamma; at 10 C under 6.5 K/km about 7.6 g/m^3 | computed from the air | Smith & Barstad 2004 |
| Heavy rain is over 7.6 mm/h (0.30 in/h); moderate 2.5-7.6 mm/h | `full_mm_h` 7.6 | AMS Glossary of Meteorology, "Rain" |
| A 10 C, 10 m/s westerly over a 1 km ridge (sigma 3 km): precipitation 0.38 mm/h 4.5 km before the crest, 1.5 at 3 km, 3.4 at the crest, 2.6 at 3 km past, 1.2 at 4.5 km past, 0 at 6 km either side; the cloud source 9.2 mm/h 3 km before the crest and 0 at 3 km past | none | a scratch run of `compute` with these parameters |

Every default is UNTUNED.

## Gates

Core tests (run by `cargo test --release`). Budgets are in the test comments.

- `orographic::tests::rain_condenses_upwind_and_drifts_downwind`: a dry map (cover 0.5) and a 10 m/s wind at 10 C over a 1 km
  ridge (sigma 3 km). 3 km upwind of the crest it rains (precip at least `rain.precip_lo`; 0.19 measured) under added cloud (cover
  above the cover 3 km downwind, where the air descends), and 4.5 km downwind gets at least 1.5x the rain of 4.5 km upwind (the
  fallout drift; 0.156 vs 0.050 measured). Run for a westerly over a north-south ridge and a southerly over an east-west one: a
  flipped tile axis swaps upwind and downwind and fails (checked by flipping the wind mapping in `bake`). Red arm `gain` 0.
- `orographic::tests::uplift_sensitivity_follows_the_air`: Cw at 10 C is 6-9 g/m^3 (tables: 9.4 g/m^3 saturation vapour density x
  about 0.8), and warmer air gives more.
- `orographic::tests::tiles_combine_sample_and_go_stale`: the combination, nothing outside the tile, and the rebake rules.
- The existing `windward_gets_more_than_lee_and_it_flips`, `non_negative_and_degenerate_inputs_are_quiet` and `fft_round_trips`.

## Limits

- CPU only: clouds, cloud shadows and rain shafts do not show it yet (slice 2), so it is off by default.
- One tile around the camera, rebaked when stale: no cache per (tile, wind octant). A rebake is one FFT of a 256 x 256 padded square
  (twice the tile) in the background.
- Cw comes from the camera's air for the whole tile; one moist stability and one moist layer depth.
- Precipitation drifts downwind by about U x (tau_c + tau_f), 20 km at 10 m/s. A ridge a few km wide therefore spills most of its rain
  past the crest (the lee half of the 1 km test ridge gets 1.5x the windward half), as the theory says; only ranges wider than the
  drift keep it windward. The cap cloud (the condensation source) stays on the ascent.
- Linear theory: no blocking or flow splitting at low Froude numbers. Heights are sampled at `cell_m`, so ridges narrower than a
  few cells vanish.
