//! Rain, the pure half: intensity from the weather sample, drop kinematics, streak length, wetness, and the per-drop
//! hash/position functions that assets/shaders/rain.wesl mirrors line for line (the shader is the renderer; this is its reference).
//! Every tunable is in [`crate::config::RainCfg`] / [`crate::config::Shafts`]. Bevy-free: tests run headless.
use glam::{Vec2, Vec3};
use crate::{Local, config::{RainCfg, Shafts}};

pub fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 { let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) }

/// Rain intensity 0..1 at a place. `force` (panel toggle, `NEPHTHENE_RAIN`) replaces the map; otherwise precip, gated by cloud cover
/// (rain needs cloud overhead).
pub fn intensity(c: &RainCfg, here: &Local, force: Option<f32>) -> f32 {
    match force {
        Some(f) => f.clamp(0.0, 1.0),
        // Below min_intensity the map's rain is dry, so a faint precip under a mostly clear sky draws no streaks.
        None => { let i = smoothstep(c.precip_lo, c.precip_hi, here.precip) * smoothstep(c.cover_lo, c.cover_hi, here.coverage); if i < c.min_intensity { 0.0 } else { i } }
    }
}

/// Terminal fall speed, m/s (real drops 5..9).
pub fn fall_speed(c: &RainCfg, intensity: f32) -> f32 { c.fall_min_mps + c.fall_gain_mps * intensity.clamp(0.0, 1.0) }

/// Drop velocity: falls at `fall_speed`, drifts with the wind (world XZ, m/s).
pub fn velocity(c: &RainCfg, wind: Vec3, intensity: f32) -> Vec3 {
    let d = Vec3::new(wind.x, 0.0, wind.z) * c.wind_drift;
    let d = if d.length() > c.wind_drift_max { d.normalize() * c.wind_drift_max } else { d };
    Vec3::new(d.x, -fall_speed(c, intensity), d.z)
}

/// Streak length (m): distance covered during the shutter time.
pub fn streak_len(c: &RainCfg, vel: Vec3) -> f32 { vel.length() * c.shutter_s }

/// Streak slant from vertical, degrees.
pub fn slant_deg(vel: Vec3) -> f32 { (vel.x.hypot(vel.z)).atan2(-vel.y).to_degrees() }

/// Wetness 0..1 after `dt` seconds: gains while it rains, dries slowly otherwise.
pub fn wetness_step(c: &RainCfg, w: f32, intensity: f32, dt: f32) -> f32 {
    let d = if intensity > c.wet_threshold { c.wet_up_per_s * intensity } else { -c.dry_per_s };
    (w + d * dt).clamp(0.0, 1.0)
}

/// Ground look at `wet`: (albedo multiplier, perceptual roughness, reflectance) from the dry roughness/reflectance. Subtle.
pub fn wet_look(c: &RainCfg, wet: f32, dry_roughness: f32, dry_reflectance: f32) -> (f32, f32, f32) {
    let w = wet.clamp(0.0, 1.0);
    (1.0 - c.wet_darken * w, dry_roughness + (c.wet_roughness - dry_roughness) * w, dry_reflectance + (c.wet_reflectance - dry_reflectance) * w)
}

/// Snow cover 0..1 after `dt` seconds: builds at `cover_up_per_s` x the snowfall (intensity x kind), melts at `melt_per_s` while the
/// air at the ground is above `melt_c`.
pub fn snow_cover_step(c: &RainCfg, cover: f32, intensity: f32, kind: f32, temp_c: f32, dt: f32) -> f32 {
    let up = c.cover_up_per_s * intensity.clamp(0.0, 1.0) * kind.clamp(0.0, 1.0);
    let down = if temp_c > c.melt_c { c.melt_per_s } else { 0.0 };
    (cover + (up - down) * dt).clamp(0.0, 1.0)
}

/// The cover a surface shows: `cover_force` when set, otherwise the live cover or the climate map's ground snow, whichever is more.
pub fn snow_cover(c: &RainCfg, live: f32, climate_snow: f32) -> f32 { c.cover_force.unwrap_or(live.max(climate_snow)).clamp(0.0, 1.0) }

/// Snow on a surface whose normal points `normal_y` up: the cover, where the ground is flat enough to hold it. surface.wesl mirrors it.
pub fn snow_mask(c: &RainCfg, cover: f32, normal_y: f32) -> f32 { cover.clamp(0.0, 1.0) * smoothstep(c.cover_slope_lo, c.cover_slope_hi, normal_y) }

/// The ground at one place: (wetness, snow cover) 0..1. Wetness is the rain's or the climate map's ground wetness, whichever is more
/// (`climate::wetness`); snow is [`snow_cover`] of the live cover and the climate map's ground snow. Under a roof neither reaches it: (0, 0).
pub fn ground_at(c: &RainCfg, wetness: f32, live_cover: f32, place: &crate::climate::Place, sheltered: bool) -> (f32, f32) {
    if sheltered { return (0.0, 0.0); }
    (crate::climate::wetness(place, wetness).clamp(0.0, 1.0), snow_cover(c, live_cover, place.snow))
}

// ---- hash: PCG (O'Neill), u32 wrapping arithmetic, identical in rain.wesl (`pcg`, `hash01`). ----
pub const PCG_MUL: u32 = 747796405;
pub const PCG_ADD: u32 = 2891336453;
pub const PCG_OUT: u32 = 277803737;

