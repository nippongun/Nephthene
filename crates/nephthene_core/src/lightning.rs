//! Lightning, the pure half: where a scheduled flash ([`crate::storm_fx::flashes`]) strikes, the shape of its channel, its
//! brightness over time (return strokes and the glow after them) and how far its thunder travels. No bevy. Positions are in the
//! storm frame (map metres, the frame of [`Now`]); altitude is metres. Everything is seeded from the storm id and the flash time,
//! so the same storm and clock give the same bolt on every machine. Constants: [`Lightning`] (`storms.lightning`), all untuned.
//! Channel shape: recursive midpoint displacement (the usual real-time bolt; Reed & Wyvill 1994 describe the jagged branching
//! look), self-similar because each level displaces by a fixed fraction of its own segment length.
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use crate::rain::{hash01, pcg};
use crate::storm::{Now, Storm};
use crate::storm_fx::Flash;

/// GPU layout: at most this many channel segments per bolt (the bolt uniform holds 2 vec4 per segment).
pub const MAX_SEGMENTS: usize = 256;

/// The `storms.lightning` section.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Lightning {
    /// Off: no flashes are drawn, lit or reported (the flash schedule itself is unchanged).
    pub enabled: bool,
    /// Cloud-to-ground: the strike point lies within this fraction of the core radius of the storm axis.
    pub strike_r_frac: f32,
    /// In-cloud: altitude band of the channel as fractions of the cell top, and its horizontal length range (m).
    pub ic_alt_frac: [f32; 2], pub ic_len_m: [f32; 2],
    /// Main channel: midpoint-displacement levels (2^levels segments) and the sideways displacement per level as a fraction of the
    /// segment length. A cloud-to-ground channel also slants: its top is up to `displace` x its height away from the strike point.
    pub levels: u32, pub displace: f32,
    /// Branches: chance per main-channel point, length as a fraction of the channel left below that point, levels, brightness
    /// relative to the main channel, and how far a branch turns sideways (fraction of its length).
    pub branch_p: f32, pub branch_len: f32, pub branch_levels: u32, pub branch_bright: f32, pub branch_spread: f32,
    /// Return strokes per flash, the gap between strokes (s), each stroke's e-folding decay (s), the glow after the first stroke
    /// (level relative to a stroke, e-folding decay s), and the flash life (s) after which it is dropped.
    pub strokes: [u32; 2], pub stroke_gap_s: [f32; 2], pub stroke_decay_s: f32, pub glow: f32, pub glow_decay_s: f32, pub life_s: f32,
    /// Channel look: width (m, the luminous core and corona), at least `min_px` pixels wide (a wider ribbon is dimmer, so the
    /// light it sends stays the same), radiance at energy 1 (cd/m^2 before exposure) and colour.
    pub width_m: f32, pub min_px: f32, pub radiance: f32, pub color: [f32; 3],
    /// The channel above the cloud base is inside the cloud: it fades out over this height (m). In-cloud flashes draw no channel.
    pub base_fade_m: f32,
    /// Flash light: lumens at energy 1 and full brightness, range (m), and where it sits along the channel (0 bottom, 1 top).
    pub light_lm: f32, pub light_range_m: f32, pub light_at: f32,
    /// Most flashes alive at once (each has a light; the newest cloud-to-ground one has the drawn channel).
    pub max_live: u32,
    /// A storm-clock step longer than this (s) is a jump (start, panel clock change): its flashes are skipped, not all fired at once.
    pub jump_s: f64,
    /// Test switch: hold a cloud-to-ground flash of the first selected storm at this age (s) every frame, so a frozen-clock shot
    /// sees it. None = the schedule.
    pub force_age_s: Option<f32>,
}
impl Default for Lightning {
    fn default() -> Self {
        Lightning { enabled: true, strike_r_frac: 0.6, ic_alt_frac: [0.35, 0.8], ic_len_m: [3000.0, 15000.0], levels: 7, displace: 0.22,
                    branch_p: 0.3, branch_len: 0.4, branch_levels: 4, branch_bright: 0.35, branch_spread: 0.6,
                    strokes: [2, 5], stroke_gap_s: [0.03, 0.1], stroke_decay_s: 0.02, glow: 0.25, glow_decay_s: 0.12, life_s: 1.0,
                    width_m: 3.0, min_px: 1.5, radiance: 1.0e6, color: [0.85, 0.9, 1.0], base_fade_m: 400.0,
                    light_lm: 1.0e10, light_range_m: 30000.0, light_at: 0.3, max_live: 2, jump_s: 5.0, force_age_s: None }
    }
}

