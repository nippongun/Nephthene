//! Air at the surface: temperature over the day and the season, the dew point and humidity, and temperature with altitude (freezing
//! level, snow line). Pure maths; the plugin integrates [`step`] on the game clock into `Weather.air`, and ground fog forms on it.
//!
//! Temperature is a force-restore model (Deardorff 1978): the sun heats the surface by its elevation, the surface loses heat by
//! radiation day and night, and both are restored toward the season's daily mean. Clouds shade the sun and block the loss, so
//! overcast days and nights stay near the mean. The dew point is the air mass's, raised by wet ground and lowered by dust (dry air).
use crate::{Clock, config::AirCfg};

/// The air at one place: surface temperature and dew point (deg C, at `ref_alt_m`), relative humidity 0..1, and the altitudes (m)
/// of the 0 deg C level and of the snow line.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State { pub temp_c: f32, pub dew_c: f32, pub rh: f32, pub freezing_m: f32, pub snow_line_m: f32 }

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

/// The state for surface temperature `temp_c`: the dew point is never above the temperature (the excess condenses as dew or fog).
/// ponytail: no latent heat, so a saturated night keeps cooling at the clear rate; holding it near the dew point is the upgrade.
pub fn state(c: &AirCfg, clock: &Clock, temp_c: f32, wetness: f32, dust: f32) -> State {
    let dew_c = air_mass_dew_c(c, clock, wetness, dust).min(temp_c);
    let freezing_m = freezing_m(c, temp_c);
    State { temp_c, dew_c, rh: rh(temp_c, dew_c), freezing_m, snow_line_m: freezing_m - c.melt_m }
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
        let flat = AirCfg { lapse_k_per_km: 0.0, ..AirCfg::default() };
        assert!(freezing_m(&flat, 5.0) == f32::INFINITY, "warm air without a lapse rate has no freezing level");
        assert!(freezing_m(&flat, -1.0) == f32::NEG_INFINITY, "cold air without a lapse rate is freezing at any height");
    }
}
