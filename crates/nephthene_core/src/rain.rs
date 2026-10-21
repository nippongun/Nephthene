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

/// Whether streak `q` is drawn at this intensity (field 3 of its hash).
pub fn streak_active(q: u32, intensity: f32) -> bool { hash01(q.wrapping_mul(4).wrapping_add(3)) < intensity }

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

/// Splash `s` at time `t`: (ground position, age 0..1, active). Each splash re-rolls its place every `splash_life_s`.
pub fn splash(c: &RainCfg, sid: u32, t: f32, cam: Vec3, ground_y: f32, intensity: f32) -> (Vec3, f32, bool) {
    let half = c.half_m;
    let tt = t / c.splash_life_s + hash01(sid.wrapping_mul(4));
    let (k, age) = (tt.floor(), tt - tt.floor());
    let k_c = sid.wrapping_mul(7919).wrapping_add(k as u32);
    let sz = 2.0 * half;
    let h = |f: u32| hash01(k_c.wrapping_mul(4).wrapping_add(f));
    let p = Vec3::new(cam.x - half + rem(h(1) * sz - cam.x, sz), ground_y, cam.z - half + rem(h(2) * sz - cam.z, sz));
    (p, age, h(3) < intensity)
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
        let frac = |i: f32| (0..rc().max_streaks).filter(|&q| streak_active(q, i)).count() as f32 / rc().max_streaks as f32;
        assert_eq!(frac(0.0), 0.0);
        assert!((frac(0.5) - 0.5).abs() < 0.03 && (frac(1.0) - 1.0).abs() < 1e-6);
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
}
