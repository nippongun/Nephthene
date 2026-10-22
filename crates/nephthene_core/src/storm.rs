//! Storm fronts, the pure timetable and profile maths.
//! No bevy. Storm time = game seconds x `clock_scale`; the caller converts. Ground plane is (x, z) in `Vec2` (x, y);
//! altitude is metres. `sky_scale` multiplies heights, radii, anvils and line lengths at birth. All tunables are in [`StormCfg`], all untuned.
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use crate::rain::{pcg, smoothstep};

/// GPU layout: 4 vec4 per cell, at most `MAX_CELLS` cells (uniform is `MAX_CELLS * PACK_VEC4` vec4).
pub const PACK_VEC4: usize = 4;
pub const MAX_CELLS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Cell, Line }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage { Developing, Mature, Dissipating }

/// Birth record, immutable. Sizes are already multiplied by `sky_scale`.
#[derive(Clone, Debug, PartialEq)]
pub struct Storm {
    pub id: u64, pub kind: Kind, pub born_s: f64, pub life_s: f32,
    pub origin: Vec2, pub vel: Vec2, pub half_axis: Vec2,
    pub core_r: f32, pub base: f32, pub top: f32, pub anvil_r: f32, pub intensity: f32,
}

/// State at storm time t. `a`..`b` is the axis (a == b for a cell).
#[derive(Clone, Debug, PartialEq)]
pub struct Now {
    pub a: Vec2, pub b: Vec2, pub dir: Vec2, pub stage: Stage, pub age: f32, pub base: f32,
    pub core_r: f32, pub top: f32, pub overshoot: f32, pub anvil_r: f32, pub anvil_shift: Vec2, pub anvil_base: f32,
    pub precip: f32, pub gust_r: f32, pub shelf_low: f32, pub outflow_mps: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct AuthoredCfg {
    pub name: String, pub scene: String, pub born_hour: f32, pub bearing_deg: f32, pub dist_m: f32,
    pub vel_mps: [f32; 2], pub kind: String, pub core_r_m: f32, pub top_m: f32, pub base_m: f32,
    pub anvil_r_m: f32, pub intensity: f32, pub life_min: f32, pub half_len_m: f32,
}
impl Default for AuthoredCfg {
    fn default() -> Self {
        Self { name: String::new(), scene: String::new(), born_hour: 14.0, bearing_deg: 0.0, dist_m: 0.0, vel_mps: [0.0, 0.0],
               kind: "cell".into(), core_r_m: 6000.0, top_m: 12000.0, base_m: 2800.0, anvil_r_m: 30000.0, intensity: 1.0,
               life_min: 45.0, half_len_m: 60000.0 }
    }
}

/// The storms section of the config. Ranges are [lo, hi].
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct StormCfg {
    /// Seed text of the timetable (same seed, same storms).
    pub seed: String,
    /// Lifecycle fractions of life: developing < dev_end <= mature < diss_start <= dissipating; size ramps 0 -> 1 over the first and last `edge` of life (no pop).
    pub dev_end: f32, pub diss_start: f32, pub edge: f32,
    /// Profile shape. Tower flare: the core radius grows by this fraction between 30% and 100% of the height, so the tower opens smoothly into the anvil.
    /// Anvil: widest at `anvil_wide` x reach downwind, `anvil_p` peak density. Shelf: thickness above the base in shelf drops at the wedge's storm edge.
    pub core_flare: f32, pub anvil_wide: f32, pub anvil_p: f32, pub shelf_ceil: f32,
    /// Slot hysteresis: a storm already selected has its distance multiplied by this (no slot flicker, `select_hysteresis`).
    pub select_hysteresis: f32,
    /// Genesis from the weather map: cloud type ramps in between these, scaled by (floor + (1 - floor) * precip). Timetable rebuild thresholds (storm s, m).
    pub genesis_type: [f32; 2], pub genesis_floor: f32, pub rebuild_s: f64, pub rebuild_m: f32,
    pub render: StormRender,
    pub fx: StormFx,
    /// Lightning: strike, channel, strokes, light (on by default).
    pub lightning: crate::lightning::Lightning,
    /// Force this authored storm (by name) alone, timetable off. And a scene tag: authored storms with that `scene` join the timetable.
    pub force: Option<String>, pub scene: Option<String>,
    pub enabled: bool, pub sky_scale: f32, pub clock_scale: f32,
    /// Storms drive the weather at the camera (rain, cover, gust front, sky light) and the rain shafts. False: test red arm, the
    /// old storms that only draw clouds.
    pub couple: bool,
    /// "Storm now" (panel button, `summon_at_start`): a mature cell this far upwind of the camera (m), drifting over it, summoned
    /// at this fraction of its life (`dev_end` < it < `diss_start`: mature, raining); `summon_at_start` summons one on the first frame
    /// (demo and test switch).
    pub summon_dist_m: f32, pub summon_age: f32, pub summon_at_start: bool,
    pub bin_m: f32, pub bin_min: f32, pub window_m: f32, pub rate_per_km2_hr: f32,
    pub diurnal_peak_hour: f32, pub diurnal_amp: f32, pub line_frac: f32,
    pub tropopause_m: f32, pub base_m: [f32; 2], pub top_frac: [f32; 2], pub core_r_m: [f32; 2], pub anvil_r_m: [f32; 2],
    pub line_len_m: [f32; 2], pub line_half_width_m: [f32; 2], pub cell_life_min: [f32; 2], pub line_life_min: [f32; 2],
    pub speed_mps: [f32; 2], pub steer_deg: f32, pub steer_jitter_deg: f32, pub intensity: [f32; 2],
    pub overshoot_m: f32, pub anvil_thickness_m: f32, pub shear_deg: f32, pub shelf_drop_m: f32, pub shelf_height_m: f32, pub gust_speed_mps: f32,
    pub authored: Vec<AuthoredCfg>,
}
impl Default for StormCfg {
    fn default() -> Self {
        Self { seed: "nephthene".into(), dev_end: 0.3, diss_start: 0.7, edge: 0.05, core_flare: 0.8, anvil_wide: 0.55, anvil_p: 0.85, shelf_ceil: 1.0,
               select_hysteresis: 0.8, genesis_type: [0.4, 0.9], genesis_floor: 0.3, rebuild_s: 60.0, rebuild_m: 2000.0, render: StormRender::default(), fx: StormFx::default(), lightning: Default::default(), force: None, scene: None,
               enabled: true, sky_scale: 1.0, clock_scale: 0.25, couple: true, summon_dist_m: 5000.0, summon_age: 0.45, summon_at_start: false, bin_m: 20000.0, bin_min: 30.0, window_m: 120000.0,
               rate_per_km2_hr: 1.0e-3, diurnal_peak_hour: 16.0, diurnal_amp: 0.9, line_frac: 0.1,
               tropopause_m: 13000.0, base_m: [2400.0, 3200.0], top_frac: [0.75, 1.0], core_r_m: [3000.0, 8000.0],
               anvil_r_m: [30000.0, 90000.0], line_len_m: [60000.0, 300000.0], line_half_width_m: [7500.0, 15000.0],
               cell_life_min: [30.0, 60.0], line_life_min: [180.0, 540.0], speed_mps: [6.0, 15.0], steer_deg: 45.0,
               steer_jitter_deg: 25.0, intensity: [0.5, 1.0], overshoot_m: 1200.0, anvil_thickness_m: 5000.0, shear_deg: 55.0,
               shelf_drop_m: 500.0, shelf_height_m: 800.0, gust_speed_mps: 12.0, authored: vec![] }
    }
}
/// Constants of the storm part of the cloud shader (clouds_common.wesl / clouds_march.wesl), as uniforms.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct StormRender {
    /// Noise frequency multiplier on the base shape (cell-sized billows) and vertical squash (taller features).
    pub freq: f32, pub vsq: f32,
    /// Vertical squash near the base (flat strata), the sky-visibility floor under a cell (slate, not black) and its noise variation.
    pub vsq_low: f32, pub amb_floor: f32, pub amb_var: f32,
    /// Carve: density = saturate((profile - erode * (1 - noise)) * gain).
    pub erode: f32, pub gain: f32,
    /// Optical depth per metre at profile 1 for analytic chords, and the sky-ambient attenuation per unit column optical depth.
    pub od_per_m: f32, pub amb_k: f32,
    /// Detail erosion only inside this ray distance (m); the detail tile is point-sampled, so further out it only aliases.
    pub detail_m: f32,
    /// Light floor of field clouds under a storm.
    pub field_floor: f32,
    /// Ray march: range (m), max steps, min step (m); where the light march ends and the analytic sun chord starts, sun chord and column lengths (m).
    pub range_m: f32, pub steps: f32, pub ds_min_m: f32, pub near_m: f32, pub sun_len_m: f32, pub col_len_m: f32,
}
impl Default for StormRender {
    fn default() -> Self {
        StormRender { freq: 2.5, vsq: 1.2, vsq_low: 3.0, amb_floor: 0.12, amb_var: 0.5, erode: 0.7, gain: 2.5, od_per_m: 1.0 / 2000.0, amb_k: 1.5,
                      detail_m: 20000.0, field_floor: 0.12, range_m: 200_000.0, steps: 64.0, ds_min_m: 150.0, near_m: 2300.0, sun_len_m: 18000.0, col_len_m: 18000.0 }
    }
}

