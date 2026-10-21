//! Nephthene core: the pure half of a weather system (no render deps, so `cargo test` runs it headless).
//! Clock -> sun and moon directions, exposure curve, wind, the weather map, rain / storm / audio maths, dashboard state.
//! Every tunable is in [`config::Config`]. The Bevy plugin crate `nephthene` is the render half.
//!
//! Compass: north = +Z, east = +X, azimuth clockwise from north.
use glam::Vec3;

pub mod config;
pub mod rain;
pub mod audio;
pub mod orographic;
pub mod dash;
pub mod storm;
pub mod storm_fx;
pub mod fog;
pub use config::{Clock, Clouds, Config, Fog, MapCfg, Moon, RainCfg, Shafts, Sky, Stars, Wind};
use serde::Deserialize;

/// Weather at one place.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Local { pub coverage: f32, pub precip: f32, pub dust: f32 }

/// The weather map: an RGBA8 image (R coverage, G precip, B cloud type, A dust) with its world extent. Map metres, rows
/// north to south. The GPU samples the same image for the clouds. Load one with `load` (weather_map.png + weather_map.json
/// in a directory) or take the built-in procedural one from `procedural`.
#[derive(Clone)]
pub struct WeatherMap { pub width: u32, pub height: u32, pub min_x: f32, pub max_z: f32, pub size_x: f32, pub size_z: f32, pub rgba: Vec<u8> }

