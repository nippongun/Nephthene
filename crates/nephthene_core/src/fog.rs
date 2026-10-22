//! Ground fog: radiation fog that pools in valleys under a level top and lies in hollows as shallow mist. Pure maths; the plugin
//! (src/fog.rs) keeps a [`Field`] and per-texel amounts around the camera, bakes them with [`map`] into a texture and fog.wesl marches
//! [`density`] through it.
//!
//! Formation ([`evolve`]): on a clear, calm night the air cools to its dew point (crate::air) and builds the amount 0..1; wind mixes
//! it out. Burn-off is local: a texel burns only while the sun reaches its fog top over the terrain ([`lit`] against the baked
//! sunlit altitude), so a valley in a mountain's shadow keeps its fog after the open ground has cleared, and its base lifts only
//! once the sun is on it. Shape ([`field`]): cold air drains off slopes into the lowest ground within `pool_radius_m`, so the
//! pool top is that floor plus a depth that grows with the amount and with how enclosed the floor is. Hilltops above it stay clear;
//! steep slopes drain their mist.
use glam::Vec3;

use crate::{Clock, air, config::{AirCfg, GroundFog}, rain::{Ground, HeightGrid, smoothstep}};

/// Formation inputs at the camera: cloud cover 0..1, wind speed m/s, ground wetness 0..1, dust 0..1, and the air's dew point spread
/// (K, temperature - dew point; [`spin_up`] and [`spin_up_local`] integrate their own from the other inputs).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Air { pub coverage: f32, pub wind_mps: f32, pub wetness: f32, pub dust: f32, pub spread_c: f32 }

/// Moisture 0..1: wet ground evaporates into the cooling air, dust marks dry air.
pub fn moisture(c: &GroundFog, a: &Air) -> f32 { (c.moisture_base + c.wet_gain * a.wetness - c.dust_dry * a.dust).clamp(0.0, 1.0) }

/// `a` with the dew point spread of surface air at `temp_c`.
fn with_spread(ac: &AirCfg, clock: &Clock, temp_c: f32, a: &Air) -> Air { Air { spread_c: temp_c - air::state(ac, clock, temp_c, a.wetness, a.dust).dew_c, ..*a } }

/// One step of `dt_h` game hours from `amount` at sun elevation `el` (deg); `lit` 0..1 is how much of the sun reaches the fog
/// (terrain shadow: 0 burns nothing). Formation follows the air's dew point spread `a.spread_c` unless `private_cooling`.
pub fn evolve(c: &GroundFog, amount: f32, dt_h: f32, el: f32, a: &Air, lit: f32) -> f32 {
    let windy = smoothstep(c.wind_lo_mps, c.wind_hi_mps, a.wind_mps);
    // The air's dew point spread; the red arm keeps the old private cooling (night by sun elevation, cover, moisture).
    let form = if c.private_cooling {
        let night = 1.0 - smoothstep(c.night_lo_deg, c.night_hi_deg, el);
        c.form_per_h * night * (1.0 - c.cover_block * a.coverage) * (1.0 - windy) * moisture(c, a)
    } else {
        c.form_per_h * (1.0 - smoothstep(c.spread_full_c, c.spread_none_c, a.spread_c)) * (1.0 - windy)
    };
    let burn = c.burn_per_h * smoothstep(0.0, c.burn_full_deg, el) * lit * (1.0 - c.cover_shade * a.coverage) + c.mix_per_h * windy;
    (amount + dt_h * (form * (1.0 - amount) - burn * amount)).clamp(0.0, 1.0)
}

/// Spin-up step (game hours): the night needs no terrain shadow, daylight steps recompute it.
const SPIN_STEP_H: f32 = 0.25;

/// The amount on open sunlit ground at `hour` after `spin_up_h` hours of this weather from none, the air integrated alongside; `force` replaces it.
pub fn spin_up(c: &GroundFog, ac: &AirCfg, clock: &Clock, hour: f32, a: &Air) -> f32 {
    if let Some(f) = c.force { return f.clamp(0.0, 1.0); }
    let n = (c.spin_up_h / SPIN_STEP_H).ceil() as u32;
    let mut t = air::spin_up(ac, clock, hour - c.spin_up_h, a.coverage);
    (0..n).fold(0.0, |m, i| {
        let h = hour - c.spin_up_h + SPIN_STEP_H * (i + 1) as f32;
        t = air::step(ac, clock, t, h - SPIN_STEP_H, SPIN_STEP_H, a.coverage);
        evolve(c, m, SPIN_STEP_H, crate::sun(clock, h).0, &with_spread(ac, clock, t, a), 1.0)
    })
}