/// Where one flash goes: a straight line from `top` to `end` (the channel wanders around it), cloud-to-ground or in-cloud.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strike { pub top: Vec3, pub end: Vec3, pub cg: bool, pub energy: f32, pub seed: u32 }

/// One straight piece of the channel and its brightness (main channel 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seg { pub a: Vec3, pub b: Vec3, pub bright: f32 }

/// Deterministic uniform stream on the rain.rs PCG.
struct Rng(u32);
impl Rng {
    fn next(&mut self) -> f32 { self.0 = pcg(self.0.wrapping_add(0x9E37_79B9)); hash01(self.0) }
    fn range(&mut self, r: [f32; 2]) -> f32 { r[0] + (r[1] - r[0]) * self.next() }
    fn unit2(&mut self) -> Vec2 { let a = self.next() * std::f32::consts::TAU; Vec2::new(a.cos(), a.sin()) }
}

/// The seed of a flash: its storm and its time (bit-exact, so the same flash always gets the same bolt).
pub fn seed(s: &Storm, f: &Flash) -> u32 {
    let t = f.t_s.to_bits();
    pcg(pcg((s.id as u32) ^ ((s.id >> 32) as u32)) ^ (t as u32) ^ ((t >> 32) as u32).wrapping_mul(0x85EB_CA6B))
}

/// Where the flash `f` of storm `s` (state `n`) strikes. `ground(xz)` is the ground height in the storm frame.
pub fn strike(c: &Lightning, s: &Storm, n: &Now, f: &Flash, ground: &dyn Fn(Vec2) -> f32) -> Strike {
    let seed = seed(s, f);
    let mut r = Rng(seed);
    // A point on the axis (a line storm's axis is a segment), then out from it within the core.
    let axis = n.a + (n.b - n.a) * r.next();
    if f.cg {
        let g = axis + r.unit2() * c.strike_r_frac * n.core_r * r.next().sqrt();
        let gy = ground(g);
        let h = (n.base - gy).max(0.0);
        let t = g + r.unit2() * c.displace * h * r.next();
        Strike { top: Vec3::new(t.x, n.base.max(gy), t.y), end: Vec3::new(g.x, gy, g.y), cg: true, energy: f.energy, seed }
    } else {
        let a = axis + r.unit2() * n.core_r * r.next().sqrt();
        let b = a + r.unit2() * r.range(c.ic_len_m);
        let (y0, y1) = (r.range(c.ic_alt_frac) * n.top, r.range(c.ic_alt_frac) * n.top);
        Strike { top: Vec3::new(a.x, y0, a.y), end: Vec3::new(b.x, y1, b.y), cg: false, energy: f.energy, seed }
    }
}

/// Two unit vectors perpendicular to `d` (unit) and to each other.
fn basis(d: Vec3) -> (Vec3, Vec3) {
    let u = if d.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let s = d.cross(u).normalize();
    (s, d.cross(s))
}

/// Points of a midpoint-displaced line from `a` to `b` (both kept), `levels` deep; no point below `floor`.
fn displace(r: &mut Rng, a: Vec3, b: Vec3, levels: u32, frac: f32, floor: f32) -> Vec<Vec3> {
    let mut pts = vec![a, b];
    for _ in 0..levels {
        let mut next = Vec::with_capacity(pts.len() * 2 - 1);
        for w in pts.windows(2) {
            let (p, q) = (w[0], w[1]);
            let len = p.distance(q);
            let (s, t) = basis((q - p).normalize_or(Vec3::Y));
            let off = (s * (2.0 * r.next() - 1.0) + t * (2.0 * r.next() - 1.0)) * frac * len;
            let mut m = 0.5 * (p + q) + off;
            m.y = m.y.max(floor);
            next.extend([p, m]);
        }
        next.push(*pts.last().unwrap());
        pts = next;
    }
    pts
}

/// The channel of strike `k`: the main channel from top to end, then branches off it while the segment budget lasts.
/// At most [`MAX_SEGMENTS`] segments; a cloud-to-ground channel never dips below its strike point's height.
pub fn bolt(c: &Lightning, k: &Strike) -> Vec<Seg> {
    let mut r = Rng(pcg(k.seed ^ 0xB017));
    let levels = c.levels.min(MAX_SEGMENTS.ilog2());
    let floor = if k.cg { k.end.y } else { f32::MIN };
    let main = displace(&mut r, k.top, k.end, levels, c.displace, floor);
    let mut out: Vec<Seg> = main.windows(2).map(|w| Seg { a: w[0], b: w[1], bright: 1.0 }).collect();
    let per_branch = 1usize << c.branch_levels.min(levels);
    for &p in main.iter().skip(1).take(main.len().saturating_sub(2)) {
        if r.next() >= c.branch_p { continue; }
        if out.len() + per_branch > MAX_SEGMENTS { break; }
        let rest = *main.last().unwrap() - p;
        let (s, t) = basis(rest.normalize_or(Vec3::NEG_Y));
        let side = (s * (2.0 * r.next() - 1.0) + t * (2.0 * r.next() - 1.0)).normalize_or(s);
        let len = c.branch_len * rest.length();
        let q = p + rest.normalize_or(Vec3::NEG_Y) * len + side * c.branch_spread * len;
        let pts = displace(&mut r, p, Vec3::new(q.x, q.y.max(floor), q.z), c.branch_levels.min(levels), c.displace, floor);
        out.extend(pts.windows(2).map(|w| Seg { a: w[0], b: w[1], bright: c.branch_bright }));
    }
    out
}

