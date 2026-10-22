//! The one configuration struct. Every tunable of the weather system lives here: clock, sun and moon, sky medium, fog, wind,
//! clouds (march, light, temporal resolve), cloud shadows, rain-shaft pass, rain FX, storms, audio mapping.
//!
//! Load from JSON with [`Config::load`] / [`Config::from_json`]; every field has a default, so a file only names what it changes.
//! `Config::default().to_json()` prints the full reference file. All defaults are starting values copied from the game this
//! library was extracted from. They are UNTUNED: a perceptual first guess, not a recommendation.
//!
//! Compass: north = +Z, east = +X, azimuth clockwise from north.
use serde::{Deserialize, Serialize};

use crate::{audio, storm::StormCfg};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    pub clock: Clock,
    /// (sun elevation deg, ev100), elevation ascending. Linear between points, clamped at the ends.
    pub exposure: Vec<[f32; 2]>,
    /// The ev100 that daylight-authored ambient values were set at; ambient scales by 2^(ev100 - this) below it.
    pub daylight_ev100: f32,
    /// Ambient light brightness (cd/m^2) by day, before the exposure scaling. Bevy's own default is 80.
    pub ambient_day_brightness: f32,
    /// (sun elevation deg, multiplier on the fog colours): DistanceFog colour is not exposure-scaled, so without this the fog glows at night.
    pub fog_brightness: Vec<[f32; 2]>,
    pub fog: Fog,
    pub moon: Moon,
    pub stars: Stars,
    pub sky: Sky,
    pub wind: Wind,
    pub map: MapCfg,
    pub clouds: Clouds,
    pub resolve: Resolve,
    pub shafts: Shafts,
    pub rain: RainCfg,
    pub storms: StormCfg,
    pub audio: audio::Cfg,
    /// Terrain-following radiation fog: valley pools under a flat top and shallow mist in hollows (`fog` above is the distance haze).
    pub ground_fog: GroundFog,
    /// Tuning of the ticking systems (smoothing, rebuild thresholds).
    pub sim: Sim,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            clock: Clock::default(),
            exposure: vec![[-14.0, -1.0], [-8.0, 2.5], [-4.0, 6.0], [0.0, 8.5], [6.0, 11.0], [42.0, 13.0], [90.0, 13.5]],
            daylight_ev100: 13.0,
            ambient_day_brightness: 80.0,
            fog_brightness: vec![[-14.0, 0.01], [-4.0, 0.05], [0.0, 0.35], [10.0, 0.9], [20.0, 1.0]],
            fog: Fog::default(), moon: Moon::default(), stars: Stars::default(), sky: Sky::default(), wind: Wind::default(),
            map: MapCfg::default(), clouds: Clouds::default(), resolve: Resolve::default(), shafts: Shafts::default(),
            rain: RainCfg::default(), storms: StormCfg::default(), audio: audio::Cfg::default(), ground_fog: GroundFog::default(), sim: Sim::default(),
        }
    }
}

impl Config {
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> { serde_json::from_str(s) }
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let s = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_json(&s).map_err(|e| format!("{}: {e}", path.display()))
    }
    pub fn to_json(&self) -> String { serde_json::to_string_pretty(self).expect("config serialises") }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Clock {
    pub start_hour: f32,
    /// Game minutes per real second. 0 freezes the clock.
    pub minutes_per_second: f32,
    pub latitude_deg: f32,
    pub day_of_year: f32,
    /// Declination amplitude, degrees (axial tilt).
    pub axial_tilt_deg: f32,
}
impl Default for Clock {
    fn default() -> Self { Clock { start_hour: 17.5, minutes_per_second: 0.25, latitude_deg: 35.0, day_of_year: 200.0, axial_tilt_deg: 23.44 } }
}

