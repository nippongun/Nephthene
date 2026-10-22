//! Air at the surface: temperature over the day and the season, the dew point and humidity, and temperature with altitude (freezing
//! level, snow line). Pure maths; the plugin integrates [`step`] on the game clock into `Weather.air`, and ground fog forms on it.
//!
//! Temperature is a force-restore model (Deardorff 1978): the sun heats the surface by its elevation, the surface loses heat by
//! radiation day and night, and both are restored toward the season's daily mean. Clouds shade the sun and block the loss, so
//! overcast days and nights stay near the mean. The dew point is the air mass's, raised by wet ground and lowered by dust (dry air).
//!
//! [`Field`] holds the temperature over the weather map, one force-restore cell per texel block, in the map's frame (it moves with the
//! weather). Cells store the temperature reduced to `ref_alt_m`; the altitude of the place read applies the lapse rates.
use crate::{Clock, Config, WeatherMap, climate, config::AirCfg, rain::smoothstep, texel_axis};
use glam::Vec2;

/// The air at one place: temperature and dew point (deg C) at the altitude read (`ref_alt_m` for [`state`]), relative humidity 0..1,
/// the altitudes (m) of the 0 deg C level and of the snow line, and the precipitation type there (0 rain, 1 snow, sleet between).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State { pub temp_c: f32, pub dew_c: f32, pub rh: f32, pub freezing_m: f32, pub snow_line_m: f32, pub precip_kind: f32 }

/// The season's daily mean temperature (deg C): `mean_c` swung by `season_c` with the sun's declination `lag_days` earlier, flipped
/// south of the equator. ponytail: the equator takes the north's season (signum of 0 is 1); set `season_c` near 0 in the tropics.
pub fn mean_c(c: &AirCfg, clock: &Clock) -> f32 {
    let dec = crate::declination(clock.axial_tilt_deg, clock.day_of_year - c.lag_days);
    c.mean_c + c.season_c * clock.latitude_deg.signum() * dec / clock.axial_tilt_deg.max(1e-3)
}

/// Rate of change of the surface temperature (K per game hour) at `temp_c` under the sun at elevation `el_deg` and cloud cover
/// `coverage` 0..1.
pub fn rate(c: &AirCfg, clock: &Clock, temp_c: f32, el_deg: f32, coverage: f32) -> f32 {
    let sun = el_deg.to_radians().sin().max(0.0);
    c.heat_k_per_h * sun * (1.0 - c.cloud_shade * coverage) - c.cool_k_per_h * (1.0 - c.cloud_block * coverage) - c.restore_per_h * (temp_c - mean_c(c, clock))
}

/// One step of `dt_h` game hours from `temp_c` at `hour`.
pub fn step(c: &AirCfg, clock: &Clock, temp_c: f32, hour: f32, dt_h: f32, coverage: f32) -> f32 {
    temp_c + dt_h * rate(c, clock, temp_c, crate::sun(clock, hour).0, coverage)
}

/// Spin-up step (game hours). ponytail: the fog's step; explicit Euler is stable while restore_per_h x step << 1.
const SPIN_STEP_H: f32 = 0.25;

/// The surface temperature at `hour` after `spin_up_h` hours of this cover, from the season's mean.
pub fn spin_up(c: &AirCfg, clock: &Clock, hour: f32, coverage: f32) -> f32 {
    let n = (c.spin_up_h / SPIN_STEP_H).ceil() as u32;
    let h0 = hour - n as f32 * SPIN_STEP_H;
    (0..n).fold(mean_c(c, clock), |t, i| step(c, clock, t, h0 + SPIN_STEP_H * i as f32, SPIN_STEP_H, coverage))
}

/// The air mass's dew point (deg C): `dew_below_c` under the season's mean, raised by ground `wetness` 0..1, lowered by `dust` 0..1.
pub fn air_mass_dew_c(c: &AirCfg, clock: &Clock, wetness: f32, dust: f32) -> f32 {
    mean_c(c, clock) - c.dew_below_c + c.wet_gain_c * wetness - c.dust_dry_c * dust
}