/// Storm consumers (storm_fx): precip curtain, outflow, sun occlusion, flashes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct StormFx {
    /// Core fraction inside which the curtain is full strength.
    pub curtain_full: f32,
    /// Sound speed, m/s.
    pub sound_mps: f32,
    /// Optical depth per metre at profile 1; quadrature step (m); sun path length searched (m).
    pub od_ext_per_m: f32, pub od_step_m: f32, pub sun_path_m: f32,
    /// Mature-cell flash rate per storm second; flash decision bin (storm s); fraction cloud-to-ground; hash salt.
    pub flash_rate_per_s: f64, pub flash_dt_s: f64, pub cg_frac: f32, pub flash_salt: u32,
    /// Sky ambient attenuation per unit column optical depth; height of the column searched (m).
    pub amb_k: f32, pub column_m: f32,
    /// Cloud cover overhead per unit column optical depth: cover = 1 - exp(-cover_k x od).
    pub cover_k: f32,
}
impl Default for StormFx {
    fn default() -> Self {
        StormFx { curtain_full: 0.6, sound_mps: 343.0, od_ext_per_m: 1.0 / 8000.0, od_step_m: 250.0, sun_path_m: 20000.0, flash_rate_per_s: 1.0 / 30.0,
                  flash_dt_s: 0.5, cg_frac: 0.25, flash_salt: 0x0F1A_5C0D, amb_k: 0.5, column_m: 20000.0, cover_k: 3.0 }
    }
}

