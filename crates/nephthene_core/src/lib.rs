//! Nephthene core: the pure half of a weather system (no render deps, so `cargo test` runs it headless).
//! Clock -> sun and moon directions, exposure curve, wind, the weather map, rain / storm / audio maths, dashboard state.
//! Every tunable is in [`config::Config`]. The Bevy plugin crate `nephthene` is the render half.
//!
//! Compass: north = +Z, east = +X, azimuth clockwise from north.
use glam::{Vec2, Vec3};

pub mod config;
pub mod rain;
pub mod audio;
pub mod orographic;
pub mod dash;
pub mod storm;
pub mod storm_fx;
pub mod fog;
pub mod lightning;
pub mod air;
pub mod climate;
pub mod terrain;
pub use config::{AirCfg, Clock, ClimateCfg, Clouds, Config, Fog, MapCfg, Moon, OroCfg, RainCfg, Shafts, Sky, Stars, Wind};
use serde::Deserialize;

/// Weather at one place.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Local { pub coverage: f32, pub precip: f32, pub dust: f32 }

/// The weather map: an RGBA8 image (R coverage, G precip, B cloud type, A dust) with its world extent. Map metres, rows
/// north to south. The GPU samples the same image for the clouds. Load one with `load` (weather_map.png + weather_map.json
/// in a directory) or take the built-in procedural one from `procedural`. `wrap` repeats the map past its edges (the plugin copies config `map.wrap`); false holds the edge texels.
#[derive(Clone)]
pub struct WeatherMap { pub width: u32, pub height: u32, pub min_x: f32, pub max_z: f32, pub size_x: f32, pub size_z: f32, pub rgba: Vec<u8>, pub wrap: bool }