/// Saturation vapour pressure (hPa) over water at `t_c` (deg C): the Magnus form with Alduchov & Eskridge (1996) constants.
/// ponytail: over water at every temperature; over ice below 0 deg C (frost point) is the upgrade.
pub fn vapour_hpa(t_c: f32) -> f32 { 6.1094 * (17.625 * t_c / (t_c + 243.04)).exp() }

/// Relative humidity 0..1 of air at `t_c` with dew point `dew_c`.
pub fn rh(t_c: f32, dew_c: f32) -> f32 { (vapour_hpa(dew_c) / vapour_hpa(t_c)).min(1.0) }

/// Temperature (deg C) at altitude `y` (m) above surface air at `temp_c`, by the lapse rate from `ref_alt_m`.
/// ponytail: one linear lapse rate, no night inversion near the ground (the fog's drainage pools the cold air); a sounding is the upgrade.
pub fn at_alt(c: &AirCfg, temp_c: f32, y: f32) -> f32 { temp_c - c.lapse_k_per_km * (y - c.ref_alt_m) / 1000.0 }

/// Altitude (m) of the 0 deg C level over surface air at `temp_c`; without a lapse rate, infinitely high (warm) or low (freezing).
pub fn freezing_m(c: &AirCfg, temp_c: f32) -> f32 {
    if c.lapse_k_per_km <= 0.0 { if temp_c > 0.0 { f32::INFINITY } else { f32::NEG_INFINITY } } else { c.ref_alt_m + 1000.0 * temp_c / c.lapse_k_per_km }
}

/// Precipitation type 0..1 (0 rain, 1 snow, sleet between) falling through air at `temp_c` at the ground.
pub fn precip_kind(c: &AirCfg, temp_c: f32) -> f32 { 1.0 - smoothstep(c.snow_c, c.rain_c, temp_c) }

/// The state at altitude `y` (m) in air whose temperature reduced to `ref_alt_m` is `temp_ref_c`: the temperature falls by
/// `lapse_k_per_km` and the dew point by `dew_lapse_k_per_km`, and the dew point is never above the temperature (the excess condenses
/// as dew or fog). ponytail: no latent heat, so a saturated night keeps cooling at the clear rate; holding it near the dew point is
/// the upgrade.
pub fn state_at(c: &AirCfg, clock: &Clock, temp_ref_c: f32, y: f32, wetness: f32, dust: f32) -> State {
    let temp_c = at_alt(c, temp_ref_c, y);
    let dew_c = (air_mass_dew_c(c, clock, wetness, dust) - c.dew_lapse_k_per_km * (y - c.ref_alt_m) / 1000.0).min(temp_c);
    let freezing_m = freezing_m(c, temp_ref_c);
    State { temp_c, dew_c, rh: rh(temp_c, dew_c), freezing_m, snow_line_m: freezing_m - c.melt_m, precip_kind: precip_kind(c, temp_c) }
}

/// The state at `ref_alt_m` for surface temperature `temp_c` there.
pub fn state(c: &AirCfg, clock: &Clock, temp_c: f32, wetness: f32, dust: f32) -> State { state_at(c, clock, temp_c, c.ref_alt_m, wetness, dust) }

/// The temperature field: `n` x `n` cells over the weather map's extent, in map coordinates (rows north to south, cell centres like the
/// map's texels), holding the surface temperature reduced to `ref_alt_m` (deg C). Each cell runs the force-restore model under the
/// map's cloud cover at its centre. Being in the map's frame, it moves with the weather (`map_shift`); read it through `map_xz`.
/// Each cell restores to the climate under it now (`climate::air`): the climate is fixed to the world, the field moves over it.
/// ponytail: one sun for the whole field (a 200 km map spans about 2 degrees of sun).
#[derive(Clone, Debug, PartialEq)]
pub struct Field { pub n: u32, pub min_x: f32, pub max_z: f32, pub size_x: f32, pub size_z: f32, pub wrap: bool, pub temp_c: Vec<f32> }