/// Read and parse a json file; panics with the path on failure.
pub fn load_json<T: for<'a> Deserialize<'a>>(path: &str) -> T {
    let s = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[derive(Deserialize)]
struct Extent { width: u32, height: u32, min_x: f32, max_z: f32, size_x: f32, size_z: f32 }

impl WeatherMap {
    pub fn load(dir: &str) -> Self {
        let e: Extent = load_json(&format!("{dir}/weather_map.json"));
        let png = std::fs::read(format!("{dir}/weather_map.png")).expect("weather_map.png");
        let mut r = png::Decoder::new(std::io::Cursor::new(png)).read_info().expect("decode weather_map.png");
        let mut rgba = vec![0; r.output_buffer_size().expect("weather_map.png size")];
        let info = r.next_frame(&mut rgba).expect("weather_map.png pixels");
        rgba.truncate(info.buffer_size());
        assert_eq!((info.color_type, info.bit_depth), (png::ColorType::Rgba, png::BitDepth::Eight), "weather_map.png must be RGBA8");
        assert_eq!((info.width, info.height, rgba.len()), (e.width, e.height, (e.width * e.height * 4) as usize), "weather_map.png vs weather_map.json");
        WeatherMap { width: e.width, height: e.height, min_x: e.min_x, max_z: e.max_z, size_x: e.size_x, size_z: e.size_z, rgba }
    }

    /// Built-in map for [`MapCfg`]: smooth coverage swell around `default_coverage`, one rain blob when `default_precip` > 0.
    /// ponytail: product of two sines is a placeholder; supply a PNG for a real map.
    pub fn procedural(m: &config::MapCfg) -> Self {
        let n = m.default_px.max(2);
        let ext = m.default_extent_m;
        let mut rgba = Vec::with_capacity((n * n * 4) as usize);
        for j in 0..n {
            for i in 0..n {
                let (u, v) = (i as f32 / (n - 1) as f32, j as f32 / (n - 1) as f32);
                let swell = (u * 7.1 + 0.4).sin() * (v * 5.3 + 1.1).sin();
                let cov = (m.default_coverage + m.default_coverage_swing * swell).clamp(0.0, 1.0);
                let blob = (1.0 - ((u - 0.7).powi(2) + (v - 0.3).powi(2)).sqrt() * 4.0).clamp(0.0, 1.0);
                let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                rgba.extend_from_slice(&[b(cov), b(m.default_precip * blob), b(m.default_type), b(m.default_dust)]);
            }
        }
        // Centred on map (0, 0); with a zero world offset the world origin is mid-map.
        WeatherMap { width: n, height: n, min_x: -ext / 2.0, max_z: ext / 2.0, size_x: ext, size_z: ext, rgba }
    }

    /// Bilinear RGBA in 0..1 at map (x, z), clamped at the edges.
    pub fn rgba(&self, x: f32, z: f32) -> [f32; 4] {
        let fx = ((x - self.min_x) / self.size_x * self.width as f32 - 0.5).clamp(0.0, (self.width - 1) as f32);
        let fz = ((self.max_z - z) / self.size_z * self.height as f32 - 0.5).clamp(0.0, (self.height - 1) as f32);
        let (x0, z0) = (fx as u32, fz as u32);
        let (x1, z1) = ((x0 + 1).min(self.width - 1), (z0 + 1).min(self.height - 1));
        let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
        let px = |x: u32, z: u32, c: usize| self.rgba[((z * self.width + x) * 4) as usize + c] as f32 / 255.0;
        std::array::from_fn(|c| {
            let a = px(x0, z0, c) + (px(x1, z0, c) - px(x0, z0, c)) * tx;
            let b = px(x0, z1, c) + (px(x1, z1, c) - px(x0, z1, c)) * tx;
            a + (b - a) * tz
        })
    }

    /// Weather at map (x, z).
    pub fn sample(&self, x: f32, z: f32) -> Local {
        let [coverage, precip, _, dust] = self.rgba(x, z);
        Local { coverage, precip, dust }
    }
}

/// Unit vector toward a body at (elevation, azimuth) in degrees.
pub fn dir(el_deg: f32, az_deg: f32) -> Vec3 {
    let (el, az) = (el_deg.to_radians(), az_deg.to_radians());
    Vec3::new(el.cos() * az.sin(), el.sin(), el.cos() * az.cos())
}

/// Solar declination, degrees (Cooper's approximation).
fn declination(tilt_deg: f32, day_of_year: f32) -> f32 { -tilt_deg * (std::f32::consts::TAU * (day_of_year + 10.0) / 365.0).cos() }

/// (elevation, azimuth) in degrees of a body at hour angle `h_deg` (0 = on the meridian, + = west) and declination `dec_deg`.
fn alt_az(lat_deg: f32, dec_deg: f32, h_deg: f32) -> (f32, f32) {
    let (p, d, h) = (lat_deg.to_radians(), dec_deg.to_radians(), h_deg.to_radians());
    let el = (p.sin() * d.sin() + p.cos() * d.cos() * h.cos()).asin();
    let az = (-h.sin()).atan2(d.tan() * p.cos() - p.sin() * h.cos());
    (el.to_degrees(), az.to_degrees().rem_euclid(360.0))
}

/// Sun (elevation, azimuth) at local solar `hour`.
pub fn sun(c: &Clock, hour: f32) -> (f32, f32) { alt_az(c.latitude_deg, declination(c.axial_tilt_deg, c.day_of_year), 15.0 * (hour - 12.0)) }

/// Moon (elevation, azimuth): trails the sun by `phase` of a turn, on the ecliptic (declination swings from the sun's
/// at new moon to its mirror at full). ponytail: no orbit inclination; full moon rises at sunset, which is what the eye checks.
pub fn moon(c: &Clock, m: &Moon, hour: f32) -> (f32, f32) {
    let dec = declination(c.axial_tilt_deg, c.day_of_year) * (std::f32::consts::TAU * m.phase).cos();
    alt_az(c.latitude_deg, dec, 15.0 * (hour - 12.0) - 360.0 * m.phase)
}

/// Moon illuminance in lux: lit fraction of the disc times the full-moon value.
pub fn moon_lux(m: &Moon) -> f32 { m.full_lux * 0.5 * (1.0 - (std::f32::consts::TAU * m.phase).cos()) }

/// Piecewise-linear lookup, clamped at both ends.
pub fn curve(points: &[[f32; 2]], x: f32) -> f32 {
    let i = points.partition_point(|p| p[0] <= x);
    if i == 0 { return points[0][1]; }
    if i == points.len() { return points[i - 1][1]; }
    let ([x0, y0], [x1, y1]) = (points[i - 1], points[i]);
    y0 + (y1 - y0) * (x - x0) / (x1 - x0)
}

/// Wind velocity (m/s, world XZ) at time `t`: blows *from* `from_deg`, speed modulated by two incommensurate sines.
/// ponytail: one global vector; a world-space gust field when something spatial (grass, dust) samples it.
pub fn wind(w: &Wind, t: f32) -> Vec3 {
    let k = std::f32::consts::TAU / w.gust_period_s;
    let g = w.gust_weights[0] * (k * t).sin() + w.gust_weights[1] * (k * w.gust_ratio * t + w.gust_phase).sin();
    -dir(0.0, w.from_deg) * w.speed_mps * (1.0 + w.gust * g)
}

/// Frame-rate independent approach of `cur` toward `target` with time constant `tau_s`.
pub fn smooth(cur: Local, target: Local, dt: f32, tau_s: f32) -> Local {
    let a = 1.0 - (-dt / tau_s).exp();
    Local { coverage: cur.coverage + (target.coverage - cur.coverage) * a, precip: cur.precip + (target.precip - cur.precip) * a,
        dust: cur.dust + (target.dust - cur.dust) * a }
}

/// Cloud shadow cookie geometry. Bevy projects a `DirectionalLightTexture` through the light's inverse GlobalTransform: a world point P
/// has local (lx, ly) = (inv * P).xy, uv = (0.5 - 0.5 lx, 0.5 + 0.5 ly), so the map covers the light plane within |l| <= 1, i.e. `half_m`
/// when the light's scale is `half_m`. `right`/`up` are the light's local X/Y axes (Transform::right / up), `sun` points toward the sun.
/// Local (lx, ly) in -1..1 at the centre of texel (i, j) of an `n` x `n` map (row 0 = top).
pub fn shadow_texel_local(i: u32, j: u32, n: u32) -> [f32; 2] {
    let (u, v) = ((i as f32 + 0.5) / n as f32, (j as f32 + 0.5) / n as f32);
    [1.0 - 2.0 * u, 2.0 * v - 1.0]
}

/// Inverse of `shadow_texel_local`: the texel containing local (lx, ly), unclamped.
pub fn shadow_local_texel(l: [f32; 2], n: u32) -> [i32; 2] {
    [((0.5 - 0.5 * l[0]) * n as f32).floor() as i32, ((0.5 + 0.5 * l[1]) * n as f32).floor() as i32]
}

/// Light-plane origin (a world point, zero component along `sun`) that centres the map on `cam`, snapped to whole texels of `2*half_m/n` m
/// so the map does not swim as the camera moves.
pub fn shadow_origin(cam: Vec3, right: Vec3, up: Vec3, half_m: f32, n: u32) -> Vec3 {
    let texel = 2.0 * half_m / n as f32;
    let snap = |a: f32| (a / texel).round() * texel;
    right * snap(cam.dot(right)) + up * snap(cam.dot(up))
}

/// The ground (y = 0) point under the sun-aligned ray through the light-plane point of local `l`.
pub fn shadow_ground(origin: Vec3, right: Vec3, up: Vec3, sun: Vec3, half_m: f32, l: [f32; 2]) -> Vec3 {
    let p = origin + right * (l[0] * half_m) + up * (l[1] * half_m);
    p - sun * (p.y / sun.y)
}

/// Samples through the layer for the shadow march: `base` where the sun is high, more as the slant path (top-bottom)/sun_y stretches, capped.
/// Deterministic in the sun height (no jitter), so frame-to-frame the same texel marches the same positions.
pub fn shadow_samples(c: &config::Clouds, layer_m: f32, sun_y: f32) -> u32 {
    let (base, slant) = (c.shadow_samples, layer_m / sun_y.max(0.02));
    ((slant / c.shadow_step_m).ceil() as u32).clamp(base, c.shadow_samples_max.max(base))
}

#[cfg(test)]
mod shadow_tests {
    use super::*;
    fn basis(sun: Vec3) -> (Vec3, Vec3) {
        let right = Vec3::Y.cross(sun).normalize();
        (right, sun.cross(right))
    }
    #[test]
    fn samples_grow_with_slant_and_cap() {
        let c = config::Clouds::default();
        let (step, max) = (c.shadow_step_m, c.shadow_samples_max);
        let hi = shadow_samples(&c, 1500.0, 0.95);
        let lo = shadow_samples(&c, 1500.0, 0.2);
        assert_eq!(hi, 10.max(((1500.0f32 / 0.95) / step).ceil() as u32));
        assert!(lo > hi && lo <= max);
        assert_eq!(shadow_samples(&c, 1500.0, 0.001), max);
        assert_eq!(shadow_samples(&c, 100.0, 1.0), 10);
    }
    #[test]
    fn texel_centre_maps_back() {
        for (i, j) in [(0, 0), (255, 3), (511, 511), (100, 400)] {
            assert_eq!(shadow_local_texel(shadow_texel_local(i, j, 512), 512), [i as i32, j as i32]);
        }
    }
    #[test]
    fn snapping_is_stable_and_centred() {
        let sun = dir(40.0, 200.0);
        let (r, u) = basis(sun);
        let (half, n) = (10000.0, 512);
        let texel = 2.0 * half / n as f32;
        let cam = Vec3::new(123.0, 1.5, -456.0);
        let o = shadow_origin(cam, r, u, half, n);
        // Moves by under a texel snap to the same origin or a neighbour, never a jump.
        let o2 = shadow_origin(cam + r * (0.2 * texel), r, u, half, n);
        assert!((o2 - o).length() <= texel + 1e-3);
        // Centred: the camera's light-plane coordinates are within half a texel of the origin.
        assert!((cam - o).dot(r).abs() <= 0.5 * texel + 1e-3 && (cam - o).dot(u).abs() <= 0.5 * texel + 1e-3);
        // Centre of the map lies on the ground along the sun ray; its plane coordinates match the origin.
        let g = shadow_ground(o, r, u, sun, half, [0.0, 0.0]);
        assert!(g.y.abs() < 1e-3 && ((g - o).cross(sun)).length() < 1e-2);
    }
}