/// Visibility lerps toward `dust_visibility_m` and colours toward `dust_tint` with local dust.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Fog {
    pub visibility_m: f32, pub dust_visibility_m: f32, pub color: [f32; 3], pub extinction: [f32; 3], pub inscattering: [f32; 3], pub dust_tint: [f32; 3],
}
impl Default for Fog {
    fn default() -> Self {
        Fog { visibility_m: 9000.0, dust_visibility_m: 1500.0, color: [0.62, 0.70, 0.80], extinction: [0.70, 0.74, 0.80],
              inscattering: [0.80, 0.84, 0.92], dust_tint: [0.85, 0.66, 0.45] }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Moon {
    /// 0 new, 0.5 full, 1 new again.
    pub phase: f32,
    /// Illuminance at full moon, lux.
    pub full_lux: f32,
    pub angular_size_deg: f32,
    pub color: [f32; 3],
}
impl Default for Moon {
    fn default() -> Self { Moon { phase: 0.5, full_lux: 0.25, angular_size_deg: 0.52, color: [1.0, 0.95, 0.88] } }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Stars {
    pub count: u32, pub seed: u32,
    /// Skybox brightness, cd/m^2 for a full-white texel.
    pub brightness: f32,
    /// Cubemap face size (px).
    pub face_px: u32,
    /// Brightness skew exponent (higher = fewer bright stars) and the floor of a star's brightness.
    pub skew: f32, pub floor: f32,
}
impl Default for Stars {
    fn default() -> Self { Stars { count: 6000, seed: 7, brightness: 0.6, face_px: 1024, skew: 6.0, floor: 0.15 } }
}

/// Desert-like medium: earth Rayleigh + ozone, with the Mie (dust) term from here.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Sky {
    pub mie_scattering: f32, pub mie_absorption: f32, pub mie_asymmetry: f32, pub mie_scale_height_km: f32,
    /// Replace Bevy's `render_sky.wesl` with a copy that shades per MSAA sample (Bevy 0.20.0 reads depth sample 0 for all samples,
    /// leaving a pale hairline at geometry edges). Off = Bevy's own.
    pub msaa_fix: bool,
    /// Terrain occludes the sun in the air: with a terrain `Ground`, the share of a view ray in the terrain's shadow loses its aerial
    /// in-scatter (the sun glow Bevy's aerial perspective puts on a ridge face with the sun behind it), all but `occlusion_keep` of it
    /// (the multiple-scattered share the shadow does not block), sampled at `occlusion_steps` points. Off = Bevy's own (test red arm).
    pub sun_occlusion: bool, pub occlusion_keep: f32, pub occlusion_steps: u32,
}
impl Default for Sky {
    fn default() -> Self {
        Sky { mie_scattering: 8.0e-6, mie_absorption: 2.0e-6, mie_asymmetry: 0.8, mie_scale_height_km: 1.6, msaa_fix: true, sun_occlusion: true, occlusion_keep: 0.2, occlusion_steps: 8 }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Wind {
    /// Compass direction the wind blows FROM.
    pub from_deg: f32, pub speed_mps: f32,
    /// Gust amplitude as a fraction of speed, and the gust period (s).
    pub gust: f32, pub gust_period_s: f32,
    /// Second gust sine: frequency ratio (incommensurate with the first), its phase, and the two weights.
    pub gust_ratio: f32, pub gust_phase: f32, pub gust_weights: [f32; 2],
}
impl Default for Wind {
    fn default() -> Self { Wind { from_deg: 225.0, speed_mps: 6.0, gust: 0.5, gust_period_s: 9.0, gust_ratio: 2.618, gust_phase: 1.3, gust_weights: [0.6, 0.4] } }
}

/// Where the weather map sits in the world. Map coordinates = world (x, z) + `world_offset_m`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct MapCfg {
    pub world_offset_m: [f32; 2],
    /// Weather map used when no PNG is supplied: a built-in procedural map of this size (px) and extent (m, square, centred on the offset).
    pub default_px: u32, pub default_extent_m: f32,
    /// Built-in map values: mean coverage and its swing, precip blob strength, cloud type, dust.
    pub default_coverage: f32, pub default_coverage_swing: f32, pub default_precip: f32, pub default_type: f32, pub default_dust: f32,
    /// Built-in map shape: the coverage swell is sin(2 pi cycles.x u + phase.x) sin(2 pi cycles.y v + phase.y) over the map (u, v in
    /// 0..1); whole cycles make the map tile (needed by `wrap`). The rain blob: centre (u, v) and radius (fraction of the map).
    pub default_cycles: [f32; 2], pub default_phase: [f32; 2], pub default_blob: [f32; 3],
    /// Advection: the map moves downwind at `advect` x the mean wind (gusts never move it), on the game clock (frozen when the
    /// clock is). 0 = the map stays put: the weather is fixed to the ground and only the cloud noise drifts through it (the old
    /// behaviour); 1 = the weather rides the wind. Untuned.
    pub advect: f32,
    /// Repeat the map past its edges instead of holding the edge texels (an advected map otherwise runs out of weather). Wants a
    /// tileable map: the built-in one tiles while `default_cycles` are whole numbers. Excludes `clouds.background`.
    pub wrap: bool,
    /// How far the weather has moved downwind at start (m, world XZ); a host restoring a saved game sets it. Map coordinates =
    /// world + `world_offset_m` - shift.
    pub start_shift_m: [f32; 2],
    /// Test red arm: the clouds keep the unshifted map while the weather here moves with it (clouds and rain disagree).
    pub static_gpu: bool,
}
impl Default for MapCfg {
    fn default() -> Self {
        MapCfg { world_offset_m: [0.0, 0.0], default_px: 64, default_extent_m: 200_000.0, default_coverage: 0.45, default_coverage_swing: 0.2,
                 default_precip: 0.0, default_type: 0.6, default_dust: 0.1, default_cycles: [1.0, 1.0], default_phase: [0.4, 1.1],
                 default_blob: [0.7, 0.3, 0.25], advect: 1.0, wrap: true, start_shift_m: [0.0, 0.0], static_gpu: false }
    }
}

/// Cloud layer: heights above the ground (m), noise tiles in metres, extinction per metre at density 1.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Clouds {
    pub bottom_m: f32, pub top_m: f32, pub base_tile_m: f32, pub detail_tile_m: f32, pub detail_strength: f32,
    /// Second base-noise scale (m, rotated against the first so no period shows), relative coverage variation from noise (0..1), curl-like detail warp (m).
    pub base_tile2_m: f32, pub coverage_var: f32, pub curl_m: f32,
    pub extinction: f32, pub ambient_scale: f32,
    /// Eye-march steps (up to `horizon_step_boost` x near the horizon).
    pub steps: u32,
    pub light_first_m: f32, pub light_far_m: f32, pub max_distance_m: f32, pub wind_scale: f32,
    pub light_gain: f32, pub shoulder: f32, pub shoulder_limit: f32,
    /// March-buffer scale (1.0 = full resolution).
    pub render_scale: f32,
    /// History blend of the temporal resolve (1.0 = no history at all).
    pub temporal_alpha: f32,
    /// Cloud shadow cookie: map resolution (px), half-width of the covered ground (m), light-march samples through the layer,
    /// redraw bands (one per frame), extinction multiplier on the shadow (1 = the sky's extinction), metres per slant sample, sample cap.
    pub shadow_res: u32, pub shadow_half_m: f32, pub shadow_samples: u32, pub shadow_every: u32, pub shadow_strength: f32,
    pub shadow_step_m: f32, pub shadow_samples_max: u32,
    /// Resolve update division (1, 2 or 4): the march covers 1 pixel in div x div per frame (HZD). 1 = every pixel every frame (reference, ~16x the march cost of 4).
    pub update_div: u32,
    /// Storm towers: curve of (cloud type 0..1, tower depth m), piecewise-linear; at most 4 stops; empty = the flat layer.
    pub tower_depth_m: Vec<[f32; 2]>,
    /// Coverage, precip, cloud type used outside the weather map's extent; None = the map's edge clamp.
    pub background: Option<[f32; 3]>,
    /// Rain-shaft pass strength (0 = off).
    pub shaft_scale: f32,
    /// Draw the cloud dome at all (off = frame-time baseline).
    pub enabled: bool,
    /// Replace the map's coverage everywhere (0..1); None = the map.
    pub coverage_override: Option<f32>,
    /// Probe output instead of the look: 0 off, 1 exposed max channel, 2 direct light, 3 sky ambient, 4 cloud alpha, 5 cloud depth.
    pub debug_view: u32,
    /// Print GPU pass timings every 60 frames (needs TIMESTAMP_QUERY).
    pub pass_timing: bool,
    /// Look constants that were shader literals.
    pub look: CloudLook,
    /// Procedural noise textures.
    pub noise: CloudNoise,
}
impl Default for Clouds {
    fn default() -> Self {
        Clouds {
            bottom_m: 1500.0, top_m: 6000.0, base_tile_m: 11000.0, detail_tile_m: 400.0, detail_strength: 0.6,
            base_tile2_m: 4300.0, coverage_var: 0.5, curl_m: 120.0, extinction: 0.04, ambient_scale: 1.0, steps: 72,
            light_first_m: 25.0, light_far_m: 1500.0, max_distance_m: 90000.0, wind_scale: 1.0, light_gain: 2.0, shoulder: 1.0, shoulder_limit: 1.7,
            render_scale: 0.5, temporal_alpha: 0.1, shadow_res: 512, shadow_half_m: 10000.0, shadow_samples: 10, shadow_every: 4, shadow_strength: 1.0,
            shadow_step_m: 120.0, shadow_samples_max: 48, update_div: 4, tower_depth_m: vec![], background: None, shaft_scale: 1.0,
            enabled: true, coverage_override: None, debug_view: 0, pass_timing: false,
            look: CloudLook::default(), noise: CloudNoise::default(),
        }
    }
}

/// Cloud look constants, formerly literals in the shaders. All untuned.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct CloudLook {
    /// Detail erosion eats the thin edge, so the visible fraction falls below coverage; the base threshold widens by this factor.
    pub cov_boost: f32,
    /// Tower noise: altitude squash when sampling the base shape, and the share of unsquashed noise in the surface.
    pub tower_vsq: f32, pub tower_billow: f32,
    /// Where the map's edge blends into `background`, as a fraction of the map size.
    pub background_blend: f32,
    /// The coverage-variation noise is sampled at base_tile_m x this.
    pub vary_tile_mul: f32,
    /// Up to this many times the steps near the horizon, fading to 1x by `horizon_step_y` (ray y).
    pub horizon_step_boost: f32, pub horizon_step_y: f32,
    /// Empty steps after which the adaptive march goes cheap again.
    pub cheap_empty_steps: u32,
}
impl Default for CloudLook {
    fn default() -> Self {
        CloudLook { cov_boost: 1.5, tower_vsq: 0.3, tower_billow: 0.5, background_blend: 0.05, vary_tile_mul: 2.3,
                    horizon_step_boost: 3.0, horizon_step_y: 0.5, cheap_empty_steps: 6 }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct CloudNoise {
    /// Base shape tile: voxels per side, Worley octave frequencies, Perlin cells per tile and octaves.
    pub base_res: u32, pub base_freqs: [u32; 3], pub base_perlin_cells: u32, pub base_perlin_octaves: u32,
    /// Detail tile: voxels per side and Worley octave frequencies.
    pub detail_res: u32, pub detail_freqs: [u32; 3],
    /// Perlin share added to the Worley fbm in the base shape.
    pub worley_mix: f32,
}
impl Default for CloudNoise {
    fn default() -> Self { CloudNoise { base_res: 128, base_freqs: [4, 8, 16], base_perlin_cells: 4, base_perlin_octaves: 4, detail_res: 32, detail_freqs: [2, 4, 8], worley_mix: 0.6 } }
}

/// Temporal resolve of the cloud buffer (clouds_resolve.wesl) and the switches that make its test red arms.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Resolve {
    /// How far outside the neighbourhood box the reprojected history lies, times this, is the weight pushed to the fresh sample.
    pub reject_gain: f32,
    /// Reprojection displacement (low-res px) below which the history clamp box is 5x5 instead of 3x3. 0 = always 3x3.
    pub still_px: f32,
    /// Confidence of a pixel revealed mid-motion: its first marched visit weighs 1/(1+fresh).
    pub fresh: f32,
    /// Confidence cap (samples).
    pub conf_max: f32,
    /// An unmarched pixel whose confidence is below this takes a running mean of each frame's upsample (a revealed strip would otherwise keep its first one until its single march visit). 0 = off.
    pub upsample_mean_frames: f32,
    /// Reprojection motion (render px) that halves the visits counted by a marched pixel's running mean; while the view moves the pixel takes its single noisy sample (a dot lattice against its smooth neighbours). 0 = off (RED ARM when > 0).
    pub motion_half_px: f32,
    /// A pixel with no history takes a 4-tap wide upsample instead of the plain bilinear one (false = RED ARM: 8 px blotches of single-sample noise).
    pub wide_upsample: bool,
    /// A cut (history reset): a turn of more than `cut_deg` in one frame (180 or more never cuts), or a move of more than `cut_m`.
    pub cut_deg: f32, pub cut_m: f32,
    /// Per-visit jitter step (golden ratio conjugate).
    pub jitter_step: f32,
    /// Light-march jitter offset in pixels, decorrelating it from the eye march.
    pub light_jitter_offset: [f32; 2],
    /// RED ARM switches (tests only): no reprojection (history sampled where the pixel is); the light march shares the eye march's jitter.
    pub no_reproject: bool, pub light_jitter_shared: bool,
}
impl Default for Resolve {
    fn default() -> Self {
        Resolve { reject_gain: 4.0, still_px: 0.5, fresh: 1.0, conf_max: 255.0, upsample_mean_frames: 3.0, motion_half_px: 0.0, wide_upsample: true, cut_deg: 20.0, cut_m: 200.0, jitter_step: 0.618034,
                  light_jitter_offset: [2053.0, 911.0], no_reproject: false, light_jitter_shared: false }
    }
}

/// Distant rain shafts (virga): a rain medium from the cloud base down to the ground.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Shafts {
    /// Extinction per metre at shaft density 1.
    pub sigma: f32,
    /// The medium starts this far from the eye and is fully in by `fade_m` (the streak FX owns the near field); max march distance.
    pub near_m: f32, pub fade_m: f32, pub max_m: f32,
    /// Streak noise: the cloud base noise tile is sampled with this horizontal period and a much longer vertical one.
    pub xz_m: f32, pub y_m: f32,
    /// Share of the density that survives at the ground.
    pub ground_keep: f32,
    /// Cloud presence below which a column has no shaft, and at which it is full.
    pub cloud_lo: f32, pub cloud_hi: f32,
    /// Streak-noise window: below lo the column is a gap, above hi it is full; gaps keep `gap` of the density.
    pub streak_lo: f32, pub streak_hi: f32, pub gap: f32,
    /// Share of the drop lean that shows as shaft lean, and the slew time constant (s).
    pub shear_frac: f32, pub slant_tau_s: f32,
    /// Rain intensity assumed when computing the lean.
    pub lean_intensity: f32,
    /// Lighting: grey rain albedo, cloud-above optical depth scale, march steps.
    pub albedo: f32, pub light_od: f32, pub steps: f32,
    /// Test red arm: the shaft lean follows the live (gusting) drop lean instead of the mean wind's shear lean, so the columns swing with every gust.
    pub gust_lean: bool,
}
impl Default for Shafts {
    fn default() -> Self {
        Shafts { sigma: 0.016, near_m: 50.0, fade_m: 250.0, max_m: 14000.0, xz_m: 1100.0, y_m: 6000.0, ground_keep: 0.4, cloud_lo: 0.05, cloud_hi: 0.4,
                 streak_lo: 0.3, streak_hi: 0.7, gap: 0.05, shear_frac: 0.1, slant_tau_s: 20.0, lean_intensity: 0.7, albedo: 0.6, light_od: 0.03, steps: 80.0,
                 gust_lean: false }
    }
}

/// Rain FX: GPU streaks and splashes, wetness, drop kinematics, light.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct RainCfg {
    /// Camera-following drop volume: half-width (m) in x and z, height (m) above the ground.
    pub half_m: f32, pub height_m: f32,
    /// Streak and splash quads in the mesh; intensity activates the fraction `intensity` of them.
    pub max_streaks: u32, pub max_splashes: u32,
    /// Splash lifetime (s); streak core width (m); streak and splash peak alpha.
    pub splash_life_s: f32, pub streak_width_m: f32, pub streak_alpha: f32, pub splash_alpha: f32,
    /// Shutter time (s) the streak length is exposed over.
    pub shutter_s: f32,
    /// Fraction of the wind speed a drop drifts at, and the cap on that drift (m/s).
    pub wind_drift: f32, pub wind_drift_max: f32,
    /// Fall speed (m/s) at intensity 0 and the added speed at intensity 1.
    pub fall_min_mps: f32, pub fall_gain_mps: f32,
    /// Precip below `precip_lo` is dry, `precip_hi` is full; coverage below `cover_lo` starves it, `cover_hi` is enough cloud.
    pub precip_lo: f32, pub precip_hi: f32, pub cover_lo: f32, pub cover_hi: f32,
    /// Map rain below this intensity is treated as none.
    pub min_intensity: f32,
    /// Wetness: per-second gain at full intensity, per-second loss when dry, and the intensity below which it dries.
    pub wet_up_per_s: f32, pub dry_per_s: f32, pub wet_threshold: f32,
    /// Wet ground look at wetness 1: albedo darkening, roughness and reflectance reached.
    pub wet_darken: f32, pub wet_roughness: f32, pub wet_reflectance: f32,
    /// Rain light before exposure: gain, sun share, sky share, night floor, how much cloud cover dims the sun part, and the exposure level scale.
    pub light_gain: f32, pub sun_share: f32, pub sky_share: f32, pub night_floor: f32, pub cloud_dim: f32, pub level_scale: f32,
    /// Light tint.
    pub light_color: [f32; 3],
    /// Sun elevation range (deg) over which the light fades in from night.
    pub day_from_deg: f32, pub day_to_deg: f32,
    /// Coverage range over which cloud dims the sun part.
    pub dim_from: f32, pub dim_to: f32,
    /// Force this intensity (0..1) everywhere, replacing the map; None = the map.
    pub force: Option<f32>,
    /// Wetness epsilon below which the ground material is left alone.
    pub wet_epsilon: f32,
}
impl Default for RainCfg {
    fn default() -> Self {
        RainCfg {
            half_m: 40.0, height_m: 30.0, max_streaks: 24000, max_splashes: 6000, splash_life_s: 0.45, streak_width_m: 0.012, streak_alpha: 0.55, splash_alpha: 0.5,
            shutter_s: 1.0 / 20.0, wind_drift: 0.8, wind_drift_max: 12.0, fall_min_mps: 5.0, fall_gain_mps: 4.0,
            precip_lo: 0.1, precip_hi: 0.9, cover_lo: 0.3, cover_hi: 0.6, min_intensity: 0.02,
            wet_up_per_s: 1.0 / 40.0, dry_per_s: 1.0 / 240.0, wet_threshold: 0.05, wet_darken: 0.35, wet_roughness: 0.35, wet_reflectance: 0.5,
            light_gain: 1.0, sun_share: 0.65, sky_share: 0.35, night_floor: 0.04, cloud_dim: 0.8, level_scale: 1.2, light_color: [1.0, 0.98, 0.95],
            day_from_deg: -6.0, day_to_deg: 10.0, dim_from: 0.2, dim_to: 0.7, force: None, wet_epsilon: 0.002,
        }
    }
}