impl StormCfg {
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> { serde_json::from_str(s) }
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let s = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_json(&s).map_err(|e| format!("{}: {e}", path.display()))
    }
    fn max_life_s(&self) -> f64 { self.cell_life_min[1].max(self.line_life_min[1]) as f64 * 60.0 }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t }
fn nearest(a: Vec2, b: Vec2, p: Vec2) -> Vec2 {
    let ab = b - a; let l2 = ab.length_squared();
    if l2 < 1e-6 { return a; }
    a + ab * ((p - a).dot(ab) / l2).clamp(0.0, 1.0)
}

/// Deterministic uniform stream on the rain.rs PCG.
struct Rng(u32);
impl Rng {
    fn next(&mut self) -> f32 { self.0 = pcg(self.0.wrapping_add(0x9E37_79B9)); (self.0 >> 8) as f32 / 16777216.0 }
    fn range(&mut self, r: [f32; 2]) -> f32 { lerp(r[0], r[1], self.next()) }
}
fn fnv(s: &str) -> u32 { s.bytes().fold(0x811C_9DC5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193)) }
const SALT: u32 = 0x5702_A11C; // fixed salt; the seed is `StormCfg::seed`
fn bin_hash(seed: &str, ix: i32, iz: i32, tb: i64) -> u32 {
    pcg(pcg(pcg(pcg(fnv(seed) ^ SALT) ^ ix as u32) ^ iz as u32) ^ (tb as u32) ^ ((tb >> 32) as u32).wrapping_mul(0x85EB_CA6B))
}

/// Monsoon afternoon peak; mean over a day is 1.
pub fn diurnal(c: &StormCfg, hour: f32) -> f32 { 1.0 + c.diurnal_amp * ((hour - c.diurnal_peak_hour) / 24.0 * std::f32::consts::TAU).cos() }
/// Expected births in one space-time bin. `g` = genesis at the bin centre; `mid_s` = bin middle in storm seconds.
pub fn lambda(c: &StormCfg, g: f32, mid_s: f64) -> f32 {
    let hour = ((mid_s / c.clock_scale as f64) / 3600.0).rem_euclid(24.0) as f32;
    g * diurnal(c, hour) * c.rate_per_km2_hr * (c.bin_m / 1000.0).powi(2) * (c.bin_min / 60.0)
}

fn births_in_bin(c: &StormCfg, seed: &str, g: f32, ix: i32, iz: i32, tb: i64) -> Vec<Storm> {
    let bin_s = c.bin_min as f64 * 60.0;
    let mut r = Rng(bin_hash(seed, ix, iz, tb));
    let lam = lambda(c, g, (tb as f64 + 0.5) * bin_s);
    let (u, mut n, mut p, mut cum) = (r.next(), 0usize, (-lam).exp(), 0.0f32);
    cum += p;
    while u > cum && n < 4 { n += 1; p *= lam / n as f32; cum += p; }
    let sc = c.sky_scale;
    (0..n).map(|i| {
        let born_s = (tb as f64 + r.next() as f64) * bin_s;
        let origin = Vec2::new((ix as f32 + r.next()) * c.bin_m, (iz as f32 + r.next()) * c.bin_m);
        let line = r.next() < c.line_frac;
        let ang = (c.steer_deg + (r.next() - 0.5) * 2.0 * c.steer_jitter_deg).to_radians();
        let dir = Vec2::new(ang.cos(), ang.sin());
        let vel = dir * r.range(c.speed_mps);
        let (life_min, core, half_axis) = if line {
            let len = r.range(c.line_len_m) * sc;
            (r.range(c.line_life_min), r.range(c.line_half_width_m) * sc, dir.perp() * len * 0.5)
        } else { (r.range(c.cell_life_min), r.range(c.core_r_m) * sc, Vec2::ZERO) };
        let base = r.range(c.base_m) * sc;
        let top = (c.tropopause_m * r.range(c.top_frac) * sc).max(base + 1000.0 * sc);
        let anvil_r = r.range(c.anvil_r_m) * sc;
        let intensity = r.range(c.intensity);
        let hi = pcg(bin_hash(seed, ix, iz, tb) ^ (i as u32 + 1));
        Storm { id: ((hi as u64) << 32) | pcg(hi ^ 1) as u64, kind: if line { Kind::Line } else { Kind::Cell },
                born_s, life_s: life_min * 60.0, origin, vel, half_axis, core_r: core, base, top, anvil_r, intensity }
    }).collect()
}

/// Every timetable birth with born_s in [t0, t1] and bin centre within `window_m` of `centre`.
pub fn births(c: &StormCfg, seed: &str, genesis: &dyn Fn(Vec2) -> f32, t0: f64, t1: f64, centre: Vec2) -> Vec<Storm> {
    let bin_s = c.bin_min as f64 * 60.0;
    let (ix0, ix1) = (((centre.x - c.window_m) / c.bin_m).floor() as i32, ((centre.x + c.window_m) / c.bin_m).floor() as i32);
    let (iz0, iz1) = (((centre.y - c.window_m) / c.bin_m).floor() as i32, ((centre.y + c.window_m) / c.bin_m).floor() as i32);
    let mut out = vec![];
    for tb in (t0 / bin_s).floor() as i64..=(t1 / bin_s).floor() as i64 {
        for iz in iz0..=iz1 { for ix in ix0..=ix1 {
            let m = Vec2::new((ix as f32 + 0.5) * c.bin_m, (iz as f32 + 0.5) * c.bin_m);
            if m.distance(centre) > c.window_m { continue; }
            out.extend(births_in_bin(c, seed, genesis(m), ix, iz, tb).into_iter().filter(|s| s.born_s >= t0 && s.born_s <= t1));
        } }
    }
    out
}