impl Field {
    /// Each cell's cloud cover and local air config. A cell at map (x, z) sits on the climate map at (x + shift.x, z + shift.y): map =
    /// world + world_offset - shift, and the climate map is read at world + world_offset.
    fn inputs(&self, cfg: &Config, map: &WeatherMap, climate: Option<&WeatherMap>, shift: Vec2) -> Vec<(f32, AirCfg)> {
        let (dx, dz) = (self.size_x / self.n as f32, self.size_z / self.n as f32);
        (0..self.n * self.n).map(|k| {
            let (i, j) = (k % self.n, k / self.n);
            let (x, z) = (self.min_x + (i as f32 + 0.5) * dx, self.max_z - (j as f32 + 0.5) * dz);
            let p = climate::place(&cfg.climate, climate, x + shift.x, z + shift.y);
            (map.sample(x, z).coverage, climate::air(&cfg.air, &cfg.climate, &p))
        }).collect()
    }

    /// The field over `map` at `hour` after `air.spin_up_h` hours of the map's cover, each cell from its own season mean: [`spin_up`]
    /// per cell with its place's air config. `climate` None reads `climate.uniform` everywhere.
    pub fn spin_up(cfg: &Config, map: &WeatherMap, climate: Option<&WeatherMap>, shift: Vec2, hour: f32) -> Field {
        let n = cfg.air.field_px.max(1);
        let mut f = Field { n, min_x: map.min_x, max_z: map.max_z, size_x: map.size_x, size_z: map.size_z, wrap: map.wrap, temp_c: Vec::new() };
        let inputs = f.inputs(cfg, map, climate, shift);
        f.temp_c = inputs.iter().map(|(_, a)| mean_c(a, &cfg.clock)).collect();
        let steps = (cfg.air.spin_up_h / SPIN_STEP_H).ceil() as u32;
        let h0 = hour - steps as f32 * SPIN_STEP_H;
        for s in 0..steps { f.advance(&cfg.clock, &inputs, h0 + SPIN_STEP_H * s as f32, SPIN_STEP_H); }
        f
    }

    /// One step of `dt_h` game hours from `hour`, under the map's cover and the climate under each cell now.
    pub fn step(&mut self, cfg: &Config, map: &WeatherMap, climate: Option<&WeatherMap>, shift: Vec2, hour: f32, dt_h: f32) {
        let inputs = self.inputs(cfg, map, climate, shift);
        self.advance(&cfg.clock, &inputs, hour, dt_h);
    }

    fn advance(&mut self, clock: &Clock, inputs: &[(f32, AirCfg)], hour: f32, dt_h: f32) {
        let el = crate::sun(clock, hour).0;
        for (t, (cov, a)) in self.temp_c.iter_mut().zip(inputs) { *t += dt_h * rate(a, clock, *t, el, *cov); }
    }

    /// Bilinear temperature reduced to `ref_alt_m` (deg C) at map (x, z), held at the edges (wrapped when the map wraps).
    pub fn temp_at(&self, x: f32, z: f32) -> f32 {
        let (x0, x1, tx) = texel_axis((x - self.min_x) / self.size_x * self.n as f32 - 0.5, self.n, self.wrap);
        let (z0, z1, tz) = texel_axis((self.max_z - z) / self.size_z * self.n as f32 - 0.5, self.n, self.wrap);
        let t = |i: u32, j: u32| self.temp_c[(j * self.n + i) as usize];
        let (a, b) = (t(x0, z0) + (t(x1, z0) - t(x0, z0)) * tx, t(x0, z1) + (t(x1, z1) - t(x0, z1)) * tx);
        a + (b - a) * tz
    }