/// Per-texel amounts at `hour` after `spin_up_h` hours of this weather, each texel burning only while the sun reaches its fog top.
/// `force` sets every texel.
pub fn spin_up_local<G: Ground + ?Sized>(c: &GroundFog, ac: &AirCfg, clock: &Clock, hour: f32, a: &Air, g: &G, f: &Field) -> Vec<f32> {
    if let Some(v) = c.force { return vec![v.clamp(0.0, 1.0); f.ground.len()]; }
    let mut m = vec![0.0; f.ground.len()];
    let mut t = air::spin_up(ac, clock, hour - c.spin_up_h, a.coverage);
    for i in 0..(c.spin_up_h / SPIN_STEP_H).ceil() as u32 {
        let h = hour - c.spin_up_h + SPIN_STEP_H * (i + 1) as f32;
        t = air::step(ac, clock, t, h - SPIN_STEP_H, SPIN_STEP_H, a.coverage);
        let (el, az) = crate::sun(clock, h);
        step(c, g, f, &mut m, SPIN_STEP_H, el, &with_spread(ac, clock, t, a), crate::dir(el, az));
    }
    m
}

/// Advance every texel's amount by `dt_h` game hours under the sun at elevation `el` (deg) and direction `sun`; returns the terrain
/// shadow ([`sun_alt`]) it used.
pub fn step<G: Ground + ?Sized>(c: &GroundFog, g: &G, f: &Field, m: &mut [f32], dt_h: f32, el: f32, a: &Air, sun: Vec3) -> Vec<f32> {
    let alt = if el > 0.0 { sun_alt(c, g, f, sun) } else { vec![NO_SUN; m.len()] };
    for (k, v) in m.iter_mut().enumerate() { *v = evolve(c, *v, dt_h, el, a, if c.ignore_shadow { 1.0 } else { lit(c, f.top(k, *v).max(f.ground[k]), alt[k]) }); }
    alt
}

/// How much of the sun reaches altitude `y` in a column that sees the sun above `sun_alt` (soft over `sun_soft_m`).
pub fn lit(c: &GroundFog, y: f32, sun_alt: f32) -> f32 { smoothstep(sun_alt - c.sun_soft_m, sun_alt + c.sun_soft_m, y) }

/// Height (m) the burning sun has lifted the fog base off the ground: the sun warms the ground first, so the base rises as the fog
/// thins (by `1 - amount`), only where the sun reaches it (`lit`); `sun_k` = smoothstep(0, lift_full_deg, elevation).
pub fn lift_m(c: &GroundFog, sun_k: f32, lit: f32, amount: f32) -> f32 { c.lift_max_m * sun_k * lit * (1.0 - amount.clamp(0.0, 1.0)) }

/// Extinction per metre in the pool and in the shallow mist at full fog: 3.912 / visibility. [`density`] thins it with the amount.
pub fn sigma(c: &GroundFog) -> (f32, f32) { (3.912 / c.visibility_m, 3.912 / c.shallow_visibility_m) }

/// Sun-independent terrain fields on the fog-map grid (texel layout as [`HeightGrid`], rows north to south): ground, pool floor and
/// pool depth at amount 1 (both averaged so the pool top is level), shallow mist strength at amount 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Field { pub min_x: f32, pub max_z: f32, pub cell_m: f32, pub n: u32, pub ground: Vec<f32>, pub floor: Vec<f32>, pub depth: Vec<f32>, pub shallow: Vec<f32> }

impl Field {
    /// World (x, z) of texel `k`'s centre.
    pub fn xz(&self, k: usize) -> (f32, f32) { (self.min_x + ((k as u32 % self.n) as f32 + 0.5) * self.cell_m, self.max_z - ((k as u32 / self.n) as f32 + 0.5) * self.cell_m) }
    /// Pool top of texel `k` at amount `a`.
    pub fn top(&self, k: usize, a: f32) -> f32 { self.floor[k] + a * self.depth[k] }
    /// Nearest texel to (x, z), None outside the grid.
    pub fn texel(&self, x: f32, z: f32) -> Option<usize> {
        let (i, j) = (((x - self.min_x) / self.cell_m).floor(), ((self.max_z - z) / self.cell_m).floor());
        (i >= 0.0 && j >= 0.0 && i < self.n as f32 && j < self.n as f32).then(|| (j as u32 * self.n + i as u32) as usize)
    }
}

