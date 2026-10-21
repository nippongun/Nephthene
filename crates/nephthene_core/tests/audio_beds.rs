//! The ambience mapping: wind and rain bed gains follow speed and intensity (the bed gain is crossfade weight x overall gain).
use nephthene_core::audio::*;

fn wind_beds(c: &Cfg, speed: f32) -> (f32, f32, f32, f32) {
    let m = wind_mix(c, speed, 0.0);
    (m.calm * m.gain, m.steady * m.gain, m.gale * m.gain, m.rate)
}

#[test]
fn wind_beds_follow_speed() {
    let c = Cfg::default();
    let (calm, mid, gale) = (wind_beds(&c, 1.0), wind_beds(&c, 6.0), wind_beds(&c, 18.0));
    let total = |b: (f32, f32, f32, f32)| b.0 + b.1 + b.2;
    assert!(total(gale) > 2.0 * total(calm), "wind not louder with speed: {} -> {}", total(calm), total(gale));
    assert!(gale.3 > calm.3 + 0.2, "wind pitch constant");
    assert!(calm.0 > calm.2 * 5.0 && gale.2 > gale.0 * 5.0, "layers do not crossfade");
    assert!(mid.1 > mid.0 * 5.0 && mid.1 > mid.2 * 5.0, "the default 6 m/s is the steady bed");
}

#[test]
fn rain_beds_follow_intensity() {
    let c = Cfg::default();
    let dry = rain_mix(&c, 0.0);
    assert!(dry.light < 1e-4 && dry.heavy < 1e-4 && dry.gain < 1e-4, "rain bed audible when dry");
    let (light, heavy) = (rain_mix(&c, 0.15), rain_mix(&c, 1.0));
    assert!(light.light > 5.0 * light.heavy && heavy.heavy > 5.0 * heavy.light, "rain layers do not crossfade");
    assert!(heavy.heavy * heavy.gain > light.light * light.gain, "heavy rain not louder than light");
}