    /// The air at map (x, z) at altitude `y` (m; the ground there for the surface air), with the climate there (its air config, and
    /// its ground wetness or the rain's `wetness`, the wetter) and `dust` 0..1.
    #[allow(clippy::too_many_arguments)]
    pub fn state(&self, cfg: &Config, climate: Option<&WeatherMap>, shift: Vec2, x: f32, z: f32, y: f32, wetness: f32, dust: f32) -> State {
        let p = climate::place(&cfg.climate, climate, x + shift.x, z + shift.y);
        state_at(&climate::air(&cfg.air, &cfg.climate, &p), &cfg.clock, self.temp_at(x, z), y, climate::wetness(&p, wetness), dust)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Surface temperature every 15 game minutes over one day at cover `cov`, after spin-up.
    fn day(c: &AirCfg, k: &Clock, cov: f32) -> Vec<(f32, f32)> {
        (0..96).map(|i| { let h = i as f32 * 0.25; (h, spin_up(c, k, h, cov)) }).collect()
    }
    /// Max temperature less min temperature (K).
    fn range(d: &[(f32, f32)]) -> f32 {
        let max = d.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);
        let min = d.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
        max - min
    }
    /// Mean temperature over the day (deg C).
    fn mean(d: &[(f32, f32)]) -> f32 { d.iter().map(|p| p.1).sum::<f32>() / d.len() as f32 }
    /// First hour (on a 0.01 h grid) with the sun above the horizon.
    fn sunrise(k: &Clock) -> f32 { (0..2400).map(|i| i as f32 * 0.01).find(|&h| crate::sun(k, h).0 > 0.0).unwrap() }
    /// (coldest hour, warmest hour) of a day.
    fn coldest_warmest(d: &[(f32, f32)]) -> (f32, f32) {
        let cold = d.iter().min_by(|a, b| a.1.total_cmp(&b.1)).unwrap().0;
        let warm = d.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap().0;
        (cold, warm)
    }

    // Budgets: the surface is coldest around sunrise (heating overtakes the loss within about an hour of it); warmest mid-afternoon
    // (observed 14-16 h solar, an hour either side allowed); clear-sky land diurnal range 8-15 K (Dai, Trenberth & Karl 1999), 2-3 K allowed.
    #[test]
    fn the_day_warms_after_noon_and_cools_to_sunrise() {
        let k = Clock::default();
        let rise = sunrise(&k);
        let measure = |c: &AirCfg| { let d = day(c, &k, 0.1); let (cold, warm) = coldest_warmest(&d); (cold, warm, range(&d)) };
        let ok = |c: &AirCfg| { let (cold, warm, r) = measure(c); (cold - rise).abs() <= 1.5 && (13.0..=17.0).contains(&warm) && (6.0..=18.0).contains(&r) };
        let (cold, warm, r) = measure(&AirCfg::default());
        println!("day: min at {cold:.2} h, max at {warm:.2} h, range {r:.2} K (sunrise {rise:.2} h)");
        assert!(ok(&AirCfg::default()), "min at {cold} h (sunrise {rise} h, budget 1.5 h), max at {warm} h (budget 13-17 h), range {r} K (budget 6-18 K)");
        let red = AirCfg { heat_k_per_h: 0.0, ..AirCfg::default() };
        assert!(!ok(&red), "red arm passes: the gate cannot fail");
    }

    // Budget: overcast diurnal range is a third to a half of clear (Dai et al. 1999); the gate takes the upper bound.
    #[test]
    fn clouds_shrink_the_daily_range() {
        let (c, k) = (AirCfg::default(), Clock::default());
        let ok = |c: &AirCfg| range(&day(c, &k, 0.9)) < 0.5 * range(&day(c, &k, 0.1));
        let (clear, overcast) = (range(&day(&c, &k, 0.1)), range(&day(&c, &k, 0.9)));
        println!("diurnal range: clear {clear:.2} K, overcast {overcast:.2} K (budget overcast < 0.5 x clear)");
        assert!(ok(&c), "overcast range {overcast} K is not under half the clear range {clear} K");
        let red = AirCfg { cloud_shade: 0.0, cloud_block: 0.0, ..AirCfg::default() };
        assert!(!ok(&red), "red arm passes: the gate cannot fail");
    }