/// The terrain fields around `centre` (snapped to whole texels so the map does not swim).
pub fn field<G: Ground + ?Sized>(c: &GroundFog, g: &G, centre: [f32; 2]) -> Field {
    let n = c.bake_px.max(2);
    let cell = c.extent_m / n as f32;
    let snap = |v: f32| (v / cell).round() * cell;
    let (min_x, max_z) = (snap(centre[0]) - c.extent_m / 2.0, snap(centre[1]) + c.extent_m / 2.0);
    let mut f = Field { min_x, max_z, cell_m: cell, n, ground: vec![], floor: vec![], depth: vec![], shallow: vec![] };
    f.ground = (0..(n * n) as usize).map(|k| { let (x, z) = f.xz(k); g.height(x, z) }).collect();
    // Drainage: cold air runs down to the lowest ground within the radius (a square min filter on the texels); the pool there is as deep
    // as that low point is enclosed (mean ground around it above it), so a ridge next to a slope does not pool on the slope's relief.
    // ponytail: a square, not a disc, and blind to drainage outlets; a priority-flood fill is the upgrade.
    let r_pool = (c.pool_radius_m / cell).round() as i32;
    let mean = box_blur(&f.ground, n, r_pool);
    let own: Vec<(f32, f32)> = (0..f.ground.len()).map(|k| (f.ground[k], c.depth_flat_m + (c.depth_max_m - c.depth_flat_m) * smoothstep(0.0, c.relief_m, mean[k] - f.ground[k]))).collect();
    // The pool is one body of air: floor and depth are averaged (a quarter and half the radius) so its top is level, not tilted to the walls.
    // The blanket red arm keeps each column's own ground and depth.
    (f.floor, f.depth) = if c.ignore_terrain { own.iter().cloned().unzip() } else {
        let low = min_by_ground(&own, n, r_pool);
        (box_blur(&low.iter().map(|p| p.0).collect::<Vec<_>>(), n, r_pool / 4), box_blur(&low.iter().map(|p| p.1).collect::<Vec<_>>(), n, r_pool / 2))
    };
    // Shallow mist: kept in hollows (ground below its surroundings), drained off slopes, a share of it on open flats.
    let around = box_blur(&f.ground, n, (c.hollow_radius_m / cell).round().max(1.0) as i32);
    f.shallow = (0..f.ground.len()).map(|k| {
        let ((x, z), d) = (f.xz(k), cell * 0.5);
        let slope = (g.height(x + d, z) - g.height(x - d, z)).hypot(g.height(x, z + d) - g.height(x, z - d)) / (2.0 * d);
        let hollow = smoothstep(0.0, c.hollow_m, around[k] - f.ground[k]);
        (1.0 - smoothstep(c.slope_lo, c.slope_hi, slope)) * (c.flat_share + (1.0 - c.flat_share) * hollow)
    }).collect();
    f
}

/// Sun altitude for a column when the sun is down: nothing is sunlit.
pub const NO_SUN: f32 = 1.0e6;

/// Terrain shadow: per texel, the altitude it must reach to see over the terrain toward the sun (NO_SUN with the sun down).
pub fn sun_alt<G: Ground + ?Sized>(c: &GroundFog, g: &G, f: &Field, sun: Vec3) -> Vec<f32> {
    if sun.y <= 0.0 { return vec![NO_SUN; f.ground.len()]; }
    let horiz = Vec3::new(sun.x, 0.0, sun.z).normalize_or_zero();
    let tan_el = sun.y / sun.x.hypot(sun.z).max(1e-6);
    let steps = (c.shadow_range_m / c.shadow_step_m.max(1.0)).ceil() as u32;
    (0..f.ground.len()).map(|k| {
        let (x, z) = f.xz(k);
        (1..=steps).map(|i| { let s = i as f32 * c.shadow_step_m; g.height(x + horiz.x * s, z + horiz.z * s) - s * tan_el }).fold(f32::MIN, f32::max)
    }).collect()
}

/// Amounts carried onto a new field (the camera moved): each new texel takes the old texel under it, or `fallback` outside the old grid.
pub fn resample(old: &Field, m: &[f32], new: &Field, fallback: f32) -> Vec<f32> {
    (0..new.ground.len()).map(|k| { let (x, z) = new.xz(k); old.texel(x, z).map_or(fallback, |o| m[o]) }).collect()
}

/// Fog map around the camera: per texel (pool top altitude, shallow mist strength 0..1, sunlit altitude, amount 0..1).
/// `y_lo`..`y_hi` bounds every altitude with fog.
#[derive(Clone, Debug, PartialEq)]
pub struct FogMap { pub min_x: f32, pub max_z: f32, pub cell_m: f32, pub n: u32, pub texels: Vec<[f32; 4]>, pub y_lo: f32, pub y_hi: f32 }