/// Ground fog (nephthene_core::fog, fog.wesl). Formation: clear, calm, moist nights build `amount` 0..1, sunshine burns it off and
/// lifts its base, wind mixes it out. Shape: cold air drains off slopes into the lowest ground within `pool_radius_m`; the pool top is
/// level there. Rates, depths and visibilities follow published field ranges (fog 10..300 m deep, ~3 h burn-off at 300 m,
/// visibility 3.912 / extinction) but every default is UNTUNED.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct GroundFog {
    /// On by default (owner, 2026-10-22: every feature on); off is the frame-time baseline.
    pub enabled: bool,
    /// Amount 0..1 used instead of the formation model; None = the model.
    pub force: Option<f32>,
    /// Test switch (red arm): the fog top follows each column's own ground (a blanket over ridges) instead of the pooled floor.
    pub ignore_terrain: bool,
    /// Test switch (red arm): burn-off ignores the terrain shadow (every texel burns as if sunlit, as an elevation-only model would).
    pub ignore_shadow: bool,
    /// Amount per game hour: gained at full formation potential, burnt at full insolation, mixed out at full wind.
    pub form_per_h: f32, pub burn_per_h: f32, pub mix_per_h: f32,
    /// Sun elevation (deg) band over which nocturnal cooling fades out (full below `night_lo_deg`), and the elevation of full burn.
    pub night_lo_deg: f32, pub night_hi_deg: f32, pub burn_full_deg: f32,
    /// Share of the cooling that full cloud cover blocks (clouds radiate back), and of the burn it blocks (shade).
    pub cover_block: f32, pub cover_shade: f32,
    /// Wind (m/s): fog forms fully below `wind_lo_mps`, none above `wind_hi_mps`; above `wind_hi_mps` it is mixed out.
    pub wind_lo_mps: f32, pub wind_hi_mps: f32,
    /// Moisture 0..1 = base + wet_gain * ground wetness - dust_dry * dust.
    pub moisture_base: f32, pub wet_gain: f32, pub dust_dry: f32,
    /// Hours integrated before the start hour (and while the clock is frozen) to reach the state the weather implies.
    pub spin_up_h: f32,
    /// Pool depth (m) at amount 1: `depth_flat_m` on open ground, up to `depth_max_m` where the basin is `relief_m` deep or more.
    pub depth_flat_m: f32, pub depth_max_m: f32, pub relief_m: f32,
    /// Radius (m) over which cold air drains to the lowest ground (the pool floor and the basin relief are taken over it).
    pub pool_radius_m: f32,
    /// Soft top thickness (m), the thinning margin (m) where the pool is shallow, and the top's undulation (m, wavelength m).
    pub top_soft_m: f32, pub edge_m: f32, pub top_wave_m: f32, pub wave_tile_m: f32,
    /// Shallow mist: e-folding height (m) above the ground, the radius (m) and depth (m) that make a hollow, the slope (rise/run) band
    /// over which drainage clears it, and the share it keeps on open flat ground.
    pub shallow_e_m: f32, pub hollow_radius_m: f32, pub hollow_m: f32, pub slope_lo: f32, pub slope_hi: f32, pub flat_share: f32,
    /// Visibility (m) inside the pool and inside the shallow mist at amount 1 (extinction = 3.912 / visibility).
    pub visibility_m: f32, pub shallow_visibility_m: f32,
    /// Burn-off lifts the fog base off the ground: up to `lift_max_m` (sun at `lift_full_deg`, scaled by 1 - amount), soft over `lift_soft_m`.
    pub lift_max_m: f32, pub lift_full_deg: f32, pub lift_soft_m: f32,
    /// Density breakup by 3D value noise: strength 0..1, horizontal and vertical cell (m). Drifts with the wind.
    pub breakup: f32, pub breakup_tile_m: f32, pub breakup_y_m: f32,
    /// Wisps: within `wisp_m` below the pool top the breakup noise erodes the pool, up to `wisp_erode` (0..1) at the top, so the top is
    /// ragged and fades with height instead of a hard level sheet. wisp_m 0 = the hard top (test red arm).
    pub wisp_m: f32, pub wisp_erode: f32,
    /// Haze above the pool top: density falls off by e over `top_fade_m` (eroded by the wisps), so the top fades with height.
    /// 0 (with wisp_m 0) = the hard top (test red arm).
    pub top_fade_m: f32,
    /// Billows: the pool top rises and falls by up to `top_billow_m` over `billow_tile_m`, and is lit and self-shadowed by that
    /// local top, so a sunlit pool seen from above has lit and shaded billows instead of one flat sheet. 0 = flat (test red arm).
    pub top_billow_m: f32, pub billow_tile_m: f32,
    /// Droplet single-scattering albedo and phase asymmetry g.
    pub albedo: f32, pub g: f32,
    /// Multiple scattering: octaves, energy (a) and optical-depth (b) factor per octave.
    pub ms_octaves: u32, pub ms_a: f32, pub ms_b: f32,
    /// Scale on the sun's diffuse light inside the fog (two-stream diffuse transmission from the sunlit top: neutral, whitens deep fog).
    /// 0 = direct beam only (test red arm).
    pub diffuse_sun: f32,
    /// Sky light: scale on the mean sky radiance, how fast it dims with depth below the fog top (share of the optical depth), and the
    /// albedo of the ground under the fog (reflects sky light back up into it).
    pub ambient_scale: f32, pub ambient_od: f32, pub ground_albedo: f32,
    /// Cloud-covered sky light: the clear-sky radiance blends by cloud cover toward the sun's diffuse light through cloud (share
    /// `cloud_diffuse` of the sun's flux, neutral). `ignore_cloud_sky`: clear sky regardless of cloud (test red arm).
    pub cloud_diffuse: f32, pub ignore_cloud_sky: bool,
    /// Terrain shadow on the fog: soft band (m) around the sunlit altitude, sun march range and step (m).
    pub sun_soft_m: f32, pub shadow_range_m: f32, pub shadow_step_m: f32,
    /// Baked fog map: extent (m, square around the camera), size (px), smoothing passes; rebake after moving `rebake_m` or every `rebake_s`.
    pub extent_m: f32, pub bake_px: u32, pub blur_passes: u32, pub rebake_m: f32, pub rebake_s: f32,
    /// March: steps per pixel and the farthest distance (m); density fades over the outer `edge_fade` share of the map. Steps are spaced
    /// as (i / steps)^step_power along the ray, dense near the eye where fog changes the image most; 1 = uniform (test red arm).
    pub steps: u32, pub max_m: f32, pub edge_fade: f32, pub step_power: f32,
    /// March resolution as a share of the view (0.25 = a sixteenth of the pixels), upsampled depth-aware; 1 = full resolution.
    pub render_scale: f32,
    /// Test switch (cost gate red arm, with render_scale 1): per-step hashed value noise instead of the baked noise texture.
    pub hash_noise: bool,
    /// Temporal accumulation of the march: each frame blends this share of the new march into the history reprojected from the last
    /// frame (clamped to the new march's neighbourhood), so the per-pixel step jitter averages out over frames. 1 = no history (test red arm).
    pub temporal_alpha: f32,
    /// Reprojection displacement (march texels) below which the history clamp box is 5x5 instead of 3x3: a still view keeps more history.
    pub still_px: f32,
}
impl Default for GroundFog {
    fn default() -> Self {
        GroundFog {
            enabled: true, force: None, ignore_terrain: false, ignore_shadow: false,
            form_per_h: 0.5, burn_per_h: 1.2, mix_per_h: 2.0, night_lo_deg: -4.0, night_hi_deg: 4.0, burn_full_deg: 20.0,
            cover_block: 0.8, cover_shade: 0.6, wind_lo_mps: 2.5, wind_hi_mps: 6.0, moisture_base: 0.55, wet_gain: 0.45, dust_dry: 1.0, spin_up_h: 24.0,
            depth_flat_m: 25.0, depth_max_m: 160.0, relief_m: 120.0, pool_radius_m: 2500.0,
            top_soft_m: 12.0, edge_m: 25.0, top_wave_m: 8.0, wave_tile_m: 900.0,
            shallow_e_m: 5.0, hollow_radius_m: 400.0, hollow_m: 15.0, slope_lo: 0.12, slope_hi: 0.35, flat_share: 0.3,
            visibility_m: 150.0, shallow_visibility_m: 400.0, lift_max_m: 80.0, lift_full_deg: 20.0, lift_soft_m: 25.0,
            breakup: 0.45, breakup_tile_m: 250.0, breakup_y_m: 40.0, wisp_m: 40.0, wisp_erode: 0.7, top_fade_m: 25.0, top_billow_m: 25.0, billow_tile_m: 300.0, albedo: 0.99, g: 0.75, ms_octaves: 3, ms_a: 0.6, ms_b: 0.35, diffuse_sun: 1.0,
            ambient_scale: 1.0, ambient_od: 0.5, ground_albedo: 0.2, cloud_diffuse: 0.35, ignore_cloud_sky: false, sun_soft_m: 20.0, shadow_range_m: 4000.0, shadow_step_m: 100.0,
            extent_m: 12000.0, bake_px: 128, blur_passes: 2, rebake_m: 1500.0, rebake_s: 2.0, steps: 48, max_m: 6000.0, edge_fade: 0.15, step_power: 2.0, render_scale: 0.25, hash_noise: false, temporal_alpha: 0.1, still_px: 0.5,
        }
    }
}

/// Ticking systems.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Sim {
    /// Time constant (s) of the smoothing of the weather sample under the camera.
    pub here_tau_s: f32,
    /// Open the weather panel at start.
    pub panel_open: bool,
}
impl Default for Sim {
    fn default() -> Self { Sim { here_tau_s: 5.0, panel_open: false } }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_round_trip_and_partial_files_work() {
        let c = Config::default();
        assert_eq!(Config::from_json(&c.to_json()).unwrap(), c);
        let p = Config::from_json(r#"{"clouds": {"steps": 99}, "wind": {"speed_mps": 3.0}}"#).unwrap();
        assert_eq!((p.clouds.steps, p.wind.speed_mps, p.clouds.bottom_m), (99, 3.0, 1500.0));
        assert!(Config::from_json(r#"{"clouds": {"stepz": 1}}"#).is_ok(), "unknown keys are ignored (serde default)");
    }
}