    // Budget: half the default annual range (2 x season_c = 20 K); the two dates sit near the extremes of the season.
    #[test]
    fn winter_is_colder_and_the_south_flips_it() {
        let mean_at = |c: &AirCfg, lat: f32, doy: f32| mean(&day(c, &Clock { day_of_year: doy, latitude_deg: lat, ..Clock::default() }, 0.1));
        let ok = |c: &AirCfg| mean_at(c, 35.0, 200.0) - mean_at(c, 35.0, 15.0) > 10.0 && mean_at(c, -35.0, 15.0) - mean_at(c, -35.0, 200.0) > 10.0;
        let (north, south) = (mean_at(&AirCfg::default(), 35.0, 200.0) - mean_at(&AirCfg::default(), 35.0, 15.0),
            mean_at(&AirCfg::default(), -35.0, 15.0) - mean_at(&AirCfg::default(), -35.0, 200.0));
        println!("summer minus winter: north {north:.2} K, south (flipped) {south:.2} K (budget 10 K each)");
        assert!(ok(&AirCfg::default()), "north {north} K, south {south} K, budget 10 K each");
        let red = AirCfg { season_c: 0.0, ..AirCfg::default() };
        assert!(!ok(&red), "red arm passes: the gate cannot fail");
    }

    // Published tables give 52.5 % at 20 C with a 10 C dew point; Magnus is within 0.1 % there, so 1 % is the budget.
    #[test]
    fn humidity_and_altitude() {
        let (c, k) = (AirCfg::default(), Clock::default());
        assert!((rh(20.0, 10.0) - 0.525).abs() < 0.01, "rh(20, 10) = {}", rh(20.0, 10.0));
        assert!(rh(12.0, 12.0) == 1.0, "saturated air is 100 %: {}", rh(12.0, 12.0));
        let wet = state(&c, &k, 15.0, 10.0, 0.0);
        assert!(wet.dew_c == 15.0 && wet.rh == 1.0, "wet ground must saturate the air: dew {} rh {}", wet.dew_c, wet.rh);
        assert!((at_alt(&c, 15.0, 1000.0) - 8.5).abs() < 1e-4, "15 C at 1 km with 6.5 K/km is 8.5 C: {}", at_alt(&c, 15.0, 1000.0));
        assert!((freezing_m(&c, 6.5) - 1000.0).abs() < 1e-3, "6.5 C at the surface is 1 km from freezing: {}", freezing_m(&c, 6.5));
        assert!((state(&c, &k, 6.5, 0.0, 0.0).snow_line_m - 700.0).abs() < 1e-3, "snow line is 300 m under the freezing level");
        // 1 km up, unsaturated: the temperature falls 6.5 K, the dew point 1.8 K.
        let up = state_at(&c, &k, 30.0, 1000.0, 0.0, 0.0);
        assert!((up.temp_c - 23.5).abs() < 1e-4 && (up.dew_c - (air_mass_dew_c(&c, &k, 0.0, 0.0) - 1.8)).abs() < 1e-4, "1 km up: {up:?}");
        assert!(precip_kind(&c, -1.0) == 1.0 && precip_kind(&c, 3.0) == 0.0 && (precip_kind(&c, 1.0) - 0.5).abs() < 1e-6, "rain-snow band");
        let flat = AirCfg { lapse_k_per_km: 0.0, ..AirCfg::default() };
        assert!(freezing_m(&flat, 5.0) == f32::INFINITY, "warm air without a lapse rate has no freezing level");
        assert!(freezing_m(&flat, -1.0) == f32::NEG_INFINITY, "cold air without a lapse rate is freezing at any height");
    }

    /// An 8 x 8 map, overcast (0.9) in the west half and clear (0.1) in the east; the field's cells sit on its texels.
    fn halves() -> (WeatherMap, AirCfg) {
        let b = |v: f32| (v * 255.0 + 0.5) as u8;
        let rgba = (0..64).flat_map(|k| [if k % 8 < 4 { b(0.9) } else { b(0.1) }, 0, 0, 0]).collect();
        (WeatherMap { width: 8, height: 8, min_x: -4000.0, max_z: 4000.0, size_x: 8000.0, size_z: 8000.0, rgba, wrap: false }, AirCfg { field_px: 8, ..AirCfg::default() })
    }