impl FogMap {
    /// Bilinear texel at (x, z), clamped at the edges (fog.wesl `map_at`).
    pub fn at(&self, x: f32, z: f32) -> [f32; 4] {
        let m = (self.n - 1) as f32;
        let fx = ((x - self.min_x) / self.cell_m - 0.5).clamp(0.0, m);
        let fz = ((self.max_z - z) / self.cell_m - 0.5).clamp(0.0, m);
        let (x0, z0) = (fx as u32, fz as u32);
        let (x1, z1) = ((x0 + 1).min(self.n - 1), (z0 + 1).min(self.n - 1));
        let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
        let t = |i: u32, j: u32| self.texels[(j * self.n + i) as usize];
        std::array::from_fn(|k| {
            let a = t(x0, z0)[k] + (t(x1, z0)[k] - t(x0, z0)[k]) * tx;
            let b = t(x0, z1)[k] + (t(x1, z1)[k] - t(x0, z1)[k]) * tx;
            a + (b - a) * tz
        })
    }
}

/// The fog map for these per-texel amounts and terrain shadow ([`sun_alt`]).
pub fn map(c: &GroundFog, f: &Field, m: &[f32], alt: &[f32]) -> FogMap {
    let n = f.n;
    let mut top: Vec<f32> = (0..m.len()).map(|k| f.top(k, m[k])).collect();
    let mut shallow: Vec<f32> = (0..m.len()).map(|k| m[k] * f.shallow[k]).collect();
    for _ in 0..c.blur_passes { top = blur(&top, n); shallow = blur(&shallow, n); }
    let y_lo = f.ground.iter().cloned().fold(f32::MAX, f32::min);
    let y_hi = (0..m.len()).map(|k| (top[k] + c.top_soft_m + c.top_wave_m).max(if shallow[k] > 0.0 { f.ground[k] + 6.0 * c.shallow_e_m } else { y_lo }))
        .fold(y_lo, f32::max);
    let texels = (0..m.len()).map(|k| [top[k], shallow[k], alt[k], m[k]]).collect();
    FogMap { min_x: f.min_x, max_z: f.max_z, cell_m: f.cell_m, n, texels, y_lo, y_hi }
}

/// Separable square filter of radius `r` texels folding with `op` (min, or + for a sum), edges clamped.
fn filter(v: &[f32], n: u32, r: i32, op: fn(f32, f32) -> f32) -> Vec<f32> {
    let n = n as i32;
    let pass = |v: &[f32], step: (i32, i32)| -> Vec<f32> {
        (0..n * n).map(|k| {
            let (i, j) = (k % n, k / n);
            (-r..=r).map(|d| v[((j + d * step.1).clamp(0, n - 1) * n + (i + d * step.0).clamp(0, n - 1)) as usize]).reduce(op).unwrap_or(0.0)
        }).collect()
    };
    pass(&pass(v, (1, 0)), (0, 1))
}

/// Separable square min filter of radius `r` texels on the first value, carrying the second, edges clamped.
fn min_by_ground(v: &[(f32, f32)], n: u32, r: i32) -> Vec<(f32, f32)> {
    let n = n as i32;
    let pass = |v: &[(f32, f32)], step: (i32, i32)| -> Vec<(f32, f32)> {
        (0..n * n).map(|k| {
            let (i, j) = (k % n, k / n);
            (-r..=r).map(|d| v[((j + d * step.1).clamp(0, n - 1) * n + (i + d * step.0).clamp(0, n - 1)) as usize]).reduce(|a, b| if b.0 < a.0 { b } else { a }).unwrap()
        }).collect()
    };
    pass(&pass(v, (1, 0)), (0, 1))
}

/// Separable box mean of radius `r` texels, edges clamped.
fn box_blur(v: &[f32], n: u32, r: i32) -> Vec<f32> {
    let k = ((2 * r + 1) * (2 * r + 1)) as f32;
    filter(v, n, r, |a, b| a + b).into_iter().map(|s| s / k).collect()
}

/// 3x3 box blur, edges clamped.
fn blur(v: &[f32], n: u32) -> Vec<f32> {
    let n = n as i32;
    (0..n * n).map(|k| {
        let (i, j) = (k % n, k / n);
        let mut s = 0.0;
        for dj in -1..=1 { for di in -1..=1 { s += v[((j + dj).clamp(0, n - 1) * n + (i + di).clamp(0, n - 1)) as usize]; } }
        s / 9.0
    }).collect()
}