/// Start times (s after the flash) of the return strokes of the flash with this seed; the first is at 0.
pub fn stroke_times(c: &Lightning, seed: u32) -> Vec<f32> {
    let mut r = Rng(pcg(seed ^ 0x57_0CE));
    let (lo, hi) = (c.strokes[0].min(c.strokes[1]).max(1), c.strokes[0].max(c.strokes[1]).max(1));
    let n = (lo + ((hi - lo + 1) as f32 * r.next()) as u32).min(hi);
    let mut t = 0.0;
    (0..n).map(|i| { if i > 0 { t += r.range(c.stroke_gap_s); } t }).collect()
}

/// Brightness 0..1 of a flash `age_s` seconds old: the brightest live return stroke (each decays from 1), or the glow after the
/// first stroke if that is brighter. 0 before the flash and from `life_s` on.
pub fn envelope(c: &Lightning, seed: u32, age_s: f32) -> f32 {
    if !(0.0..c.life_s).contains(&age_s) { return 0.0; }
    let stroke = stroke_times(c, seed).into_iter().filter(|&t| t <= age_s).map(|t| (-(age_s - t) / c.stroke_decay_s).exp()).fold(0.0, f32::max);
    stroke.max(c.glow * (-age_s / c.glow_decay_s).exp())
}

