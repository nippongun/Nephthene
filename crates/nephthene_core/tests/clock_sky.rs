//! Clock, sky, wind and weather-map maths with the default config (35 N, day 200).
use nephthene_core::{config::*, *};

/// Solar noon at 35N in July: due south (az 180), elevation 90 - lat + declination (~20.6 deg on day 200).
#[test]
fn noon_sun_is_south_at_the_expected_height() {
    let (el, az) = sun(&Clock::default(), 12.0);
    assert!((az - 180.0).abs() < 0.5, "noon azimuth {az}");
    assert!((el - 75.6).abs() < 1.0, "noon elevation {el}");
}

/// Morning sun is east, evening west, and it sets between 19:00 and 20:00 solar time in July.
#[test]
fn sun_rises_east_sets_west_in_the_evening() {
    let c = Clock::default();
    let (_, am) = sun(&c, 8.0);
    let (_, pm) = sun(&c, 17.0);
    assert!((45.0..135.0).contains(&am) && (225.0..315.0).contains(&pm), "am {am} pm {pm}");
    assert!(sun(&c, 19.0).0 > 0.0 && sun(&c, 20.0).0 < 0.0, "sunset not between 19 and 20");
}

/// A full moon sits opposite the sun: up at midnight, near the antipode.
#[test]
fn full_moon_is_opposite_the_sun() {
    let c = Clock::default();
    let m = Moon { phase: 0.5, ..Moon::default() };
    let (s, mo) = (sun(&c, 0.0), moon(&c, &m, 0.0));
    assert!(mo.0 > 30.0, "full moon at midnight elevation {}", mo.0);
    assert!(dir(s.0, s.1).dot(dir(mo.0, mo.1)) < -0.99, "moon not opposite the sun");
    assert!((moon_lux(&m) - m.full_lux).abs() < 1e-6 && moon_lux(&Moon { phase: 0.0, ..m }) < 1e-6);
}

#[test]
fn curve_interpolates_and_clamps() {
    let c = [[0.0, 0.0], [10.0, 100.0]];
    assert_eq!((curve(&c, -5.0), curve(&c, 5.0), curve(&c, 50.0)), (0.0, 50.0, 100.0));
}

/// The built-in map: its size is `default_px`, the centre reads inside the configured swing around the mean, the swing reaches
/// above 0.5 somewhere, and sampling far outside clamps to the edge texel instead of failing.
#[test]
fn procedural_map_follows_its_config() {
    let m = MapCfg::default();
    let map = WeatherMap::procedural(&m);
    assert_eq!((map.width, map.height), (m.default_px, m.default_px));
    assert_eq!((map.size_x, map.size_z), (m.default_extent_m, m.default_extent_m));
    let (lo, hi) = (m.default_coverage - m.default_coverage_swing - 0.01, m.default_coverage + m.default_coverage_swing + 0.01);
    let mut peak = 0.0f32;
    for j in 0..map.height {
        for i in 0..map.width {
            let gx = map.min_x + (i as f32 + 0.5) / map.width as f32 * map.size_x;
            let gz = map.max_z - (j as f32 + 0.5) / map.height as f32 * map.size_z;
            let here = map.sample(gx, gz);
            assert!((lo..=hi).contains(&here.coverage), "coverage {} outside the swing at ({gx}, {gz})", here.coverage);
            assert!((here.dust - m.default_dust).abs() < 0.01 && here.precip == 0.0);
            peak = peak.max(here.coverage);
        }
    }
    assert!(peak > 0.5, "max coverage over the map {peak}");
    let far = map.sample(map.min_x - 1.0e7, map.max_z + 1.0e7);
    let edge = map.sample(map.min_x, map.max_z);
    assert_eq!(far, edge, "outside the map must clamp to the edge");
}

/// Wind *from* the south-west blows toward +X (east) and +Z (north); speed stays within the gust band.
#[test]
fn wind_blows_from_its_heading() {
    let w = Wind::default();
    for t in [0.0, 1.0, 4.5, 13.0] {
        let v = wind(&w, t);
        assert!(v.x > 0.0 && v.z > 0.0 && v.y == 0.0, "wind {v} at {t}");
        assert!(v.length() <= w.speed_mps * (1.0 + w.gust) + 1e-4, "gust out of band");
    }
}