/// Extinction per metre at altitude `y` over `ground` for a fog map texel (fog.wesl `fog_density` mirrors it). `dtop` (m) moves the
/// top (the shader's waves and billows); `n` 0..1 is the breakup noise, which erodes the pool into wisps toward its top; `sun_k` = smoothstep(0, lift_full_deg, sun
/// elevation) drives the lift ([`lift_m`]) where the texel's top is sunlit; `sig` is [`sigma`], thinned while the texel's amount is low.
/// The shader also multiplies the result by the noise breakup and the map's edge fade.
pub fn density(c: &GroundFog, texel: [f32; 4], ground: f32, y: f32, sun_k: f32, dtop: f32, n: f32, sig: (f32, f32)) -> f32 {
    let h = y - ground;
    if h < 0.0 { return 0.0; }
    let top = texel[0] + dtop;
    // ponytail: 0.2 is the noise band a wisp edge fades over; a knob when wisps are tuned.
    let thr = c.wisp_erode * (1.0 - ((top - y) / c.wisp_m.max(1e-3)).clamp(0.0, 1.0));
    let wisp = if c.wisp_m > 0.0 { smoothstep(thr, thr + 0.2, n) } else { 1.0 };
    let tail = if c.top_fade_m > 0.0 { (-(y - top).max(0.0) / c.top_fade_m).exp() } else { 0.0 };
    let vert = smoothstep(-0.5, 0.5, (top - y) / c.top_soft_m.max(1e-3)).max(tail);
    let pool = vert * smoothstep(0.0, c.edge_m, top - ground) * smoothstep(0.0, 0.5, texel[3]) * wisp;
    let mist = texel[1] * (-h / c.shallow_e_m.max(1e-3)).exp();
    let lift = lift_m(c, sun_k, lit(c, texel[0].max(ground), texel[2]), texel[3]);
    let base = if lift > 0.0 { smoothstep(lift, lift + c.lift_soft_m, h) } else { 1.0 };
    (sig.0 * pool + sig.1 * mist) * base
}

/// IEEE half-float bits of `v`, rounded to nearest even; beyond the half range it saturates to infinity (callers clamp first).
pub fn f16_bits(v: f32) -> u16 {
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let (exp, man) = (((b >> 23) & 0xff) as i32, b & 0x7f_ffff);
    if exp == 255 { return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 }; }
    let e = exp - 127 + 15;
    if e >= 31 { return sign | 0x7c00; }
    let (m, shift) = if e <= 0 { (man | 0x80_0000, (14 - e) as u32) } else { (man, 13) };
    if shift > 24 { return sign; }
    let base = if e <= 0 { m >> shift } else { ((e as u32) << 10) | (m >> shift) };
    let (rem, mid) = (m & ((1 << shift) - 1), 1 << (shift - 1));
    sign | (base + (rem > mid || rem == mid && base & 1 == 1) as u32) as u16
}

/// Tiling value-noise lattice, `n`^3 bytes (x fastest), one random value per cell; the shader samples it with repeat and a
/// smoothstep-warped coordinate, so one fetch gives smooth value noise.
pub fn noise_lattice(n: u32) -> Vec<u8> {
    (0..n * n * n).map(|k| { let mut h = k.wrapping_mul(747796405).wrapping_add(2891336453); h = ((h >> ((h >> 28) + 4)) ^ h).wrapping_mul(277803737); ((h >> 22) ^ h) as u8 }).collect()
}