/// Storms alive at storm time `t` near `centre`. Pure in (cfg, seed, genesis, t, centre): the clock slider scrubs.
pub fn timetable(c: &StormCfg, seed: &str, genesis: &dyn Fn(Vec2) -> f32, t: f64, centre: Vec2) -> Vec<Storm> {
    if !c.enabled { return vec![]; }
    let mut v = births(c, seed, genesis, t - c.max_life_s(), t, centre);
    v.retain(|s| s.born_s + s.life_s as f64 >= t);
    v
}

/// storms.json "authored" storms. `t0` = storm seconds at game-day start; `centre` = scene centre.
/// (Signature gains `centre` vs the plan: bearing/distance need an origin.)
pub fn authored(c: &StormCfg, t0: f64, centre: Vec2) -> Vec<Storm> {
    let sc = c.sky_scale;
    c.authored.iter().enumerate().map(|(i, a)| {
        let b = a.bearing_deg.to_radians();
        let origin = centre + Vec2::new(b.sin(), -b.cos()) * a.dist_m; // bearing: 0 = -z (north), 90 = +x (east)
        let vel = Vec2::from(a.vel_mps);
        let line = a.kind == "line";
        let dir = if vel.length() > 1e-3 { vel.normalize() } else { Vec2::X };
        Storm { id: ((fnv(&a.name) as u64) << 32) | (0xA000_0000 + i as u64), kind: if line { Kind::Line } else { Kind::Cell },
                born_s: t0 + a.born_hour as f64 * 3600.0 * c.clock_scale as f64, life_s: a.life_min * 60.0,
                origin, vel, half_axis: if line { dir.perp() * a.half_len_m * sc } else { Vec2::ZERO },
                core_r: a.core_r_m * sc, base: a.base_m * sc, top: a.top_m * sc, anvil_r: a.anvil_r_m * sc, intensity: a.intensity }
    }).collect()
}

/// A mature cell `summon_dist_m` from `cam` toward `from` (map frame; `from` is the direction the wind comes from, any length,
/// +x if zero), moving back over `cam` at the middle of `speed_mps`, `summon_age` through its life. Sizes are the middles of their
/// ranges (x `sky_scale`), intensity the top of `intensity`, life the middle of `cell_life_min`.
pub fn summon(c: &StormCfg, id: u64, t: f64, cam: Vec2, from: Vec2) -> Storm {
    let d = from.normalize_or(Vec2::X);
    let mid = |r: [f32; 2]| 0.5 * (r[0] + r[1]);
    let sc = c.sky_scale;
    let life_s = mid(c.cell_life_min) * 60.0;
    let base = mid(c.base_m) * sc;
    let vel = -d * mid(c.speed_mps);
    let age = c.summon_age * life_s;
    Storm { id, kind: Kind::Cell, born_s: t - age as f64, life_s, origin: cam + d * c.summon_dist_m - vel * age, vel,
            half_axis: Vec2::ZERO, core_r: mid(c.core_r_m) * sc, base, top: (c.tropopause_m * mid(c.top_frac) * sc).max(base + 1000.0 * sc),
            anvil_r: mid(c.anvil_r_m) * sc, intensity: c.intensity[1] }
}

/// Anvil shear direction: the storm's heading turned by `shear_deg` (upper wind is not the steering wind). `Now::anvil_shift` is this x 0.5 anvil_r.
pub fn shear_dir(dir: Vec2, c: &StormCfg) -> Vec2 {
    let a = c.shear_deg.to_radians();
    Vec2::new(dir.x * a.cos() - dir.y * a.sin(), dir.x * a.sin() + dir.y * a.cos())
}

/// State at storm time t; None outside [born, born + life].
pub fn now(s: &Storm, t: f64, c: &StormCfg) -> Option<Now> {
    let age = (t - s.born_s) as f32;
    if age < 0.0 || age > s.life_s { return None; }
    let f = age / s.life_s;
    let sc = c.sky_scale;
    let (dev_end, diss_start, edge) = (c.dev_end, c.diss_start, c.edge);
    let env = smoothstep(0.0, edge, f) * (1.0 - smoothstep(1.0 - edge, 1.0, f));
    let dev = smoothstep(0.0, 1.0, (f / dev_end).min(1.0));
    let stage = if f < dev_end { Stage::Developing } else if f < diss_start { Stage::Mature } else { Stage::Dissipating };
    let diss = smoothstep(diss_start, 1.0, f);
    let mature_in = smoothstep(dev_end, dev_end + 0.2, f);
    let full = s.top - s.base;
    let depth = lerp((1000.0 * sc).min(full), full, dev) * env;
    let top = s.base + depth;
    let core_r = s.core_r * (0.5 + 0.5 * dev) * (1.0 - 0.4 * diss) * env;
    let dir = if s.vel.length() > 1e-3 { s.vel.normalize() } else { Vec2::X };
    let pos = s.origin + s.vel * age;
    let anvil_r = s.anvil_r * smoothstep(dev_end, diss_start, f) * (1.0 - 0.5 * diss) * env;
    let precip = s.intensity * smoothstep(dev_end, dev_end + 0.15, f) * (1.0 - diss) * env;
    let since = (age - dev_end * s.life_s).max(0.0);
    let gust_r = (c.gust_speed_mps * sc * since + core_r * smoothstep(dev_end, dev_end + 0.1, f)) * env;
    Some(Now {
        a: pos - s.half_axis, b: pos + s.half_axis, dir, stage, age, base: s.base, core_r, top,
        overshoot: c.overshoot_m * sc * mature_in * (1.0 - smoothstep(diss_start, diss_start + 0.15, f)) * env,
        anvil_r, anvil_shift: shear_dir(dir, c) * 0.5 * anvil_r, anvil_base: (top - c.anvil_thickness_m * sc).max(s.base),
        precip, gust_r, shelf_low: s.base - c.shelf_drop_m * sc * mature_in * (1.0 - diss) * env,
        outflow_mps: if s.intensity > 1e-3 { c.gust_speed_mps * precip / s.intensity } else { 0.0 },
    })
}

