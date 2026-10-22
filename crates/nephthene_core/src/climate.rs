//! Climate: what a place is like, as opposed to what the weather is doing there. A second RGBA8 map in the weather map's format,
//! fixed to the world (read at world + `map.world_offset_m`, never shifted): R mean temperature offset and G humidity (signed, byte
//! 128 neutral), B ground snow and A ground wetness (0..1). It localises the air config per place: the restore target, the air mass
//! dew point and the sun's heating (snow albedo). Pure; `air::Field` reads it per cell, the plugin at the camera.
use crate::{WeatherMap, config::{AirCfg, ClimateCfg}};

/// The climate at one place: mean temperature offset (K), humidity -1 (dry) .. 1 (humid), ground snow and ground wetness 0..1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Place { pub temp_k: f32, pub humidity: f32, pub snow: f32, pub wetness: f32 }

/// A signed channel read 0..1 (byte / 255) as -1..1: byte 128 is 0, 0 is -1 (clamped), 255 is 1.
fn signed(v: f32) -> f32 { ((v * 255.0 - 128.0) / 127.0).clamp(-1.0, 1.0) }

/// The climate at climate-map (x, z) = world + `map.world_offset_m`: the map when given (bilinear, held at its edges), else `uniform`;
/// neutral when `ignore`.
pub fn place(cc: &ClimateCfg, map: Option<&WeatherMap>, x: f32, z: f32) -> Place {
    if cc.ignore { return Place::default(); }
    match map {
        Some(m) => { let [r, g, b, a] = m.rgba(x, z); Place { temp_k: cc.temp_range_k * signed(r), humidity: signed(g), snow: b, wetness: a } }
        None => { let [t, h, s, w] = cc.uniform; Place { temp_k: t, humidity: h.clamp(-1.0, 1.0), snow: s.clamp(0.0, 1.0), wetness: w.clamp(0.0, 1.0) } }
    }
}

/// The air config at place `p`: `mean_c` raised by the offset, `dew_below_c` toward `dry_dew_below_c` (humidity < 0) or
/// `wet_dew_below_c` (> 0), and `heat_k_per_h` scaled by the share of sunlight the ground absorbs compared with bare ground.
/// A neutral place returns `ac` unchanged.
pub fn air(ac: &AirCfg, cc: &ClimateCfg, p: &Place) -> AirCfg {
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let dew_below_c = if p.humidity < 0.0 { lerp(ac.dew_below_c, cc.dry_dew_below_c, -p.humidity) } else { lerp(ac.dew_below_c, cc.wet_dew_below_c, p.humidity) };
    let albedo = lerp(cc.bare_albedo, cc.snow_albedo, p.snow);
    // ponytail: the floor keeps a bare albedo of 1 from dividing by zero; such ground absorbs nothing either way.
    let absorbed = (1.0 - albedo) / (1.0 - cc.bare_albedo).max(1e-3);
    AirCfg { mean_c: ac.mean_c + p.temp_k, dew_below_c, heat_k_per_h: ac.heat_k_per_h * absorbed, ..ac.clone() }
}

/// Ground wetness 0..1 at place `p`: the wetter of the climate's and the rain's (`rain_wetness`).
pub fn wetness(p: &Place, rain_wetness: f32) -> f32 { p.wetness.max(rain_wetness) }

#[cfg(test)]
mod tests {
    use super::*;

    fn one(t: [u8; 4]) -> WeatherMap { WeatherMap { width: 1, height: 1, min_x: -50.0, max_z: 50.0, size_x: 100.0, size_z: 100.0, rgba: t.to_vec(), wrap: false } }

    #[test]
    fn bytes_decode_with_128_neutral() {
        let cc = ClimateCfg::default();
        let n = place(&cc, Some(&one([128, 128, 0, 0])), 0.0, 0.0);
        assert!(n.temp_k.abs() < 1e-3 && n.humidity.abs() < 1e-4 && n.snow == 0.0 && n.wetness == 0.0, "128 must be neutral: {n:?}");
        let p = place(&cc, Some(&one([0, 255, 255, 51])), 0.0, 0.0);
        assert!((p.temp_k + 25.0).abs() < 1e-4 && (p.humidity - 1.0).abs() < 1e-4 && p.snow == 1.0 && (p.wetness - 0.2).abs() < 1e-4, "{p:?}");
        assert!((place(&cc, Some(&one([255, 0, 0, 0])), 0.0, 0.0).temp_k - 25.0).abs() < 1e-4);
    }

    #[test]
    fn a_neutral_place_keeps_the_air() {
        let (ac, cc) = (AirCfg::default(), ClimateCfg::default());
        assert_eq!(place(&cc, None, 0.0, 0.0), Place::default(), "the default uniform climate is neutral");
        assert_eq!(air(&ac, &cc, &Place::default()), ac, "a neutral place must leave the air config exactly as it is");
    }

    #[test]
    fn places_shape_the_air() {
        let (ac, cc) = (AirCfg::default(), ClimateCfg::default());
        let at = |temp_k: f32, humidity: f32, snow: f32| air(&ac, &cc, &Place { temp_k, humidity, snow, wetness: 0.0 });
        assert_eq!(at(-10.0, 0.0, 0.0).mean_c, ac.mean_c - 10.0);
        assert_eq!((at(0.0, -1.0, 0.0).dew_below_c, at(0.0, 1.0, 0.0).dew_below_c), (cc.dry_dew_below_c, cc.wet_dew_below_c));
        assert!((at(0.0, -0.5, 0.0).dew_below_c - 0.5 * (ac.dew_below_c + cc.dry_dew_below_c)).abs() < 1e-5);
        // Snow absorbs (1 - 0.7) / (1 - 0.2) = 0.375 of the bare ground's sunlight.
        assert!((at(0.0, 0.0, 1.0).heat_k_per_h - ac.heat_k_per_h * 0.375).abs() < 1e-5);
        assert_eq!(wetness(&Place { wetness: 0.3, ..Place::default() }, 0.6), 0.6);
        assert_eq!(wetness(&Place { wetness: 0.3, ..Place::default() }, 0.1), 0.3);
    }

    #[test]
    fn ignore_reads_the_neutral_climate() {
        let cc = ClimateCfg { uniform: [5.0, -1.0, 1.0, 1.0], ignore: true, ..ClimateCfg::default() };
        assert_eq!(place(&cc, None, 0.0, 0.0), Place::default());
        assert_eq!(place(&cc, Some(&one([0, 0, 255, 255])), 0.0, 0.0), Place::default());
        let on = ClimateCfg { ignore: false, ..cc };
        assert_eq!(place(&on, None, 0.0, 0.0), Place { temp_k: 5.0, humidity: -1.0, snow: 1.0, wetness: 1.0 });
    }
}