/// A test valley (24 km square, 50 m cells): a north-south valley falling 10 m/km to the south between 260 m walls, a mountain ridge
/// east of it (900 m above the wall, crest at x 2600: the morning sun reaches the floor ~20 deg up), rolling hills on the walls and a closed
/// basin in the west hills (centre (-3500, 2500), 800 m radius, 140 m deep). The floor at (0, 0) is 0 m.
pub fn test_valley() -> HeightGrid {
    let (n, cell) = (480u32, 50.0f32);
    let (min_x, max_z) = (-(n as f32) * cell / 2.0, n as f32 * cell / 2.0);
    let y = (0..n * n).map(|k| {
        let (x, z) = (min_x + ((k % n) as f32 + 0.5) * cell, max_z - ((k / n) as f32 + 0.5) * cell);
        let walls = 260.0 * (1.0 - (-(x / 1300.0).powi(2)).exp());
        let hills = 45.0 * (z / 900.0).sin() * (x / 700.0).cos() * smoothstep(400.0, 1500.0, x.abs());
        let basin = -140.0 * (-((x + 3500.0).powi(2) + (z - 2500.0).powi(2)) / (800.0f32 * 800.0)).exp();
        let ridge = 900.0 * (-((x - 2600.0) / 1000.0).powi(2)).exp();
        walls + ridge + hills + basin + 0.01 * z
    }).collect();
    HeightGrid { min_x, max_z, cell_m: cell, w: n, h: n, y }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rain::HeightGrid;

    // spread_c: a morning air 5 K above its dew point (direct evolve calls form nothing); spin-ups integrate their own.
    fn calm() -> Air { Air { coverage: 0.1, wind_mps: 1.0, wetness: 0.0, dust: 0.1, spread_c: 5.0 } }
    fn sunrise(k: &Clock) -> f32 { (0..2400).map(|i| i as f32 * 0.01).find(|&h| crate::sun(k, h).0 > 0.0).unwrap() }

    #[test]
    fn clear_calm_nights_make_fog_that_burns_off_by_midday() {
        let (c, k) = (GroundFog::default(), Clock::default());
        let dawn = spin_up(&c, &AirCfg::default(), &k, 5.5, &calm());
        let noon = spin_up(&c, &AirCfg::default(), &k, 12.0, &calm());
        assert!(dawn > 0.6, "dawn {dawn}");
        assert!(noon < 0.05, "noon {noon}");
        assert!(spin_up(&c, &AirCfg::default(), &k, 22.0, &calm()) < dawn, "fog builds through the night, peaks near sunrise");
    }

    #[test]
    fn wind_cloud_and_dry_air_suppress_it_rain_helps() {
        let (c, k) = (GroundFog::default(), Clock::default());
        let at = |a: Air| spin_up(&c, &AirCfg::default(), &k, 5.5, &a);
        let base = at(calm());
        assert!(at(Air { wind_mps: 8.0, ..calm() }) < 0.02, "wind mixes it out");
        assert!(at(Air { coverage: 0.9, ..calm() }) < base * 0.6, "cloud blocks the cooling");
        assert!(at(Air { dust: 0.6, ..calm() }) < base * 0.5, "dry air");
        assert!(at(Air { wetness: 1.0, ..calm() }) > base, "wet ground after rain");
        assert_eq!(spin_up(&GroundFog { force: Some(0.3), ..c.clone() }, &AirCfg::default(), &k, 12.0, &calm()), 0.3);
    }

    /// Fog forms where the air cools to its dew point, not on every clear calm night. Budget: fog is visibility under 1 km (WMO); the
    /// pool's visibility at amount a is visibility_m / smoothstep(0, 0.5, a) ([`density`]'s amount factor). A humid air mass (the
    /// default: dew point 4 K under the daily mean) reaches it by dawn, a dry one (12 K under) does not. Red arm `private_cooling` (the
    /// old night cooling, blind to the dew point) makes fog in the dry air mass.
    #[test]
    fn fog_waits_for_the_dew_point() {
        let (c, k) = (GroundFog::default(), Clock::default());
        let dry = AirCfg { dew_below_c: 12.0, ..AirCfg::default() };
        let vis = |c: &GroundFog, ac: &AirCfg| c.visibility_m / smoothstep(0.0, 0.5, spin_up(c, ac, &k, 5.5, &calm())).max(1e-6);
        let (humid, dry_v) = (vis(&c, &AirCfg::default()), vis(&c, &dry));
        assert!(humid < 1000.0, "humid air mass: dawn visibility {humid} m is not fog");
        assert!(dry_v >= 1000.0, "dry air mass: dawn visibility {dry_v} m is fog");
        let red = vis(&GroundFog { private_cooling: true, ..c.clone() }, &dry);
        assert!(red < 1000.0, "red arm (private_cooling) visibility {red} m: the gate cannot fail");
    }

    #[test]
    fn burn_off_takes_hours_in_the_sun_and_none_in_shadow() {
        let (c, k) = (GroundFog::default(), Clock::default());
        let rise = sunrise(&k);
        let after = |hours: f32, lit: f32| (0..(hours / 0.05) as u32).fold(1.0, |m, i| evolve(&c, m, 0.05, crate::sun(&k, rise + 0.05 * i as f32).0, &calm(), lit));
        assert!(after(1.0, 1.0) > 0.5, "1 h after sunrise {}", after(1.0, 1.0));
        assert!(after(4.0, 1.0) < 0.2, "4 h after sunrise {}", after(4.0, 1.0));
        assert!(after(4.0, 0.0) > 0.9, "shaded fog does not burn: {}", after(4.0, 0.0));
        assert_eq!(lift_m(&c, 0.0, 1.0, 0.5), 0.0, "no lift before sunrise");
        assert_eq!(lift_m(&c, 1.0, 1.0, 1.0), 0.0, "thick fog sits on the ground");
        assert_eq!(lift_m(&c, 1.0, 0.0, 0.2), 0.0, "fog in shadow is not lifted");
        assert!(lift_m(&c, 1.0, 1.0, 0.2) > lift_m(&c, 1.0, 1.0, 0.6), "thinning sunlit fog lifts");
    }

    /// Density 1 m above the ground at (x, z).
    fn near_ground(c: &GroundFog, m: &FogMap, g: &HeightGrid, x: f32, z: f32) -> f32 {
        let gy = g.height(x, z);
        density(c, m.at(x, z), gy, gy + 1.0, 0.0, 0.0, 0.5, sigma(c))
    }
    fn full(c: &GroundFog, g: &HeightGrid, sun: Vec3) -> FogMap {
        let f = field(c, g, [0.0, 0.0]);
        map(c, &f, &vec![1.0; f.ground.len()], &sun_alt(c, g, &f, sun))
    }

    #[test]
    fn fog_pools_in_the_valley_under_a_level_top_and_leaves_the_ridges() {
        let c = GroundFog::default();
        let g = test_valley();
        let m = full(&c, &g, Vec3::Y);
        let tops: Vec<f32> = (-4..=4).map(|k| m.at(k as f32 * 100.0, 0.0)[0]).collect();
        let spread = tops.iter().cloned().fold(f32::MIN, f32::max) - tops.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread < 6.0, "top across the valley floor is level: {tops:?}");
        assert!(tops[4] > 60.0 && tops[4] < c.depth_max_m + 10.0, "valley pool top {}", tops[4]);
        let (sp, ss) = sigma(&c);
        assert!(near_ground(&c, &m, &g, 0.0, 0.0) > 0.8 * sp, "valley floor is in the pool");
        assert!(m.at(4000.0, 0.0)[0] < g.height(4000.0, 0.0) - 20.0, "ridge stands above the pool top");
        assert!(near_ground(&c, &m, &g, 4000.0, 0.0) < ss * c.flat_share, "ridge keeps only the open-ground mist");
        assert!(near_ground(&c, &m, &g, -3500.0, 2500.0) > 0.8 * sp, "the closed basin pools too");
        let floor = g.height(0.0, 0.0);
        let above = |c: &GroundFog, dy: f32| density(c, m.at(0.0, 0.0), floor, tops[4] + dy, 0.0, 0.0, 0.95, sigma(c));
        assert!(above(&c, 3.0 * c.top_fade_m) < 0.1 * above(&c, -c.wisp_m), "haze above the top fades");
        assert!(above(&GroundFog { top_fade_m: 0.0, wisp_m: 0.0, ..c.clone() }, 2.0 * c.top_soft_m) < 1e-4, "hard top: nothing above");
        // Red arm: a blanket that follows each column's own ground fogs the ridge as much as the floor.
        let blanket = GroundFog { ignore_terrain: true, ..c.clone() };
        assert!(near_ground(&blanket, &full(&blanket, &g, Vec3::Y), &g, 4000.0, 0.0) > 0.5 * sp, "ignore_terrain fogs the ridge");
    }

    #[test]
    fn steep_slopes_drain_their_mist_and_low_sun_leaves_the_floor_in_shadow() {
        let c = GroundFog::default();
        let g = test_valley();
        let m = full(&c, &g, Vec3::new(1.0, 0.08, 0.0).normalize());
        assert!(m.at(1100.0, 0.0)[1] < m.at(0.0, 0.0)[1], "slope {} floor {}", m.at(1100.0, 0.0)[1], m.at(0.0, 0.0)[1]);
        // Sun low in the east: the east wall hides it from the valley floor, the east ridge top sees it.
        assert!(m.at(0.0, 0.0)[2] > g.height(0.0, 0.0) + 50.0, "floor needs {} to see the sun", m.at(0.0, 0.0)[2]);
        assert!(m.at(4000.0, 0.0)[2] < g.height(4000.0, 0.0), "east ridge is sunlit");
        assert_eq!(full(&c, &g, Vec3::new(0.0, -0.1, 1.0).normalize()).at(0.0, 0.0)[2], NO_SUN);
    }

    #[test]
    fn a_valley_in_mountain_shadow_keeps_its_fog_after_open_ground_clears() {
        let (c, k) = (GroundFog::default(), Clock::default());
        let g = test_valley();
        let f = field(&c, &g, [0.0, 0.0]);
        let flat = HeightGrid::flat(0.0);
        let ff = field(&c, &flat, [0.0, 0.0]);
        // The east ridge hides the morning sun from the floor until it is ~20 deg up, about 1.75 h after sunrise.
        let h = sunrise(&k) + 1.5;
        let (m, open) = (spin_up_local(&c, &AirCfg::default(), &k, h, &calm(), &g, &f), spin_up_local(&c, &AirCfg::default(), &k, h, &calm(), &flat, &ff));
        let floor = f.texel(0.0, 0.0).unwrap();
        let o = ff.texel(0.0, 0.0).unwrap();
        assert!((open[o] - spin_up(&c, &AirCfg::default(), &k, h, &calm())).abs() < 1e-5, "flat ground is all sunlit: {} vs {}", open[o], spin_up(&c, &AirCfg::default(), &k, h, &calm()));
        assert!(m[floor] > open[o] + 0.4, "1.5 h after sunrise the shaded valley floor keeps {} vs open ground {}", m[floor], open[o]);
        let red = spin_up_local(&GroundFog { ignore_shadow: true, ..c.clone() }, &AirCfg::default(), &k, h, &calm(), &g, &f);
        assert!((red[floor] - open[o]).abs() < 0.05, "red arm (ignore_shadow) burns the floor like open ground: {} vs {}", red[floor], open[o]);
        let fm = map(&c, &f, &m, &sun_alt(&c, &g, &f, crate::dir(crate::sun(&k, h).0, crate::sun(&k, h).1)));
        assert_eq!(fm.at(0.0, 0.0)[3], m[floor], "the map carries the amount");
    }

    #[test]
    fn f16_bits_round_trip_known_values() {
        for (v, h) in [(1.0, 0x3c00), (-2.0, 0xc000), (0.5, 0x3800), (65504.0, 0x7bff), (0.0, 0), (1e-8, 0), (6.1035156e-5, 0x0400), (1.0 + 1.0 / 2048.0, 0x3c00), (1.0 + 3.0 / 2048.0, 0x3c02), (1e6, 0x7c00)] {
            assert_eq!(f16_bits(v), h, "{v}");
        }
    }

    #[test]
    fn noise_lattice_is_spread_and_deterministic() {
        let (a, b) = (noise_lattice(16), noise_lattice(16));
        assert_eq!((a.len(), &a), (4096, &b));
        let mean = a.iter().map(|&v| v as f32).sum::<f32>() / a.len() as f32;
        assert!((mean - 127.5).abs() < 8.0, "mean {mean}");
        assert!(a.iter().any(|&v| v < 16) && a.iter().any(|&v| v > 240), "covers the range");
    }

    #[test]
    fn wisps_erode_the_top_not_the_core() {
        let c = GroundFog::default();
        let texel = [100.0, 0.0, NO_SUN, 1.0];
        let at = |c: &GroundFog, y: f32, n: f32| density(c, texel, 0.0, y, 0.0, 0.0, n, sigma(c));
        let (low, high) = (0.3, 0.9);
        assert_eq!(at(&c, 20.0, low), at(&c, 20.0, high), "deep in the pool the noise does not erode");
        assert!(at(&c, 95.0, low) < 0.2 * at(&c, 95.0, high), "near the top low noise is eroded away: {} vs {}", at(&c, 95.0, low), at(&c, 95.0, high));
        let hard = GroundFog { wisp_m: 0.0, ..c.clone() };
        assert_eq!(at(&hard, 95.0, low), at(&hard, 95.0, high), "red arm: wisp_m 0 keeps the hard top");
    }

    #[test]
    fn flat_ground_gets_a_shallow_level_layer_and_burn_lift_clears_the_ground() {
        let c = GroundFog::default();
        let flat = HeightGrid::flat(10.0);
        let f = field(&c, &flat, [123.0, -456.0]);
        let m = map(&c, &f, &vec![0.5; f.ground.len()], &sun_alt(&c, &flat, &f, Vec3::Y));
        assert!((m.at(0.0, 0.0)[0] - (10.0 + 0.5 * c.depth_flat_m)).abs() < 1e-3, "top {}", m.at(0.0, 0.0)[0]);
        assert!((m.min_x + c.extent_m / 2.0) % m.cell_m == 0.0, "centre snapped to texels");
        let s = sigma(&c);
        let thin = [m.at(0.0, 0.0)[0], m.at(0.0, 0.0)[1], m.at(0.0, 0.0)[2], 0.2];
        assert!(density(&c, thin, 10.0, 11.0, 1.0, 0.0, 0.5, s) < 1e-4, "sunlit thinning fog lifts off the ground");
        assert!(density(&c, thin, 10.0, 11.0, 0.0, 0.0, 0.5, s) > 0.0, "before sunrise it sits on it");
        assert!(m.y_lo <= 10.0 && m.y_hi >= m.at(0.0, 0.0)[0], "slab bounds {} {}", m.y_lo, m.y_hi);
        let moved = field(&c, &flat, [3000.0, 0.0]);
        let r = resample(&f, &vec![0.7; f.ground.len()], &moved, 0.1);
        assert_eq!((r[moved.texel(0.0, 0.0).unwrap()], r[moved.texel(8900.0, 0.0).unwrap()]), (0.7, 0.1), "carried inside, fallback outside");
    }
}