/// Top-k nearest to `cam` (distance to the storm's outer edge). Storms in `prev` get a distance bonus (hysteresis,
/// no slot flicker). Returned sorted by id so GPU slots are stable.
pub fn select(all: &[Storm], t: f64, cam: Vec2, k: usize, prev: &[u64], c: &StormCfg) -> Vec<(Storm, Now)> {
    let mut v: Vec<(f32, Storm, Now)> = all.iter().filter_map(|s| {
        let n = now(s, t, c)?;
        let d = (nearest(n.a, n.b, cam).distance(cam) - n.anvil_r.max(n.core_r)).max(0.0);
        Some((if prev.contains(&s.id) { d * c.select_hysteresis } else { d }, s.clone(), n))
    }).collect();
    v.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.id.cmp(&y.1.id)));
    v.truncate(k);
    v.sort_by_key(|x| x.1.id);
    v.into_iter().map(|(_, s, n)| (s, n)).collect()
}

/// Analytic cloud envelope 0..1 at p (x, altitude, z). Zero above top+overshoot and below shelf_low; the anvil top is
/// flat at `top`; the shelf (a wedge between shelf_low and the base) exists only ahead of the axis along `dir`.
/// clouds_common.wesl mirrors this (shape values arrive as uniforms).
pub fn profile(c: &StormCfg, n: &Now, p: Vec3) -> f32 {
    let g = Vec2::new(p.x, p.z);
    let q = nearest(n.a, n.b, g);
    let d = g.distance(q);
    let mut v = 0.0f32;
    if n.core_r > 1.0 && n.top > n.base {
        let hf = ((p.y - n.base) / (n.top - n.base).max(1.0)).clamp(0.0, 1.0);
        let rc = n.core_r * (1.0 + c.core_flare * smoothstep(0.3, 1.0, hf));
        let dome = n.overshoot * (1.0 - (d / (0.8 * n.core_r * (1.0 + c.core_flare))).powi(2)).max(0.0);
        let rise = (n.top - n.base).min(1000.0).max(1.0);
        let vert = smoothstep(n.base, n.base + 0.5 * rise, p.y) * (1.0 - smoothstep(n.top + dome - 0.5 * rise, n.top + dome, p.y));
        let vert = if p.y > n.top + dome { 0.0 } else { vert };
        v = v.max((1.0 - smoothstep(0.6, 1.0, d / rc)) * vert);
    }
    if n.anvil_r > 1.0 && n.core_r > 1.0 && p.y >= n.anvil_base && p.y <= n.top {
        let sd = n.anvil_shift.try_normalize().unwrap_or(n.dir);
        let r = g - q;
        let (s, t) = (r.dot(sd), r.dot(sd.perp()));
        let hw0 = n.core_r * (1.0 + c.core_flare);
        let u = s / n.anvil_r;
        let w = if s >= 0.0 {
            (hw0 + (c.anvil_wide * n.anvil_r - hw0).max(0.0) * smoothstep(0.0, 0.5, u)) * (1.0 - smoothstep(0.55, 1.0, u)).sqrt()
        } else { hw0 * (1.0 - (s / hw0).powi(2)).max(0.0).sqrt() };
        if w > 1.0 {
            let thick = (n.top - n.anvil_base).max(1.0);
            let yb = n.anvil_base + 0.6 * thick * smoothstep(0.2, 1.0, u.clamp(0.0, 1.0));
            let a = c.anvil_p * (1.0 - smoothstep(0.6, 1.0, t.abs() / w)) * (1.0 - 0.7 * smoothstep(0.4, 1.0, u.clamp(0.0, 1.0)))
                * smoothstep(yb, yb + 0.3 * thick, p.y);
            v = v.max(a);
        }
    }
    if n.shelf_low < n.base && n.gust_r > n.core_r && (g - q).dot(n.dir) > 0.0 && p.y >= n.shelf_low && p.y <= n.base + (n.base - n.shelf_low) * 2.0 {
        let dr = n.base - n.shelf_low;
        let ef = ((d - n.core_r) / (n.gust_r - n.core_r).max(1.0)).clamp(0.0, 1.0);
        let radial = smoothstep(n.core_r * 0.8, n.core_r * 1.2, d) * (1.0 - smoothstep(0.7, 1.0, d / n.gust_r));
        // Floor: lowest at the leading edge, rising toward the storm. Ceiling thins toward the leading edge.
        let floor = n.base - dr * smoothstep(0.0, 0.7, ef);
        let ceil = n.base + dr * (c.shelf_ceil * (1.0 - ef) + 0.5);
        let vert = smoothstep(floor, floor + 0.35 * dr, p.y) * (1.0 - smoothstep(ceil - 0.5 * dr, ceil, p.y));
        // An arc facing `dir`, not a disc: it thins to nothing toward the sides.
        let ahead = smoothstep(0.4, 0.95, (g - q).dot(n.dir) / d.max(1.0));
        v = v.max(0.8 * radial * vert * ahead);
    }
    v.clamp(0.0, 1.0)
}