    // Budget: as `clouds_shrink_the_daily_range` (Dai et al. 1999), between two places of one field: the overcast half's diurnal range
    // under half the clear half's. Each cell must also be the single-place model under its own cover (1e-4 K: f32 rounding).
    #[test]
    fn the_field_follows_the_cover_under_it() {
        let (map, c0) = halves();
        let k = Clock::default();
        let ranges = |c: &AirCfg| {
            let days: Vec<_> = (0..96).map(|i| { let h = i as f32 * 0.25; (h, Field::spin_up(&Config { air: c.clone(), clock: k.clone(), ..Config::default() }, &map, None, Vec2::ZERO, h)) }).collect();
            let at = |x: f32, cov: f32| {
                let d: Vec<(f32, f32)> = days.iter().map(|(h, f)| (*h, f.temp_at(x, 500.0))).collect();
                let worst = days.iter().map(|(h, f)| (f.temp_at(x, 500.0) - spin_up(c, &k, *h, cov)).abs()).fold(0.0, f32::max);
                (range(&d), worst)
            };
            let cov = |v: f32| (v * 255.0 + 0.5).floor() / 255.0;
            (at(-2500.0, cov(0.9)), at(2500.0, cov(0.1)))
        };
        let ((over, e0), (clear, e1)) = ranges(&c0);
        println!("field diurnal range: overcast half {over:.2} K, clear half {clear:.2} K; cell vs single-place model {:.2e} K", e0.max(e1));
        assert!(e0.max(e1) < 1e-4, "a cell is not the single-place model under its cover: {} K", e0.max(e1));
        assert!(over < 0.5 * clear, "overcast half range {over} K is not under half the clear half {clear} K");
        let ((ro, _), (rc, _)) = ranges(&AirCfg { cloud_shade: 0.0, cloud_block: 0.0, ..c0.clone() });
        assert!(ro >= 0.5 * rc, "red arm passes: the gate cannot fail ({ro} vs {rc} K)");
    }

    // Gate C5-place (roadmap): one place in winter, read on the valley floor (0 m) and on a ridge (3000 m), gives rain and snow.
    // Budget: rain is precip_kind <= 0.1 and snow >= 0.9 (the ends of the sleet band). Mid-afternoon at 35 N in mid-January.
    #[test]
    fn rain_in_the_valley_snow_on_the_ridge() {
        let k = Clock { day_of_year: 15.0, ..Clock::default() };
        let m = crate::config::MapCfg::default();
        let map = WeatherMap::procedural(&m);
        let kinds = |c: &AirCfg| {
            let cfg = Config { air: c.clone(), clock: k.clone(), ..Config::default() };
            let f = Field::spin_up(&cfg, &map, None, Vec2::ZERO, 14.0);
            let [x, z] = crate::map_xz(&m, Vec2::ZERO, 0.0, 0.0);
            (f.state(&cfg, None, Vec2::ZERO, x, z, 0.0, 0.0, 0.0), f.state(&cfg, None, Vec2::ZERO, x, z, 3000.0, 0.0, 0.0))
        };
        let ok = |c: &AirCfg| { let (lo, hi) = kinds(c); lo.precip_kind <= 0.1 && hi.precip_kind >= 0.9 };
        let (lo, hi) = kinds(&AirCfg::default());
        println!("winter 14 h: 0 m {:.1} C kind {:.2}, 3000 m {:.1} C kind {:.2}", lo.temp_c, lo.precip_kind, hi.temp_c, hi.precip_kind);
        assert!(ok(&AirCfg::default()), "valley {lo:?}, ridge {hi:?}: budget rain <= 0.1 and snow >= 0.9");
        assert!(!ok(&AirCfg { lapse_k_per_km: 0.0, ..AirCfg::default() }), "red arm (lapse rate 0) passes: the gate cannot fail");
    }

    /// The halves' extent with uniform cover 0.1, and a climate map painted -25 K (R 0) in the west half, neutral (128) in the east.
    fn cold_west() -> (WeatherMap, WeatherMap) {
        let (mut map, _) = halves();
        map.rgba = (0..64).flat_map(|_| [26u8, 0, 0, 0]).collect();
        let clim = WeatherMap { rgba: (0..64).flat_map(|k| [if k % 8 < 4 { 0u8 } else { 128 }, 128, 0, 0]).collect(), ..map.clone() };
        (map, clim)
    }