pub fn pcg(v: u32) -> u32 {
    let s = v.wrapping_mul(PCG_MUL).wrapping_add(PCG_ADD);
    let w = ((s >> ((s >> 28) + 4)) ^ s).wrapping_mul(PCG_OUT);
    (w >> 22) ^ w
}
/// Uniform in [0, 1): top 24 bits.
pub fn hash01(v: u32) -> f32 { (pcg(v) >> 8) as f32 / 16777216.0 }
fn rem(x: f32, s: f32) -> f32 { x - s * (x / s).floor() }

/// Whether quad `q` of `count` falls at this intensity: the falling quads are the prefix `q < intensity x count`, so the pass can draw
/// just that range (`draw_quads`). Each quad's place is hashed, so any prefix is spread evenly through the volume.
pub fn active(q: u32, count: u32, intensity: f32) -> bool { (q as f32) < intensity.clamp(0.0, 1.0) * count as f32 }

/// Quads to draw for `count` at `intensity`: the smallest `count >> k` (k = 0..=levels, never 0) that still holds every falling quad.
pub fn draw_quads(count: u32, intensity: f32, levels: u32) -> u32 {
    let need = (intensity.clamp(0.0, 1.0) * count as f32).ceil() as u32;
    let mut n = count;
    for _ in 0..levels { if n / 2 >= need.max(1) { n /= 2 } else { break } }
    n
}

/// World position of the head of streak `q` at time `t` (s): a fixed point per drop, falling with `vel`, wrapped into the
/// `half_m` x `height_m` window around `cam` (x, z) and `ground_y` (y).
pub fn streak_pos(c: &RainCfg, q: u32, t: f32, vel: Vec3, cam: Vec3, ground_y: f32) -> Vec3 {
    let (s, half, height) = (2.0 * c.half_m, c.half_m, c.height_m);
    let b = |k: u32| hash01(q.wrapping_mul(4).wrapping_add(k));
    Vec3::new(
        cam.x - half + rem(b(0) * s + vel.x * t - cam.x, s),
        ground_y + rem(b(1) * height + vel.y * t, height),
        cam.z - half + rem(b(2) * s + vel.z * t - cam.z, s),
    )
}

/// Splash `s` at time `t`: (ground position, age 0..1, active). Each splash re-rolls its place every `splash_life_s`. Active: the prefix
/// `sid < intensity x max_splashes`.
pub fn splash(c: &RainCfg, sid: u32, t: f32, cam: Vec3, ground_y: f32, intensity: f32) -> (Vec3, f32, bool) {
    let half = c.half_m;
    let tt = t / c.splash_life_s + hash01(sid.wrapping_mul(4));
    let (k, age) = (tt.floor(), tt - tt.floor());
    let k_c = sid.wrapping_mul(7919).wrapping_add(k as u32);
    let sz = 2.0 * half;
    let h = |f: u32| hash01(k_c.wrapping_mul(4).wrapping_add(f));
    let p = Vec3::new(cam.x - half + rem(h(1) * sz - cam.x, sz), ground_y, cam.z - half + rem(h(2) * sz - cam.z, sz));
    (p, age, active(sid, c.max_splashes, intensity))
}


// ---- Snow and sleet: the same streak quads; the share `kind` of them are flakes (sleet is a mix of drops and flakes). rain.wesl mirrors this. ----

/// Salt of the flake hash, so a quad's kind does not correlate with its drop fields (q * 4 + 0..3). rain.wesl has the same constant.
pub const FLAKE_SALT: u32 = 1759714724;

/// Precipitation kind 0 rain .. 1 snow at the camera: `kind_force` when set, otherwise the air state's `precip_kind`.
pub fn kind(c: &RainCfg, air_kind: f32) -> f32 { c.kind_force.unwrap_or(air_kind).clamp(0.0, 1.0) }

/// The liquid share of the precipitation: what splashes and wets the ground.
pub fn liquid(intensity: f32, kind: f32) -> f32 { intensity * (1.0 - kind.clamp(0.0, 1.0)) }

fn flake_field(q: u32, k: u32) -> f32 { hash01((q ^ FLAKE_SALT).wrapping_mul(4).wrapping_add(k)) }

/// Whether streak quad `q` is a flake at this kind (field 0 of its flake hash).
pub fn is_flake(q: u32, kind: f32) -> bool { flake_field(q, 0) < kind }

/// Flake velocity: falls at `snow_fall_mps`, drifts with the wind (world XZ, m/s) at `snow_drift`, capped at `snow_drift_max`.
pub fn snow_velocity(c: &RainCfg, wind: Vec3) -> Vec3 {
    let d = Vec3::new(wind.x, 0.0, wind.z) * c.snow_drift;
    let d = if d.length() > c.snow_drift_max { d.normalize() * c.snow_drift_max } else { d };
    Vec3::new(d.x, -c.snow_fall_mps, d.z)
}

/// Flake `q` at time `t`: the drop's fixed point (`streak_pos`'s hash fields) falling with the flake velocity `vel`, wrapped into the
/// `snow_half_m` x `height_m` window, plus a horizontal swirl of radius `flutter_m` at `flutter_hz` turns per second (phase: field 1 of the flake hash).
pub fn flake_pos(c: &RainCfg, q: u32, t: f32, vel: Vec3, cam: Vec3, ground_y: f32) -> Vec3 {
    let (s, half) = (2.0 * c.snow_half_m, c.snow_half_m);
    let b = |k: u32| hash01(q.wrapping_mul(4).wrapping_add(k));
    let a = std::f32::consts::TAU * (c.flutter_hz * t + flake_field(q, 1));
    Vec3::new(
        cam.x - half + rem(b(0) * s + vel.x * t - cam.x, s) + c.flutter_m * a.sin(),
        ground_y + rem(b(1) * c.height_m + vel.y * t, c.height_m),
        cam.z - half + rem(b(2) * s + vel.z * t - cam.z, s) + c.flutter_m * a.cos(),
    )
}