/// GPU layout, `PACK_VEC4` vec4 per cell (clouds_common.wesl `StormCell` must match; text-pin test in W2a):
///   v0 = (a.x, a.z, b.x, b.z)   v1 = (core_r, base, top, overshoot)
///   v2 = (anvil_r, shift.x, shift.z, anvil_base)   v3 = (precip, gust_r, shelf_low, dir_angle_rad)
/// Returns `PACK_VEC4 * min(n, MAX_CELLS)` vec4; the host zero-pads to the uniform size.
pub fn pack(sel: &[(Storm, Now)]) -> Vec<[f32; 4]> {
    sel.iter().take(MAX_CELLS).flat_map(|(_, n)| [
        [n.a.x, n.a.y, n.b.x, n.b.y], [n.core_r, n.base, n.top, n.overshoot],
        [n.anvil_r, n.anvil_shift.x, n.anvil_shift.y, n.anvil_base], [n.precip, n.gust_r, n.shelf_low, n.dir.y.atan2(n.dir.x)],
    ]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tcfg() -> StormCfg { StormCfg { enabled: true, ..StormCfg::default() } }
    fn g1(_: Vec2) -> f32 { 1.0 }
    fn gx(p: Vec2) -> f32 { 0.5 + 0.5 * (p.x / 30000.0).sin() }
    fn cell() -> Storm {
        Storm { id: 7, kind: Kind::Cell, born_s: 1000.0, life_s: 2400.0, origin: Vec2::new(5000.0, -3000.0), vel: Vec2::new(8.0, 4.0),
                half_axis: Vec2::ZERO, core_r: 6000.0, base: 2800.0, top: 12000.0, anvil_r: 30000.0, intensity: 1.0 }
    }
    fn line() -> Storm { Storm { kind: Kind::Line, half_axis: Vec2::new(-4.0, 8.0).normalize() * 60000.0, core_r: 10000.0, ..cell() } }
    fn at(s: &Storm, f: f32) -> Now { now(s, s.born_s + (f * s.life_s) as f64, &tcfg()).unwrap() }

    #[test]
    fn determinism_bitwise() {
        let c = tcfg();
        let run = || { let v = timetable(&c, "seed-a", &gx, 3.0e5, Vec2::ZERO); let s = select(&v, 3.0e5, Vec2::ZERO, 16, &[], &c); (format!("{v:?}"), pack(&s).iter().flatten().map(|x| x.to_bits()).collect::<Vec<_>>()) };
        let (a, b) = (run(), run());
        assert_eq!(a, b);
        assert_ne!(format!("{:?}", births(&c, "seed-a", &gx, 0.0, 3.0e5, Vec2::ZERO)), format!("{:?}", births(&c, "Other", &gx, 0.0, 3.0e5, Vec2::ZERO)));
        // scrub: the same storm is the same storm whichever t you ask at
        let (t1, t2) = (3.0e5, 3.0e5 + 600.0);
        let (v1, v2) = (timetable(&c, "seed-a", &gx, t1, Vec2::ZERO), timetable(&c, "seed-a", &gx, t2, Vec2::ZERO));
        for s in v1.iter().filter(|s| s.born_s + s.life_s as f64 >= t2) { assert!(v2.contains(s)); }
    }

    #[test]
    fn no_pop() {
        let c = tcfg();
        for s in [cell(), line()] {
            let eps = 40.0 * (s.top - s.base).max(s.anvil_r).max(s.core_r) / s.life_s; // m per storm second, from the EDGE ramp budget
            let (mut prev, mut mx) = (now(&s, s.born_s, &c).unwrap(), 0.0f32);
            assert!(prev.core_r == 0.0 && prev.top == s.base && prev.anvil_r == 0.0 && prev.precip == 0.0 && prev.gust_r == 0.0);
            for i in 1..=s.life_s as i32 {
                let n = now(&s, s.born_s + i as f64, &c).unwrap();
                mx = mx.max((n.core_r - prev.core_r).abs()).max((n.top - prev.top).abs()).max((n.anvil_r - prev.anvil_r).abs())
                    .max((n.gust_r - prev.gust_r).abs()).max((n.overshoot - prev.overshoot).abs());
                prev = n;
            }
            assert!(mx <= eps, "step {mx} > {eps}");
            assert!(prev.core_r == 0.0 && prev.top == s.base && prev.anvil_r == 0.0 && prev.precip == 0.0 && prev.gust_r == 0.0);
            assert!(now(&s, s.born_s - 1.0, &c).is_none() && now(&s, s.born_s + s.life_s as f64 + 1.0, &c).is_none());
        }
    }

    #[test]
    fn lifecycle() {
        let s = cell();
        let mut last = 0.0;
        for i in 0..=300 {
            let n = at(&s, i as f32 / 1000.0);
            assert!(n.top >= last - 1e-3 && n.top <= s.top + 1e-3);
            assert_eq!(n.precip, 0.0, "precip while developing");
            assert_eq!(n.anvil_r, 0.0, "anvil before mature");
            last = n.top;
        }
        assert!((at(&s, tcfg().dev_end).top - s.top).abs() < 1.0);
        assert_eq!(at(&s, 0.5).stage, Stage::Mature);
        assert!(at(&s, 0.5).precip > 0.9 && at(&s, 0.5).anvil_r > 0.0 && at(&s, 0.5).overshoot > 0.0);
        assert_eq!(at(&s, 0.85).stage, Stage::Dissipating);
        assert!(at(&s, 0.85).precip < at(&s, 0.5).precip && at(&s, 0.85).anvil_r > 0.0);
    }

    #[test]
    fn motion() {
        for s in [cell(), line()] {
            let (n1, n2) = (at(&s, 0.4), at(&s, 0.6));
            let dt = 0.2 * s.life_s;
            assert!(((n2.a - n1.a) - s.vel * dt).length() < 1.0 && ((n2.b - n1.b) - s.vel * dt).length() < 1.0);
            assert!((n1.b - n1.a - s.half_axis * 2.0).length() < 1.0);
        }
    }

    #[test]
    fn thirty_day_count_matches_lambda() {
        let c = tcfg();
        let end = 30.0 * 86400.0 * c.clock_scale as f64; // 30 game days in storm seconds
        for g in [&g1 as &dyn Fn(Vec2) -> f32, &gx] {
            let n = births(&c, "seed-a", g, 0.0, end - 1.0, Vec2::ZERO).len() as f64;
            let (bin_s, mut lam) = (c.bin_min as f64 * 60.0, 0.0f64);
            let (nb, r) = ((c.window_m / c.bin_m).ceil() as i32 + 1, c.window_m);
            for tb in 0..(end / bin_s) as i64 { for iz in -nb..=nb { for ix in -nb..=nb {
                let m = Vec2::new((ix as f32 + 0.5) * c.bin_m, (iz as f32 + 0.5) * c.bin_m);
                if m.length() <= r { lam += lambda(&c, g(m), (tb as f64 + 0.5) * bin_s) as f64; }
            } } }
            assert!(lam > 100.0, "lambda {lam}");
            assert!((n - lam).abs() <= 3.0 * lam.sqrt(), "count {n} vs lambda {lam}");
        }
    }

    #[test]
    fn profile_envelope() {
        let s = cell(); let n = at(&s, 0.5);
        let c = Vec3::new(n.a.x, 0.0, n.a.y);
        let p = |dx: f32, y: f32, dz: f32| profile(&tcfg(), &n, Vec3::new(c.x + dx, y, c.z + dz));
        assert!(p(0.0, 8000.0, 0.0) > 0.9, "core");
        assert_eq!(p(0.0, n.top + n.overshoot + 1.0, 0.0), 0.0, "above top");
        assert_eq!(p(20000.0, n.top + 1.0, 0.0), 0.0, "above anvil");
        assert_eq!(p(0.0, n.shelf_low - 1.0, 0.0), 0.0);
        assert_eq!(p(0.0, n.shelf_low - 1.0, 9000.0), 0.0);
        // sheared anvil: flat top (cloud just under `top`, none just over), thinning at the far edge, a small overhang upwind
        let sd = n.anvil_shift.normalize();
        let (near, far) = (sd * 0.6 * n.anvil_r, sd * 0.95 * n.anvil_r);
        let yy = n.top - 0.1 * (n.top - n.anvil_base);
        assert!(p(near.x, n.top - 1.0, near.y) > 0.0 && p(near.x, n.top + 1.0, near.y) == 0.0, "flat anvil top");
        assert!(p(far.x, yy, far.y) < p(near.x, yy, near.y), "thins downwind");
        assert!(p(-sd.x * 0.95 * n.anvil_r, yy, -sd.y * 0.95 * n.anvil_r) == 0.0, "no anvil far upwind");
        assert!(p(sd.x * 2.0 * n.core_r, yy, sd.y * 2.0 * n.core_r) > 0.0, "anvil reaches >= 2 core radii downwind");
        // no neck: the cloud is at least as wide at 80% of the height as at 40% (core flares into the anvil)
        let wid = |f: f32| (0..400).filter(|i| p(*i as f32 * 100.0 - 20000.0, n.base + f * (n.top - n.base), 0.0) > 0.1).count();
        assert!(wid(0.8) >= wid(0.4), "tower narrows going up: {} < {}", wid(0.8), wid(0.4));
        // shelf only ahead of the axis (dir), between shelf_low and base
        let y = (n.shelf_low + n.base) * 0.5; let r = (n.core_r + n.gust_r) * 0.5;
        let d = n.dir * r;
        assert!(n.gust_r > n.core_r);
        assert!(p(d.x, y, d.y) > 0.0, "shelf ahead");
        assert_eq!(p(-d.x, y, -d.y), 0.0, "no shelf behind");
        assert_eq!(p(d.x, n.shelf_low - 1.0, d.y), 0.0);
        // wedge: the shelf floor is lower at the leading edge than near the core
        let (ne, fa) = (n.dir * (n.core_r + 0.2 * (n.gust_r - n.core_r)), n.dir * (n.core_r + 0.6 * (n.gust_r - n.core_r)));
        let yl = n.base - 0.5 * (n.base - n.shelf_low);
        assert!(p(fa.x, yl, fa.y) > p(ne.x, yl, ne.y), "shelf floor slopes up toward the storm");
    }

    #[test]
    fn pack_layout() {
        assert_eq!((PACK_VEC4, MAX_CELLS), (4, 16));
        let s = cell(); let n = at(&s, 0.5);
        let v = pack(&[(s.clone(), n.clone())]);
        assert_eq!(v.len(), PACK_VEC4);
        assert_eq!(v[0], [n.a.x, n.a.y, n.b.x, n.b.y]);
        assert_eq!(v[1], [n.core_r, n.base, n.top, n.overshoot]);
        assert_eq!(v[2], [n.anvil_r, n.anvil_shift.x, n.anvil_shift.y, n.anvil_base]);
        assert_eq!(v[3], [n.precip, n.gust_r, n.shelf_low, n.dir.y.atan2(n.dir.x)]);
        let many: Vec<_> = (0..20).map(|_| (s.clone(), n.clone())).collect();
        assert_eq!(pack(&many).len(), PACK_VEC4 * MAX_CELLS);
    }

    #[test]
    fn sky_scale_halves_sizes() {
        let (c1, mut c5) = (tcfg(), tcfg());
        c5.sky_scale = 0.5;
        let (a, b) = (births(&c1, "seed-a", &g1, 0.0, 2.0e5, Vec2::ZERO), births(&c5, "seed-a", &g1, 0.0, 2.0e5, Vec2::ZERO));
        assert!(a.len() > 5 && a.len() == b.len());
        for (x, y) in a.iter().zip(&b) {
            assert_eq!((x.id, x.born_s, x.origin, x.vel), (y.id, y.born_s, y.origin, y.vel));
            for (p, q) in [(x.base, y.base), (x.top, y.top), (x.core_r, y.core_r), (x.anvil_r, y.anvil_r), (x.half_axis.length(), y.half_axis.length())] {
                assert!((p * 0.5 - q).abs() <= 1e-3 * p.max(1.0), "{p} vs {q}");
            }
            let t = x.born_s + 0.5 * x.life_s as f64;
            let (nx, ny) = (now(x, t, &c1).unwrap(), now(y, t, &c5).unwrap());
            assert!((nx.top * 0.5 - ny.top).abs() < 1.0 && (nx.core_r * 0.5 - ny.core_r).abs() < 1.0 && (nx.anvil_r * 0.5 - ny.anvil_r).abs() < 1.0);
        }
        let mut ca = StormCfg::from_json(r#"{"authored": [{"name": "t", "kind": "cell", "bearing_deg": 270, "dist_m": 40000, "top_m": 14000}]}"#).unwrap();
        assert!(!ca.authored.is_empty());
        let a1 = authored(&ca, 0.0, Vec2::ZERO)[0].clone(); ca.sky_scale = 0.5;
        let a5 = authored(&ca, 0.0, Vec2::ZERO)[0].clone();
        assert_eq!((a1.top * 0.5, a1.core_r * 0.5, a1.anvil_r * 0.5), (a5.top, a5.core_r, a5.anvil_r));
        assert_eq!(a1.origin, a5.origin);
    }

    #[test]
    fn select_is_stable_topk() {
        let c = tcfg();
        let mut all: Vec<Storm> = (0..6).map(|i| Storm { id: i + 1, origin: Vec2::new(i as f32 * 3000.0 + 50000.0, 0.0), vel: Vec2::ZERO, ..cell() }).collect();
        all[5].origin.x = 50000.0 + 3000.0 * 2.6; // near-tie with id 3
        let t = cell().born_s + 1200.0;
        let a = select(&all, t, Vec2::ZERO, 3, &[], &c);
        assert_eq!(a.iter().map(|x| x.0.id).collect::<Vec<_>>(), vec![1, 2, 3]);
        let b = select(&all, t, Vec2::ZERO, 3, &[3], &c); // 3 was in prev: kept over the near-tie 6
        assert!(b.iter().any(|x| x.0.id == 3));
    }

    // The panel's Storm now must bring rain to the player before the cell dissipates.
    #[test]
    fn summoned_storm_rains_on_the_camera() {
        let c = StormCfg { enabled: true, ..StormCfg::default() };
        let cam = Vec2::new(300.0, -200.0);
        let s = summon(&c, 7, 1000.0, cam, Vec2::new(0.0, 5.0));
        let n = now(&s, 1000.0, &c).unwrap();
        assert_eq!(n.stage, Stage::Mature);
        assert!((n.a - (cam + Vec2::new(0.0, 1.0) * c.summon_dist_m)).length() < 1.0, "summoned at {:?}", n.a);
        let end = s.born_s + s.life_s as f64;
        let mut rained = false;
        let mut t = 1000.0;
        while t <= end {
            let Some(n) = now(&s, t, &c) else { break };
            if n.stage == Stage::Mature && crate::storm_fx::precip_at(&c, &n, Vec3::new(cam.x, 0.0, cam.y)) >= 0.5 * s.intensity {
                rained = true;
                break;
            }
            t += 10.0;
        }
        assert!(rained, "no rain over the camera while mature");
    }
}