/// Distance (m) from `p` to the straight line of the strike, the path the thunder takes first.
pub fn distance_m(k: &Strike, p: Vec3) -> f32 {
    let ab = k.end - k.top;
    // ponytail: 1e-6 m^2 only guards a zero-length strike against a division by zero.
    let t = if ab.length_squared() < 1e-6 { 0.0 } else { ((p - k.top).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) };
    p.distance(k.top + ab * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storm::{Kind, Stage};

    fn cell() -> (Storm, Now) {
        let s = Storm { id: 0x1234_5678_9abc, kind: Kind::Cell, born_s: 0.0, life_s: 3600.0, origin: Vec2::ZERO, vel: Vec2::ZERO,
                        half_axis: Vec2::ZERO, core_r: 6000.0, base: 2800.0, top: 12000.0, anvil_r: 0.0, intensity: 1.0 };
        let n = Now { a: Vec2::new(500.0, -300.0), b: Vec2::new(500.0, -300.0), dir: Vec2::X, stage: Stage::Mature, age: 1000.0, base: 2800.0,
                      core_r: 6000.0, top: 12000.0, overshoot: 0.0, anvil_r: 0.0, anvil_shift: Vec2::ZERO, anvil_base: 11000.0,
                      precip: 5.0, gust_r: 20000.0, shelf_low: 2800.0, outflow_mps: 20.0 };
        (s, n)
    }
    fn flash(i: u32, cg: bool) -> Flash { Flash { t_s: 1000.0 + i as f64 * 7.3, cg, energy: 0.8 } }
    // Ground rising to the east: a channel must stop on the ground under it, not at y = 0.
    fn ground(p: Vec2) -> f32 { 0.02 * p.x + 100.0 }

    #[test]
    fn same_flash_same_bolt() {
        let (s, n) = cell();
        let c = Lightning::default();
        let (a, b) = (strike(&c, &s, &n, &flash(0, true), &ground), strike(&c, &s, &n, &flash(0, true), &ground));
        assert_eq!(a, b, "strike not deterministic");
        assert_eq!(bolt(&c, &a), bolt(&c, &b), "bolt not deterministic");
        assert_ne!(bolt(&c, &a), bolt(&c, &strike(&c, &s, &n, &flash(1, true), &ground)), "two flashes share a bolt");
    }

    #[test]
    fn cloud_to_ground_runs_from_the_base_to_the_ground_inside_the_core() {
        let (s, n) = cell();
        let c = Lightning::default();
        for i in 0..200 {
            let k = strike(&c, &s, &n, &flash(i, true), &ground);
            let g = Vec2::new(k.end.x, k.end.z);
            assert!(g.distance(n.a) <= c.strike_r_frac * n.core_r + 1e-2, "strike {i} {:.0} m off the axis", g.distance(n.a));
            assert!((k.end.y - ground(g)).abs() < 1e-3, "strike {i} ends at y {} not on the ground {}", k.end.y, ground(g));
            assert_eq!(k.top.y, n.base, "strike {i} does not start at the cloud base");
            let b = bolt(&c, &k);
            assert!(b.len() <= MAX_SEGMENTS, "{} segments", b.len());
            let main: Vec<_> = b.iter().filter(|x| x.bright == 1.0).collect();
            assert_eq!(main.len(), 1 << c.levels, "main channel segments");
            assert_eq!((main[0].a, main.last().unwrap().b), (k.top, k.end), "main channel does not join top and ground");
            for w in main.windows(2) { assert_eq!(w[0].b, w[1].a, "main channel broken"); }
            for x in &b { assert!(x.a.y >= k.end.y - 1e-3 && x.b.y >= k.end.y - 1e-3, "strike {i} channel below its ground point"); }
            // Every branch starts on the channel drawn before it.
            for (j, x) in b.iter().enumerate().filter(|(_, x)| x.bright != 1.0) {
                assert!(b[..j].iter().any(|y| y.b == x.a || y.a == x.a), "strike {i} branch {j} floats free");
            }
        }
    }

    #[test]
    fn in_cloud_stays_in_its_altitude_band() {
        let (s, n) = cell();
        let c = Lightning::default();
        for i in 0..200 {
            let k = strike(&c, &s, &n, &flash(i, false), &ground);
            for y in [k.top.y, k.end.y] {
                assert!(y >= c.ic_alt_frac[0] * n.top && y <= c.ic_alt_frac[1] * n.top, "in-cloud end at {y} m");
            }
            let len = Vec2::new(k.end.x - k.top.x, k.end.z - k.top.z).length();
            assert!(len >= c.ic_len_m[0] - 1e-2 && len <= c.ic_len_m[1] + 1e-2, "in-cloud length {len}");
        }
    }

    #[test]
    fn branches_stop_at_the_segment_budget() {
        let (s, n) = cell();
        let c = Lightning { levels: 20, branch_p: 1.0, ..Lightning::default() };
        let b = bolt(&c, &strike(&c, &s, &n, &flash(3, true), &ground));
        assert!(b.len() <= MAX_SEGMENTS, "{} segments > {MAX_SEGMENTS}", b.len());
        assert_eq!(b.iter().filter(|x| x.bright == 1.0).count(), MAX_SEGMENTS, "levels are capped at the budget");
    }

    #[test]
    fn envelope_has_one_peak_per_stroke_and_ends() {
        let c = Lightning::default();
        for seed in 0..50u32 {
            let t = stroke_times(&c, seed);
            assert!(t.len() as u32 >= c.strokes[0] && t.len() as u32 <= c.strokes[1], "{} strokes", t.len());
            assert_eq!(t[0], 0.0);
            for w in t.windows(2) { let g = w[1] - w[0]; assert!(g >= c.stroke_gap_s[0] && g <= c.stroke_gap_s[1], "gap {g}"); }
            assert_eq!(envelope(&c, seed, -0.001), 0.0, "light before the flash");
            assert_eq!(envelope(&c, seed, c.life_s), 0.0, "light after the flash's life");
            // Each stroke starts at full brightness and is below half one half-life later (no later stroke starts that soon: the
            // shortest gap is longer than a half-life at the defaults; the glow stays under half).
            let half = c.stroke_decay_s * 2.0f32.ln() * 1.01;
            assert!(half < c.stroke_gap_s[0] && c.glow < 0.5, "test premise: gaps longer than a half-life, glow under half");
            for &ts in &t {
                assert!((envelope(&c, seed, ts) - 1.0).abs() < 1e-6, "stroke at {ts} not at full brightness");
                assert!(envelope(&c, seed, ts + half) < 0.5, "stroke at {ts} does not fade");
            }
        }
    }

    #[test]
    fn thunder_distance_is_to_the_nearest_point_of_the_channel() {
        let k = Strike { top: Vec3::new(0.0, 3000.0, 0.0), end: Vec3::ZERO, cg: true, energy: 1.0, seed: 0 };
        assert_eq!(distance_m(&k, Vec3::new(4000.0, 1500.0, 0.0)), 4000.0);
        assert_eq!(distance_m(&k, Vec3::new(0.0, -100.0, 0.0)), 100.0);
        assert_eq!(distance_m(&k, Vec3::new(0.0, 3500.0, 0.0)), 500.0);
    }
}