// ---- Ground height source (near rain sits on the slopes) ----

/// Terrain height under (x, z), world metres. Flat ground is `Flat(h)`; a terrain supplies a `HeightGrid`.
pub trait Ground { fn height(&self, x: f32, z: f32) -> f32; }
pub struct Flat(pub f32);
impl Ground for Flat { fn height(&self, _: f32, _: f32) -> f32 { self.0 } }

/// Heights on a regular grid, rows north to south, cell centres at `(i + 0.5) * cell_m` from `min_x` / down from `max_z`.
/// Bilinear, clamped at the edges. rain.wesl / clouds_shaft.wesl (`ground_h`) mirror `height`.
#[derive(Clone, Debug)]
pub struct HeightGrid { pub min_x: f32, pub max_z: f32, pub cell_m: f32, pub w: u32, pub h: u32, pub y: Vec<f32> }
impl HeightGrid {
    /// One cell, huge: the same height everywhere.
    pub fn flat(y: f32) -> Self { Self { min_x: -5.0e5, max_z: 5.0e5, cell_m: 1.0e6, w: 1, h: 1, y: vec![y] } }
}
impl Ground for HeightGrid {
    fn height(&self, x: f32, z: f32) -> f32 {
        let fx = ((x - self.min_x) / self.cell_m - 0.5).clamp(0.0, (self.w - 1) as f32);
        let fy = ((self.max_z - z) / self.cell_m - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (fx as u32, fy as u32);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let p = |x: u32, y: u32| self.y[(y * self.w + x) as usize];
        let (a, b) = (p(x0, y0) + (p(x1, y0) - p(x0, y0)) * tx, p(x0, y1) + (p(x1, y1) - p(x0, y1)) * tx);
        a + (b - a) * ty
    }
}

// ---- Shelter: roofs the precipitation stops on (occlusion). rain.wesl mirrors `HeightGrid::top` (`shelter_h`) and `floor_at` (`floor_h`). ----

/// Height of "no roof here": below any ground.
pub const NO_ROOF: f32 = -1.0e9;

impl HeightGrid {
    /// The nearest cell's height, or `NO_ROOF` outside the grid (no clamping: a roof ends at its edge).
    pub fn top(&self, x: f32, z: f32) -> f32 {
        let (fx, fy) = ((x - self.min_x) / self.cell_m, (self.max_z - z) / self.cell_m);
        if fx < 0.0 || fy < 0.0 || fx >= self.w as f32 || fy >= self.h as f32 { return NO_ROOF; }
        self.y[(fy as u32 * self.w + fx as u32) as usize]
    }
}

/// Roof heights from world boxes (min, max corners): each cell centre takes the highest box top over it, `NO_ROOF` where none is.
/// The cell grows from `cell_m` so the grid stays near `max_cells`. No boxes: one `NO_ROOF` cell.
pub fn shelter_grid(boxes: &[(Vec3, Vec3)], cell_m: f32, max_cells: u32) -> HeightGrid {
    if boxes.is_empty() { return HeightGrid { min_x: 0.0, max_z: 0.0, cell_m: 1.0, w: 1, h: 1, y: vec![NO_ROOF] }; }
    let (lo, hi) = boxes.iter().fold((Vec3::MAX, Vec3::MIN), |(l, h), (a, b)| (l.min(*a), h.max(*b)));
    // ponytail: 1 mm floor keeps a zero-width box from dividing by zero; not a look value.
    let (sx, sz) = ((hi.x - lo.x).max(1e-3), (hi.z - lo.z).max(1e-3));
    let cell = cell_m.max((sx * sz / max_cells.max(1) as f32).sqrt());
    let (w, h) = (((sx / cell).ceil() as u32).max(1), ((sz / cell).ceil() as u32).max(1));
    let y = (0..h).flat_map(|j| (0..w).map(move |i| (i, j))).map(|(i, j)| {
        let (x, z) = (lo.x + (i as f32 + 0.5) * cell, hi.z - (j as f32 + 0.5) * cell);
        boxes.iter().filter(|(a, b)| x >= a.x && x <= b.x && z >= a.z && z <= b.z).map(|(_, b)| b.y).fold(NO_ROOF, f32::max)
    }).collect();
    HeightGrid { min_x: lo.x, max_z: hi.z, cell_m: cell, w, h, y }
}

/// Where precipitation lands at (x, z): the ground, or a roof above it.
pub fn floor_at(g: &dyn Ground, roofs: &HeightGrid, x: f32, z: f32) -> f32 { g.height(x, z).max(roofs.top(x, z)) }

/// The camera stands under a roof.
pub fn sheltered(roofs: &HeightGrid, cam: Vec3) -> bool { roofs.top(cam.x, cam.z) > cam.y }

/// `streak_pos` over terrain: the drop's xz wraps in the window as before, its y is `height_m` above the ground under it.
pub fn streak_pos_on(c: &RainCfg, q: u32, t: f32, vel: Vec3, cam: Vec3, g: &dyn Ground) -> Vec3 {
    let p = streak_pos(c, q, t, vel, cam, 0.0);
    Vec3::new(p.x, p.y + g.height(p.x, p.z), p.z)
}

// ---- Distant rain shafts (virga): a rain medium from the cloud base down to the ground, marched in clouds_shaft.wesl ----
// The shader mirrors `column_origin`, `shaft_density`, `shaft_slant` with the values of [`Shafts`] (uniform).

/// Shaft lean (x, z) from the MEAN wind (config speed, no gust): a small shear fraction of the drop lean, so gusts never swing the column.
/// The cloud drifts with the wind, so relative to it rain leans only by the shear between cloud base and ground.
pub fn shaft_slant(r: &RainCfg, sh: &Shafts, mean_wind: &crate::Wind) -> Vec2 {
    let m = crate::wind(&crate::Wind { gust: 0.0, ..mean_wind.clone() }, 0.0);
    let v = velocity(r, m, sh.lean_intensity);
    sh.shear_frac * Vec2::new(v.x, v.z) / -v.y
}

/// The full drop lean on the live (gusting) wind: the A/B reference for the gust-immune lean.
pub fn shaft_slant_live(r: &RainCfg, sh: &Shafts, wind: Vec3) -> Vec2 { let v = velocity(r, wind, sh.lean_intensity); Vec2::new(v.x, v.z) / -v.y }

/// Frame-rate independent slew of the lean toward `target`.
pub fn slew_slant(sh: &Shafts, cur: Vec2, target: Vec2, dt: f32) -> Vec2 { cur + (target - cur) * (1.0 - (-dt / sh.slant_tau_s).exp()) }

/// One frame of the shaft lean: slews `cur` toward the mean-wind lean (`shaft_slant`), or toward the live drop lean with `gust_lean` (red arm). `cur` zero = first frame: start at the target.
pub fn shaft_slant_step(r: &RainCfg, sh: &Shafts, mean_wind: &crate::Wind, live: Vec3, cur: Vec2, dt: f32) -> Vec2 { let tgt = if sh.gust_lean { shaft_slant_live(r, sh, live) } else { shaft_slant(r, sh, mean_wind) }; if cur == Vec2::ZERO { tgt } else { slew_slant(sh, cur, tgt, dt) } }

/// Where the rain at (xz, y) left the cloud base `base_y`: upwind of it by the lean.
pub fn column_origin(xz: Vec2, y: f32, base_y: f32, slant: Vec2) -> Vec2 { xz - slant * (base_y - y) }

/// Shaft density 0..1 at height fraction `hf` (0 ground, 1 cloud base): `precip` is the map's rain at the point itself, `cloud` the presence of
/// the cloud at the column's origin (`column_origin`, upwind at the base) and uniform-0..1 `streak` noise.
pub fn shaft_density(r: &RainCfg, sh: &Shafts, precip: f32, cloud: f32, streak: f32, hf: f32) -> f32 {
    let gate = smoothstep(r.precip_lo, r.precip_hi, precip) * smoothstep(sh.cloud_lo, sh.cloud_hi, cloud);
    let streaks = sh.gap + (1.0 - sh.gap) * smoothstep(sh.streak_lo, sh.streak_hi, streak);
    gate * streaks * (sh.ground_keep + (1.0 - sh.ground_keep) * hf.clamp(0.0, 1.0))
}

/// Ray distance at which a ray from height `y` along vertical component `dy` reaches the cloud base (or `max` when it never does).
pub fn shaft_t_end(y: f32, dy: f32, base_y: f32, max: f32) -> f32 {
    if dy > 1e-4 { ((base_y - y) / dy).clamp(0.0, max) } else { max }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rc() -> RainCfg { RainCfg::default() }
    fn sh() -> Shafts { Shafts::default() }
    fn here(coverage: f32, precip: f32) -> Local { Local { coverage, precip, dust: 0.0 } }

    #[test]
    fn faint_map_rain_is_dry() {
        // A faint map rain (0.12) under partly cloudy sky must be intensity exactly 0.
        assert_eq!(intensity(&rc(), &here(0.9, 0.12), None), 0.0);
        assert_eq!(intensity(&rc(), &here(0.5, 0.15), None), 0.0);
        assert_eq!(intensity(&rc(), &here(0.9, 0.12), Some(0.01)), 0.01, "forced values are not clipped");
    }

    #[test]
    fn intensity_follows_precip_coverage_and_force() {
        assert_eq!(intensity(&rc(), &here(0.9, 0.0), None), 0.0);
        assert_eq!(intensity(&rc(), &here(0.0, 1.0), None), 0.0, "no cloud, no rain");
        assert_eq!(intensity(&rc(), &here(0.9, 1.0), None), 1.0);
        let (a, b) = (intensity(&rc(), &here(0.9, 0.3), None), intensity(&rc(), &here(0.9, 0.6), None));
        assert!(0.0 < a && a < b && b < 1.0);
        assert_eq!(intensity(&rc(), &here(0.0, 0.0), Some(1.0)), 1.0, "override forces rain");
        assert_eq!(intensity(&rc(), &here(0.9, 1.0), Some(0.0)), 0.0, "override can force dry");
        assert_eq!(intensity(&rc(), &here(0.0, 0.0), Some(7.0)), 1.0);
    }
    #[test]
    fn wind_slants_the_fall() {
        let still = velocity(&rc(), Vec3::ZERO, 1.0);
        assert_eq!((still.x, still.z), (0.0, 0.0));
        assert!(slant_deg(still).abs() < 1e-4);
        let v = velocity(&rc(), Vec3::new(6.0, 0.0, 0.0), 1.0);
        assert!(v.x > 0.0 && v.y < 0.0 && slant_deg(v) > 20.0 && slant_deg(v) < 70.0);
        assert!(velocity(&rc(), Vec3::new(500.0, 0.0, 0.0), 1.0).x <= rc().wind_drift_max + 1e-4);
        assert!(fall_speed(&rc(), 1.0) > fall_speed(&rc(), 0.0));
    }
    #[test]
    fn streak_length_is_speed_times_shutter() {
        let v = velocity(&rc(), Vec3::ZERO, 1.0);
        assert!((streak_len(&rc(), v) - 9.0 * rc().shutter_s).abs() < 1e-5);
        assert!(streak_len(&rc(), velocity(&rc(), Vec3::new(8.0, 0.0, 0.0), 1.0)) > streak_len(&rc(), v));
    }
    #[test]
    fn wetness_accumulates_and_dries_slower() {
        let mut w = 0.0;
        for _ in 0..(40 * 10) { w = wetness_step(&rc(), w, 1.0, 0.1); }
        assert!((w - 1.0).abs() < 1e-3, "40 s of full rain saturates: {w}");
        let mut d = w;
        for _ in 0..(40 * 10) { d = wetness_step(&rc(), d, 0.0, 0.1); }
        assert!(d > 0.7 && d < 1.0, "40 s dry leaves most of it: {d}");
        assert_eq!(wetness_step(&rc(), 0.5, 0.02, 1.0), 0.5 - rc().dry_per_s);
        let (m, r, f) = wet_look(&rc(), 1.0, 0.95, 0.1);
        assert!(m < 1.0 && r < 0.95 && f > 0.1);
        assert_eq!(wet_look(&rc(), 0.0, 0.95, 0.1), (1.0, 0.95, 0.1));
    }
    #[test]
    fn hash_is_pcg_reference_and_uniform() {
        // Values computed independently in Python from the same constants; rain.wesl must produce the same.
        assert_eq!((pcg(0), pcg(1), pcg(12345)), (129708002, 2831084092, 4099845390));
        let n = 20000;
        let mean = (0..n).map(hash01).sum::<f32>() / n as f32;
        assert!((mean - 0.5).abs() < 0.01 && (0..n).all(|i| (0.0..1.0).contains(&hash01(i))));
    }
    #[test]
    fn streaks_wrap_inside_the_window_and_fall() {
        let cam = Vec3::new(123.0, 1.7, -45.0);
        let v = velocity(&rc(), Vec3::new(4.0, 0.0, 2.0), 1.0);
        for q in (0..rc().max_streaks).step_by(97) {
            for t in [0.0, 1.3, 77.7, 3000.0] {
                let p = streak_pos(&rc(), q, t, v, cam, 0.0);
                assert!((p.x - cam.x).abs() <= rc().half_m + 1e-3 && (p.z - cam.z).abs() <= rc().half_m + 1e-3, "{p:?}");
                assert!(p.y >= 0.0 && p.y <= rc().height_m + 1e-3);
            }
            // World-fixed: moving the camera within the window does not move the drop.
            let (a, b) = (streak_pos(&rc(), q, 2.0, v, cam, 0.0), streak_pos(&rc(), q, 2.0, v, cam + Vec3::new(3.0, 0.0, -2.0), 0.0));
            let d = (a - b).abs();
            assert!(d.y < 1e-3 && (d.x < 1e-2 || (d.x - 2.0 * rc().half_m).abs() < 1e-2) && (d.z < 1e-2 || (d.z - 2.0 * rc().half_m).abs() < 1e-2));
        }
        let p0 = streak_pos(&rc(), 5, 0.0, v, cam, 0.0);
        let p1 = streak_pos(&rc(), 5, 0.01, v, cam, 0.0);
        assert!(p1.y < p0.y || p0.y - p1.y > rc().height_m - 1.0, "falls");
    }
    #[test]
    fn active_fraction_tracks_intensity() {
        let n = rc().max_streaks;
        let frac = |i: f32| (0..n).filter(|&q| active(q, n, i)).count() as f32 / n as f32;
        assert_eq!(frac(0.0), 0.0);
        assert!((frac(0.5) - 0.5).abs() < 0.03 && (frac(1.0) - 1.0).abs() < 1e-6);
        assert!((0..n).filter(|&q| active(q, n, 0.3)).all(|q| q < (0.3 * n as f32).ceil() as u32), "falling quads are a prefix");
    }
    #[test]
    fn prefix_mesh_holds_every_falling_quad() {
        for count in [24000u32, 6000, 1000, 7] {
            for i in [0.0f32, 0.001, 0.05, 0.3, 0.5, 0.51, 1.0] {
                for levels in [0u32, 1, 6] {
                    let n = draw_quads(count, i, levels);
                    let need = (i * count as f32).ceil() as u32;
                    assert!(n <= count && (0..=levels).any(|k| count >> k == n), "{count} {i} {levels}: {n}");
                    assert!((0..count).filter(|&q| active(q, count, i)).all(|q| q < n), "{count} {i} {levels}: {n}");
                    // Within 2x only where the mesh can shrink to a 2x fit: none at zero intensity (1 quad is the floor).
                    if levels == 6 && need >= 1 && need >= count >> 6 { assert!(n <= 2 * need, "{count} {i}: {n} for {need}"); }
                }
            }
        }
        assert_eq!(draw_quads(24000, 0.05, 6), 1500);
        assert_eq!(draw_quads(24000, 1.0, 6), 24000);
        assert_eq!(draw_quads(24000, 0.05, 0), 24000);
        assert_eq!(draw_quads(24000, 0.0, 6), 375);
    }
    #[test]
    fn splashes_land_on_the_ground_in_the_window() {
        let cam = Vec3::new(-10.0, 1.7, 9.0);
        for s in 0..200 {
            let (p, age, on) = splash(&rc(), s, 12.34, cam, 0.0, 1.0);
            assert!(on && p.y == 0.0 && (0.0..1.0).contains(&age) && (p.x - cam.x).abs() <= rc().half_m + 1e-3 && (p.z - cam.z).abs() <= rc().half_m + 1e-3);
        }
        assert!(!splash(&rc(), 3, 1.0, cam, 0.0, 0.0).2);
    }

    #[test]
    fn height_grid_is_bilinear_and_clamped() {
        // 2 x 2 cells of 10 m, west edge -10, north edge 10: heights NW 0, NE 10, SW 20, SE 30 at the cell centres.
        let g = HeightGrid { min_x: -10.0, max_z: 10.0, cell_m: 10.0, w: 2, h: 2, y: vec![0.0, 10.0, 20.0, 30.0] };
        assert_eq!(g.height(-5.0, 5.0), 0.0);
        assert_eq!(g.height(5.0, -5.0), 30.0);
        assert!((g.height(0.0, 0.0) - 15.0).abs() < 1e-4, "centre is the mean");
        assert_eq!(g.height(-500.0, 500.0), 0.0, "clamped");
        assert_eq!(Flat(3.0).height(1.0, 2.0), 3.0);
        assert_eq!(HeightGrid::flat(7.5).height(123456.0, -9876.0), 7.5);
    }
    #[test]
    fn streaks_stand_on_the_slope() {
        let g = HeightGrid { min_x: -10.0, max_z: 10.0, cell_m: 10.0, w: 2, h: 2, y: vec![0.0, 10.0, 20.0, 30.0] };
        let (cam, v) = (Vec3::ZERO, velocity(&rc(), Vec3::ZERO, 1.0));
        for q in (0..rc().max_streaks).step_by(211) {
            let p = streak_pos_on(&rc(), q, 3.0, v, cam, &g);
            let base = g.height(p.x, p.z);
            assert!(p.y >= base - 1e-3 && p.y <= base + rc().height_m + 1e-3, "{p:?} base {base}");
        }
        let flat = streak_pos_on(&rc(), 9, 1.0, v, cam, &Flat(0.0));
        assert_eq!(flat, streak_pos(&rc(), 9, 1.0, v, cam, 0.0));
    }
    #[test]
    fn shaft_needs_rain_and_cloud_and_thins_toward_the_ground() {
        assert_eq!(shaft_density(&rc(), &sh(), 0.0, 1.0, 1.0, 1.0), 0.0, "dry map, no shaft");
        assert_eq!(shaft_density(&rc(), &sh(), 1.0, 0.0, 1.0, 1.0), 0.0, "no cloud overhead, no shaft");
        let full = shaft_density(&rc(), &sh(), 1.0, 1.0, 1.0, 1.0);
        assert!((full - 1.0).abs() < 1e-5);
        let ground = shaft_density(&rc(), &sh(), 1.0, 1.0, 1.0, 0.0);
        assert!((ground - sh().ground_keep).abs() < 1e-5 && ground < full, "fades toward the ground: {ground}");
        assert!(shaft_density(&rc(), &sh(), 1.0, 1.0, 0.0, 1.0) < 0.2 * full, "noise gaps are mostly clear");
        assert!(shaft_density(&rc(), &sh(), 0.4, 1.0, 1.0, 1.0) < shaft_density(&rc(), &sh(), 0.8, 1.0, 1.0, 1.0), "more precip, denser");
    }
    #[test]
    fn shaft_leans_downwind_and_ends_at_the_cloud_base() {
        let w = |sp: f32, g: f32| crate::Wind { from_deg: 270.0, speed_mps: sp, gust: g, ..crate::Wind::default() };
        assert_eq!(shaft_slant(&rc(), &sh(), &w(0.0, 0.0)), Vec2::ZERO);
        let s = shaft_slant(&rc(), &sh(), &w(8.0, 0.0));
        assert!(s.x > 0.02 && s.y.abs() < 1e-5, "{s:?}");
        assert_eq!(shaft_slant(&rc(), &sh(), &w(10.0, 0.0)), shaft_slant(&rc(), &sh(), &w(10.0, 0.9)), "gust must not move the shafts");
        assert!(shaft_slant(&rc(), &sh(), &w(10.0, 0.5)).length().atan().to_degrees() < 15.0, "bounded lean at 10 m/s");
        let (mut c, t) = (Vec2::ZERO, s);
        for _ in 0..60 { c = slew_slant(&sh(), c, t, 1.0); }
        assert!((c - t).length() < 0.06 * t.length(), "slew reaches the target");
        assert!(slew_slant(&sh(), Vec2::ZERO, t, 0.1).length() < 0.01 * t.length(), "slow");
        let base = Vec2::new(100.0, 50.0);
        assert_eq!(column_origin(base, 1000.0, 1000.0, s), base, "at the base the column starts at its own xz");
        let low = column_origin(base, 0.0, 1000.0, s);
        assert!(low.x < base.x, "rain at the ground fell from upwind: {low:?}");
        assert!((shaft_t_end(0.0, 0.5, 700.0, 1.0e4) - 1400.0).abs() < 1e-2);
        assert_eq!(shaft_t_end(0.0, -0.2, 700.0, 1.0e4), 1.0e4);
        assert_eq!(shaft_t_end(0.0, 0.001, 700.0, 5000.0), 5000.0);
    }
    // Budget (m) for the foot of a shaft swinging over a gust cycle: one pixel at 5 km for a 1920 px wide 75 deg view is
    // 2 * 5000 * tan(37.5 deg) / 1920 = 4.0 m, so a foot that moves less than that does not visibly swing.
    const SWING_BUDGET_M: f32 = 4.0;
    #[test]
    fn shafts_do_not_lean_with_gusts() {
        let (r, wind) = (rc(), crate::Wind::default());
        let swing = |cfg: &Shafts| {
            let (dt, mut cur, mut lo, mut hi) = (1.0 / 60.0, Vec2::ZERO, Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
            for i in 0..(70 * 60) {
                let t = i as f32 * dt;
                cur = shaft_slant_step(&r, cfg, &wind, crate::wind(&wind, t), cur, dt);
                if t >= 10.0 { lo = lo.min(cur); hi = hi.max(cur); }
            }
            (hi - lo).max_element() * crate::config::Config::default().clouds.bottom_m
        };
        let (default, red) = (swing(&sh()), swing(&Shafts { gust_lean: true, ..sh() }));
        println!("shaft foot swing over a gust cycle: default {default:.3} m, red arm {red:.3} m (budget {SWING_BUDGET_M} m)");
        assert!(default <= SWING_BUDGET_M, "default shafts swing {default} m, budget {SWING_BUDGET_M} m");
        assert!(red > SWING_BUDGET_M, "red arm must swing past the budget, got {red} m (budget {SWING_BUDGET_M} m)");
    }
    #[test]
    fn shelter_grid_takes_the_highest_roof() {
        let a = (Vec3::new(-10.0, 0.0, -10.0), Vec3::new(10.0, 4.0, 10.0));
        let b = (Vec3::new(0.0, 0.0, 0.0), Vec3::new(5.0, 7.0, 5.0));
        let g = shelter_grid(&[a, b], 0.5, 1 << 20);
        assert_eq!(g.top(-5.0, -5.0), 4.0);
        assert_eq!(g.top(2.5, 2.5), 7.0, "the higher box wins where they overlap");
        assert_eq!(g.top(20.0, 0.0), NO_ROOF);
        assert_eq!(g.top(0.0, -10.5), NO_ROOF);
        assert!(g.cell_m == 0.5 && g.w == 40 && g.h == 40);
    }
    #[test]
    fn shelter_grid_caps_its_cells_and_handles_none() {
        assert_eq!(shelter_grid(&[], 0.5, 100).top(0.0, 0.0), NO_ROOF);
        let big = (Vec3::new(0.0, 0.0, 0.0), Vec3::new(1000.0, 3.0, 1000.0));
        let g = shelter_grid(&[big], 0.5, 10_000);
        assert!(g.cell_m >= 10.0 - 1e-3 && g.w * g.h <= 10_100, "cell {} grid {}x{}", g.cell_m, g.w, g.h);
        assert_eq!(g.top(500.0, 500.0), 3.0);
    }
    #[test]
    fn floor_is_the_roof_over_the_ground() {
        let roofs = shelter_grid(&[(Vec3::new(-4.0, 0.0, -4.0), Vec3::new(4.0, 3.0, 4.0))], 0.5, 1 << 20);
        assert_eq!(floor_at(&Flat(1.0), &roofs, 0.0, 0.0), 3.0);
        assert_eq!(floor_at(&Flat(1.0), &roofs, 10.0, 0.0), 1.0);
        assert!(sheltered(&roofs, Vec3::new(0.0, 1.7, 0.0)));
        assert!(!sheltered(&roofs, Vec3::new(0.0, 5.0, 0.0)), "above the roof is open sky");
        assert!(!sheltered(&roofs, Vec3::new(10.0, 1.7, 0.0)));
    }
    #[test]
    fn kind_follows_air_unless_forced() {
        assert_eq!(kind(&rc(), 0.3), 0.3);
        assert_eq!(kind(&RainCfg { kind_force: Some(0.0), ..rc() }, 1.0), 0.0);
        assert_eq!(kind(&RainCfg { kind_force: Some(7.0), ..rc() }, 0.0), 1.0);
        assert_eq!(kind(&rc(), -2.0), 0.0);
    }
    #[test]
    fn liquid_share() {
        assert_eq!(liquid(1.0, 1.0), 0.0);
        assert_eq!(liquid(0.8, 0.0), 0.8);
        assert!((liquid(1.0, 0.5) - 0.5).abs() < 1e-6);
    }
    #[test]
    fn flake_share_tracks_kind() {
        let n = rc().max_streaks;
        let share = |k: f32| (0..n).filter(|&q| is_flake(q, k)).count() as f32 / n as f32;
        assert_eq!(share(0.0), 0.0);
        assert_eq!(share(1.0), 1.0);
        assert!((share(0.5) - 0.5).abs() < 0.03, "{}", share(0.5));
        let live: Vec<u32> = (0..n).filter(|&q| active(q, n, 0.5)).collect();
        let flakes = live.iter().filter(|&&q| is_flake(q, 0.5)).count() as f32 / live.len() as f32;
        assert!((flakes - 0.5).abs() < 0.05, "flakes among live streaks: {flakes}");
    }
    #[test]
    fn snow_falls_slowly_and_drifts() {
        let still = snow_velocity(&rc(), Vec3::ZERO);
        assert_eq!(still, Vec3::new(0.0, -rc().snow_fall_mps, 0.0));
        assert!(-still.y < fall_speed(&rc(), 0.0), "flakes fall slower than the lightest rain");
        let drift = snow_velocity(&rc(), Vec3::new(6.0, 0.0, 0.0));
        assert!(drift.x > 0.0 && slant_deg(drift) > slant_deg(velocity(&rc(), Vec3::new(6.0, 0.0, 0.0), 1.0)), "{drift:?}");
        assert!(snow_velocity(&rc(), Vec3::new(500.0, 0.0, 0.0)).x <= rc().snow_drift_max + 1e-4);
    }
    #[test]
    fn flakes_swirl_inside_the_window() {
        let cam = Vec3::new(123.0, 1.7, -45.0);
        let vel = snow_velocity(&rc(), Vec3::ZERO);
        for q in (0..rc().max_streaks).step_by(97) {
            for t in [0.0, 1.3, 77.7] {
                let p = flake_pos(&rc(), q, t, vel, cam, 0.0);
                let s = flake_pos(&RainCfg { flutter_m: 0.0, ..rc() }, q, t, vel, cam, 0.0);
                assert!(((p.x - s.x).hypot(p.z - s.z) - rc().flutter_m).abs() < 1e-3, "{p:?} {s:?}");
                assert!((p.x - cam.x).abs() <= rc().snow_half_m + rc().flutter_m + 1e-3 && (p.z - cam.z).abs() <= rc().snow_half_m + rc().flutter_m + 1e-3, "{p:?}");
                assert!(p.y >= 0.0 && p.y <= rc().height_m + 1e-3);
            }
        }
        let period = 1.0 / rc().flutter_hz;
        let xs: Vec<f32> = (0..8).map(|i| flake_pos(&rc(), 5, period * i as f32 / 8.0, vel, cam, 0.0).x).collect();
        let (lo, hi) = xs.iter().fold((f32::MAX, f32::MIN), |(l, h), &x| (l.min(x), h.max(x)));
        assert!(hi - lo > rc().flutter_m, "flake swirls: x spans {}", hi - lo);
    }
    #[test]
    fn snow_cover_builds_while_snowing_and_melts_when_warm() {
        let step = |c: f32, i: f32, k: f32, t: f32, n: usize| (0..n).fold(c, |c, _| snow_cover_step(&rc(), c, i, k, t, 0.1));
        // 120 s of full snow reach the top (to f32 rounding); 130 s clamp to it exactly.
        assert!(step(0.0, 1.0, 1.0, -5.0, 1200) > 1.0 - 1e-5 && step(0.0, 1.0, 1.0, -5.0, 1300) == 1.0, "120 s of snow saturates");
        assert_eq!(step(0.0, 1.0, 0.0, -5.0, 1200), 0.0, "rain does not cover");
        let melted = step(1.0, 0.0, 1.0, 5.0, 1200);
        assert!(0.8 < melted && melted < 0.9 && (melted - 0.8667).abs() < 1e-3, "{melted}");
        assert_eq!(step(1.0, 0.0, 1.0, 0.0, 1200), 1.0, "below melt_c the cover holds");
    }
    #[test]
    fn snow_cover_takes_force_then_the_more_of_live_and_climate() {
        assert_eq!(snow_cover(&rc(), 0.2, 0.6), 0.6);
        assert_eq!(snow_cover(&rc(), 0.7, 0.1), 0.7);
        assert_eq!(snow_cover(&RainCfg { cover_force: Some(0.0), ..rc() }, 1.0, 1.0), 0.0);
        assert_eq!(snow_cover(&RainCfg { cover_force: Some(3.0), ..rc() }, 0.0, 0.0), 1.0);
    }
    #[test]
    fn snow_holds_on_flat_ground_only() {
        assert_eq!(snow_mask(&rc(), 1.0, 1.0), 1.0);
        assert_eq!(snow_mask(&rc(), 1.0, 0.5), 0.0, "steeper than cover_slope_lo");
        assert_eq!(snow_mask(&rc(), 0.5, 1.0), 0.5);
        let m = snow_mask(&rc(), 1.0, 0.725);
        assert!(0.0 < m && m < 1.0, "{m}");
    }
    #[test]
    fn ground_at_is_dry_and_bare_under_a_roof() {
        let place = crate::climate::Place { snow: 1.0, wetness: 1.0, ..Default::default() };
        assert_eq!(ground_at(&rc(), 1.0, 1.0, &place, true), (0.0, 0.0));
    }
    #[test]
    fn ground_at_takes_the_more_of_rain_and_climate() {
        let place = crate::climate::Place { wetness: 0.5, snow: 0.6, ..Default::default() };
        assert_eq!(ground_at(&rc(), 0.2, 0.3, &place, false), (0.5, 0.6));
        assert_eq!(ground_at(&rc(), 0.7, 0.8, &crate::climate::Place::default(), false), (0.7, 0.8));
    }
    #[test]
    fn ground_at_snow_follows_cover_force() {
        let place = crate::climate::Place { snow: 1.0, ..Default::default() };
        assert_eq!(ground_at(&RainCfg { cover_force: Some(0.0), ..Default::default() }, 0.0, 1.0, &place, false).1, 0.0);
    }
}