    // Budget: 1 % of the painted offset (0.25 K). The model is linear in the temperature, so a restore target 25 K lower makes the
    // spun-up day 25 K colder, less e^(-restore_per_h x spin_up_h) = 6e-6 of it.
    #[test]
    fn a_painted_cold_place_is_colder() {
        let (map, clim) = cold_west();
        let diff = |c: &Config| {
            let days: Vec<Field> = (0..24).map(|h| Field::spin_up(c, &map, Some(&clim), Vec2::ZERO, h as f32)).collect();
            let mean = |x: f32| days.iter().map(|f| f.temp_at(x, 500.0)).sum::<f32>() / 24.0;
            mean(-2500.0) - mean(2500.0)
        };
        let c = Config { air: AirCfg { field_px: 8, ..AirCfg::default() }, ..Config::default() };
        let d = diff(&c);
        println!("painted -25 K: daily mean west minus east {d:.3} K (budget -25 +- 0.25 K)");
        assert!((d + 25.0).abs() <= 0.25, "west minus east {d} K, painted -25 K");
        let mut red = c.clone();
        red.climate.ignore = true;
        assert!((diff(&red) + 25.0).abs() > 0.25, "red arm (climate.ignore) passes: the gate cannot fail");
    }

    /// A clear map (cover 0.1 everywhere).
    fn clear() -> WeatherMap { WeatherMap::procedural(&crate::config::MapCfg { default_coverage: 0.1, default_coverage_swing: 0.0, ..Default::default() }) }

    // Budget: desert afternoon air under 25 % relative humidity (Phoenix: about 10 % on June afternoons, 30 % year-round;
    // docs/climate.md). Mid-July 15 h at 35 N, clear, the air mass at full dryness.
    #[test]
    fn desert_air_is_dry_in_the_afternoon() {
        let map = clear();
        let rh = |humidity: f32, ignore: bool| {
            let mut c = Config::default();
            (c.climate.uniform, c.climate.ignore) = ([0.0, humidity, 0.0, 0.0], ignore);
            Field::spin_up(&c, &map, None, Vec2::ZERO, 15.0).state(&c, None, Vec2::ZERO, 0.0, 0.0, 0.0, 0.0, 0.0).rh
        };
        let (dry, neutral, humid) = (rh(-1.0, false), rh(0.0, false), rh(1.0, false));
        println!("15 h July rh: dry {dry:.2}, neutral {neutral:.2}, humid {humid:.2} (budget dry <= 0.25)");
        assert!(dry <= 0.25, "desert afternoon rh {dry}, budget 0.25");
        assert!(humid >= neutral && neutral > dry, "humidity must order the air: dry {dry}, neutral {neutral}, humid {humid}");
        assert!(rh(-1.0, true) > 0.25, "red arm (climate.ignore) passes: the gate cannot fail");
    }

    // Budget: snow absorbs (1 - 0.7) / (1 - 0.2) = 0.375 of the sunlight bare ground does (albedo tables, docs/climate.md), and the
    // model's diurnal swing is linear in the heating, so the range falls to about 0.375 of bare; 0.5 is the budget.
    #[test]
    fn snow_cover_cuts_the_days_warming() {
        let map = clear();
        let range_with = |snow: f32, ignore: bool| {
            let mut c = Config { air: AirCfg { field_px: 1, ..AirCfg::default() }, ..Config::default() };
            (c.climate.uniform, c.climate.ignore) = ([0.0, 0.0, snow, 0.0], ignore);
            let d: Vec<(f32, f32)> = (0..96).map(|i| { let h = i as f32 * 0.25; (h, Field::spin_up(&c, &map, None, Vec2::ZERO, h).temp_c[0]) }).collect();
            range(&d)
        };
        let (bare, snow) = (range_with(0.0, false), range_with(1.0, false));
        println!("diurnal range: bare {bare:.2} K, snow {snow:.2} K (budget snow <= 0.5 x bare)");
        assert!(snow <= 0.5 * bare, "snow range {snow} K is not under half the bare range {bare} K");
        assert!(range_with(1.0, true) > 0.5 * bare, "red arm (climate.ignore) passes: the gate cannot fail");
    }
}