/// Read and parse a json file; panics with the path on failure.
pub fn load_json<T: for<'a> Deserialize<'a>>(path: &str) -> T {
    let s = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[derive(Deserialize)]
struct Extent { width: u32, height: u32, min_x: f32, max_z: f32, size_x: f32, size_z: f32 }

impl WeatherMap {
    /// <dir>/weather_map.png + .json.
    pub fn load(dir: &str) -> Self {
        Self::load_named(dir, "weather_map")
    }

    /// An RGBA8 map `<dir>/<stem>.png` with its extent in `<dir>/<stem>.json` (the climate map, `climate_map`, uses the same format).
    pub fn load_named(dir: &str, stem: &str) -> Self {
        let e: Extent = load_json(&format!("{dir}/{stem}.json"));
        let png = std::fs::read(format!("{dir}/{stem}.png")).unwrap_or_else(|e| panic!("{stem}.png: {e}"));
        let mut r = png::Decoder::new(std::io::Cursor::new(png)).read_info().unwrap_or_else(|e| panic!("decode {stem}.png: {e}"));
        let mut rgba = vec![0; r.output_buffer_size().unwrap_or_else(|| panic!("{stem}.png size"))];
        let info = r.next_frame(&mut rgba).unwrap_or_else(|e| panic!("{stem}.png pixels: {e}"));
        rgba.truncate(info.buffer_size());
        assert_eq!((info.color_type, info.bit_depth), (png::ColorType::Rgba, png::BitDepth::Eight), "{stem}.png must be RGBA8");
        assert_eq!((info.width, info.height, rgba.len()), (e.width, e.height, (e.width * e.height * 4) as usize), "{stem}.png vs {stem}.json");
        WeatherMap { width: e.width, height: e.height, min_x: e.min_x, max_z: e.max_z, size_x: e.size_x, size_z: e.size_z, rgba, wrap: false }
    }

    /// Built-in map for [`MapCfg`]: smooth coverage swell around `default_coverage`, one rain blob when `default_precip` > 0. Texel
    /// centres sit at u = i / n, so whole `default_cycles` tile and the blob's distance wraps.
    /// ponytail: product of two sines is a placeholder; supply a PNG for a real map.
    pub fn procedural(m: &config::MapCfg) -> Self {
        let n = m.default_px.max(2);
        let ext = m.default_extent_m;
        let mut rgba = Vec::with_capacity((n * n * 4) as usize);
        for j in 0..n {
            for i in 0..n {
                let (u, v) = (i as f32 / n as f32, j as f32 / n as f32);
                let (cy, ph, bl) = (m.default_cycles, m.default_phase, m.default_blob);
                let swell = (std::f32::consts::TAU * cy[0] * u + ph[0]).sin() * (std::f32::consts::TAU * cy[1] * v + ph[1]).sin();
                let cov = (m.default_coverage + m.default_coverage_swing * swell).clamp(0.0, 1.0);
                let wrap = |d: f32| d - d.round();
                let blob = (1.0 - wrap(u - bl[0]).hypot(wrap(v - bl[1])) / bl[2]).clamp(0.0, 1.0);
                let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                rgba.extend_from_slice(&[b(cov), b(m.default_precip * blob), b(m.default_type), b(m.default_dust)]);
            }
        }
        // Centred on map (0, 0); with a zero world offset the world origin is mid-map.
        WeatherMap { width: n, height: n, min_x: -ext / 2.0, max_z: ext / 2.0, size_x: ext, size_z: ext, rgba, wrap: false }
    }

    /// Bilinear RGBA in 0..1 at map (x, z), clamped at the edges (wrapped when `wrap`).
    pub fn rgba(&self, x: f32, z: f32) -> [f32; 4] {
        let (x0, x1, tx) = texel_axis((x - self.min_x) / self.size_x * self.width as f32 - 0.5, self.width, self.wrap);
        let (z0, z1, tz) = texel_axis((self.max_z - z) / self.size_z * self.height as f32 - 0.5, self.height, self.wrap);
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

/// One axis of a bilinear read at texel coordinate `f` (texel centres at whole numbers) of `n` texels: the two texels to blend and the
/// weight of the second; held at the edge, or wrapped across it.
pub(crate) fn texel_axis(f: f32, n: u32, wrap: bool) -> (u32, u32, f32) {
    if wrap {
        let f0 = f.floor();
        let i = (f0 as i64).rem_euclid(n as i64) as u32;
        (i, (i + 1) % n, f - f0)
    } else {
        let f = f.clamp(0.0, (n - 1) as f32);
        let i = f as u32;
        (i, (i + 1).min(n - 1), f - i as f32)
    }
}

/// Map coordinates of world (x, z): the world offset, less how far the weather has moved downwind (`shift`, m).
pub fn map_xz(m: &config::MapCfg, shift: Vec2, x: f32, z: f32) -> [f32; 2] {
    [x + m.world_offset_m[0] - shift.x, z + m.world_offset_m[1] - shift.y]
}

/// The wind the weather map rides (m/s, world XZ): the direction of the live wind `now` (gusts scale its speed, never turn it) at
/// the mean speed `mean_mps` (no gusts). ponytail: a gust factor below zero (`wind.gust` x the gust weights > 1) would flip it;
/// take the direction from the config once the wind is a field (Phase 2).
pub fn map_wind(now: Vec3, mean_mps: f32) -> Vec2 { Vec2::new(now.x, now.z).normalize_or_zero() * mean_mps }

/// One step of weather map advection: the map moves `map.advect` x `wind` (m/s, world XZ; the mean wind, never the gusts) for
/// `dt_s` game seconds. A uniform wind advects a field by pure translation, so the map is shifted, not resampled. A wrapped map
/// keeps the shift within one period (the same weather, and f32 keeps its precision over long sessions).
pub fn advect(m: &config::MapCfg, map: &WeatherMap, shift: Vec2, wind: Vec2, dt_s: f32) -> Vec2 {
    let s = shift + wind * m.advect * dt_s;
    if map.wrap { Vec2::new(s.x.rem_euclid(map.size_x), s.y.rem_euclid(map.size_z)) } else { s }
}

/// Unit vector toward a body at (elevation, azimuth) in degrees.
pub fn dir(el_deg: f32, az_deg: f32) -> Vec3 {
    let (el, az) = (el_deg.to_radians(), az_deg.to_radians());
    Vec3::new(el.cos() * az.sin(), el.sin(), el.cos() * az.cos())
}

/// Solar declination, degrees (Cooper's approximation).
pub(crate) fn declination(tilt_deg: f32, day_of_year: f32) -> f32 { -tilt_deg * (std::f32::consts::TAU * (day_of_year + 10.0) / 365.0).cos() }

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

/// Share of the newest finished cookie shown on this frame of a redraw cycle: rises 1/every .. 1 so each cycle's change spreads over its frames.
pub fn shadow_mix(frame: u32, every: u32, blend: bool) -> f32 {
    if !blend || every <= 1 { return 1.0; }
    (frame % every + 1) as f32 / every as f32
}

/// UV offset from a texel of the cookie at `cur` to the texel of the cookie at `prev` covering the same light ray (uv maps to local l = (1 - 2u, 2v - 1)).
pub fn shadow_prev_shift(cur: Vec3, prev: Vec3, right: Vec3, up: Vec3, half_m: f32) -> [f32; 2] {
    let d = cur - prev;
    [-d.dot(right) / (2.0 * half_m), d.dot(up) / (2.0 * half_m)]
}

#[cfg(test)]
mod shadow_tests {
    use super::*;
    fn basis(sun: Vec3) -> (Vec3, Vec3) {
        let right = Vec3::Y.cross(sun).normalize();
        (right, sun.cross(right))
    }
    #[test]
    fn shadow_mix_rises_to_one_each_cycle() {
        let m: Vec<f32> = (0..4).map(|f| shadow_mix(f, 4, true)).collect();
        assert_eq!(m, [0.25, 0.5, 0.75, 1.0]);
        assert_eq!(shadow_mix(4, 4, true), 0.25);
        assert_eq!(shadow_mix(0, 4, false), 1.0);
        assert_eq!(shadow_mix(0, 1, true), 1.0);
    }
    #[test]
    fn shadow_prev_shift_finds_the_same_ray() {
        let sun = Vec3::new(0.3, 0.6, 0.2).normalize();
        let (right, up) = basis(sun);
        let half = 10000.0;
        let cur = right * 390.0 + up * -780.0;
        let prev = right * -195.0 + up * 195.0;
        let (u, v) = (0.3f32, 0.7f32);
        let s = shadow_prev_shift(cur, prev, right, up, half);
        let (u2, v2) = (u + s[0], v + s[1]);
        let p = cur + right * (1.0 - 2.0 * u) * half + up * (2.0 * v - 1.0) * half;
        let q = prev + right * (1.0 - 2.0 * u2) * half + up * (2.0 * v2 - 1.0) * half;
        assert!((p - q).length() < 0.01, "{p:?} vs {q:?}");
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

#[cfg(test)]
mod map_tests {
    use super::*;
    use config::MapCfg;

    // Where the weather has got to after 10 game minutes of steady wind must be within 1 m of where the wind carried it: far under one
    // texel of any practical map (the built-in one's is 3.1 km), so the clouds and the weather here cannot visibly part.
    // The loop takes the plugin's path (map_wind of the gusting wind), so a sign flip or a gust leak there fails it too.
    const DRIFT_BUDGET_M: f32 = 1.0;

    #[test]
    fn map_rides_the_mean_wind() {
        let w = config::Wind::default();
        let v3 = -dir(0.0, w.from_deg) * w.speed_mps;
        let v = Vec2::new(v3.x, v3.z);
        // 60 fps at 15 game seconds per real second (the default clock), 600 game seconds.
        let (dt, frames) = (15.0 / 60.0, 2400);
        let travel = v * dt * frames as f32;
        let drift_error = |m: &MapCfg| {
            let map = WeatherMap::procedural(m);
            let mut s = Vec2::from(m.start_shift_m);
            // As the plugin does: the gusting wind of each frame (60 fps real time), at the mean speed.
            for i in 0..frames { s = advect(m, &map, s, map_wind(wind(&w, i as f32 / 60.0), w.speed_mps), dt); }
            // The weather seen at world w0 at the start is now seen downwind, at w0 + travel.
            for w0 in [Vec2::new(0.0, 0.0), Vec2::new(-4000.0, 2500.0), Vec2::new(12000.0, -7000.0)] {
                let [ax, az] = map_xz(m, Vec2::from(m.start_shift_m), w0.x, w0.y);
                let [bx, bz] = map_xz(m, s, w0.x + travel.x, w0.y + travel.y);
                let (a, b) = (map.sample(ax, az), map.sample(bx, bz));
                if m.advect == 1.0 { assert!((a.coverage - b.coverage).abs() < 1e-3, "coverage at the carried point {} vs {}", b.coverage, a.coverage); }
            }
            (s - Vec2::from(m.start_shift_m) - travel).length()
        };
        // Unwrapped, so the shift is the plain distance travelled (a wrapped shift folds into one period).
        let on = drift_error(&MapCfg { advect: 1.0, wrap: false, ..MapCfg::default() });
        let red = drift_error(&MapCfg { advect: 0.0, wrap: false, ..MapCfg::default() });
        println!("weather drift error after 600 s: advect 1 {on:.4} m, red arm (advect 0) {red:.1} m (budget {DRIFT_BUDGET_M} m)");
        assert!(on <= DRIFT_BUDGET_M, "advected map is {on} m from where the wind carried it, budget {DRIFT_BUDGET_M} m");
        assert!(red > DRIFT_BUDGET_M, "red arm must leave the weather behind, got {red} m (budget {DRIFT_BUDGET_M} m)");
    }

    // The built-in map tiles: across the wrap seam (last column to first, last row to first) the coverage steps no more than the
    // largest step between neighbours inside the map, plus one 8-bit level for rounding. A seam steeper than any texture step is a
    // visible line in the sky once the map wraps. Red arm: the old non-whole cycle counts (7.1 and 5.3 rad over the map).
    const SEAM_LEVELS: f32 = 1.0;

    #[test]
    fn built_in_map_tiles() {
        let steps = |m: &MapCfg| {
            let map = WeatherMap::procedural(m);
            let (w, h) = (map.width as usize, map.height as usize);
            let at = |i: usize, j: usize| map.rgba[(j * w + i) * 4] as f32;
            let (mut inner, mut seam) = (0f32, 0f32);
            for j in 0..h { for i in 0..w {
                let (dx, dy) = ((at(i, j) - at((i + 1) % w, j)).abs(), (at(i, j) - at(i, (j + 1) % h)).abs());
                if i + 1 < w { inner = inner.max(dx) } else { seam = seam.max(dx) }
                if j + 1 < h { inner = inner.max(dy) } else { seam = seam.max(dy) }
            }}
            (inner, seam)
        };
        let (inner, seam) = steps(&MapCfg::default());
        let (red_inner, red_seam) = steps(&MapCfg { default_cycles: [7.1 / std::f32::consts::TAU, 5.3 / std::f32::consts::TAU], ..MapCfg::default() });
        println!("built-in map, coverage levels: seam step {seam} vs inner {inner}; red arm (old cycles) seam {red_seam} vs inner {red_inner} (budget inner + {SEAM_LEVELS})");
        assert!(seam <= inner + SEAM_LEVELS, "the built-in map does not tile: seam step {seam} > inner {inner} + {SEAM_LEVELS}");
        assert!(red_seam > red_inner + SEAM_LEVELS, "red arm tiles: seam {red_seam} vs inner {red_inner}");
    }

    // One map period away must read the same weather when wrapped: 1e-4 is f32 rounding of the 0..1 channel values.
    const WRAP_BUDGET: f32 = 1e-4;

    #[test]
    fn wrap_tiles_the_map() {
        let m = MapCfg::default();
        let mut map = WeatherMap::procedural(&m);
        let worst = |map: &WeatherMap| {
            let mut d: f32 = 0.0;
            for i in 0..9 { for j in 0..9 {
                let (x, z) = (map.min_x + map.size_x * (i as f32 + 0.37) / 9.0, map.max_z - map.size_z * (j as f32 + 0.61) / 9.0);
                let (a, bx, bz) = (map.rgba(x, z)[0], map.rgba(x + map.size_x, z)[0], map.rgba(x, z - map.size_z)[0]);
                d = d.max((a - bx).abs()).max((a - bz).abs());
            }}
            d
        };
        map.wrap = true;
        let wrapped = worst(&map);
        // Continuity across the seam: on the map edge, halfway between the last and the first texel centres, the read is their mean.
        let n = map.width;
        let z_row0 = map.max_z - 0.5 * map.size_z / map.height as f32;
        let texel = |i: u32| map.rgba[(i * 4) as usize] as f32 / 255.0;
        let seam = (map.rgba(map.min_x, z_row0)[0] - 0.5 * (texel(n - 1) + texel(0))).abs();
        // A long advection keeps a wrapped shift inside one period.
        let s = advect(&MapCfg { advect: 1.0, ..m.clone() }, &map, Vec2::ZERO, Vec2::new(6.0, -4.0), 1.0e6);
        map.wrap = false;
        let clamped = worst(&map);
        println!("one period away: wrapped {wrapped:.6}, red arm (clamped) {clamped:.4} (budget {WRAP_BUDGET}); seam {seam:.6}; wrapped shift {s:?}");
        assert!(wrapped <= WRAP_BUDGET, "wrapped map differs {wrapped} one period away, budget {WRAP_BUDGET}");
        assert!(seam <= WRAP_BUDGET, "seam read {seam} off the mean of the edge texels");
        assert!(s.x >= 0.0 && s.x < map.size_x && s.y >= 0.0 && s.y < map.size_z, "wrapped shift {s:?} outside one period");
        assert!(clamped > WRAP_BUDGET, "red arm (clamped) must differ one period away, got {clamped}");
    }
}
