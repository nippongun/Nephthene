//! Storm fronts, the consumers: precip curtain, outflow, sun occlusion, flash and thunder schedule.
//! No bevy. Ground plane is (x, z); altitude is metres. Constants: [`crate::storm::StormFx`], all untuned.
use glam::{Vec2, Vec3};
use crate::rain::{hash01, pcg, smoothstep};
use crate::storm::{now, profile, Now, Stage, Storm, StormCfg};


fn nearest(a: Vec2, b: Vec2, p: Vec2) -> Vec2 {
    let ab = b - a; let l2 = ab.length_squared();
    if l2 < 1e-6 { return a; }
    a + ab * ((p - a).dot(ab) / l2).clamp(0.0, 1.0)
}

/// Rain curtain under a cell or line: full strength inside the core, zero outside it, rain falls from the base to the ground.
pub fn precip_at(c: &StormCfg, n: &Now, p: Vec3) -> f32 {
    if n.precip <= 0.0 || n.core_r < 1.0 || p.y < 0.0 || p.y > n.base { return 0.0; }
    let g = Vec2::new(p.x, p.z);
    let d = g.distance(nearest(n.a, n.b, g));
    n.precip * (1.0 - smoothstep(c.fx.curtain_full * n.core_r, n.core_r, d))
}

/// Ground-plane outflow at p (x, z): radially away from the downdraft (the axis), a density current that peaks
/// behind the gust front and is zero at the front.
pub fn outflow_at(_c: &StormCfg, n: &Now, p: Vec3) -> Vec2 {
    if n.outflow_mps <= 0.0 || n.core_r < 1.0 || n.gust_r <= n.core_r { return Vec2::ZERO; }
    let g = Vec2::new(p.x, p.z);
    let off = g - nearest(n.a, n.b, g);
    let d = off.length();
    if d < 1e-3 { return Vec2::ZERO; }
    let w = n.gust_r - n.core_r;
    let shape = smoothstep(n.core_r, n.core_r + 0.5 * w, d) * (1.0 - smoothstep(n.gust_r - 0.2 * w, n.gust_r, d));
    off * (n.outflow_mps * shape / d)
}

/// Optical depth along a ray of length `len_m` from p in direction `dir` (unit). Non-decreasing in `len_m`.
pub fn od_along(c: &StormCfg, n: &Now, p: Vec3, dir: Vec3, len_m: f32) -> f32 {
    let (step, ext) = (c.fx.od_step_m, c.fx.od_ext_per_m);
    let steps = (len_m.max(0.0) / step).floor() as i32;
    let mut sum = 0.0f32;
    for i in 0..steps {
        sum += profile(c, n, p + dir * ((i as f32 + 0.5) * step));
    }
    sum * step * ext
}

/// Sun optical depth at p toward `sun` (unit, pointing at the sun).
pub fn sun_od(c: &StormCfg, n: &Now, p: Vec3, sun: Vec3) -> f32 { od_along(c, n, p, sun, c.fx.sun_path_m) }

/// One lightning flash in storm time.
#[derive(Clone, Debug, PartialEq)]
pub struct Flash { pub t_s: f64, pub cg: bool, pub energy: f32 }

/// Flash schedule in [t0, t1): a Bernoulli-per-bin Poisson process on absolute storm-time bins, so the same storm gives
/// the same flashes whatever window asks, and only mature storms flash.
pub fn flashes(s: &Storm, c: &StormCfg, t0: f64, t1: f64) -> Vec<Flash> {
    let mut out = Vec::new();
    if t1 <= t0 { return out; }
    let dt = c.fx.flash_dt_s;
    let base = (s.id as u32) ^ ((s.id >> 32) as u32) ^ c.fx.flash_salt;
    let p_bin = (c.fx.flash_rate_per_s * dt) as f32;
    for k in (t0 / dt).floor() as i64..(t1 / dt).ceil() as i64 {
        let h = pcg(base ^ (k as u32).wrapping_mul(0x9E37_79B9));
        let mature = now(s, (k as f64 + 0.5) * dt, c).is_some_and(|n| n.stage == Stage::Mature);
        if !mature || hash01(h) >= p_bin { continue; }
        let t = (k as f64 + hash01(h ^ 1) as f64) * dt;
        if t < t0 || t >= t1 { continue; }
        out.push(Flash { t_s: t, cg: hash01(h ^ 2) < c.fx.cg_frac, energy: hash01(h ^ 3) });
    }
    out
}

/// Thunder delay for a flash `d_m` metres away (game metres).
pub fn thunder_delay_s(c: &StormCfg, d_m: f32) -> f32 { d_m / c.fx.sound_mps }


/// Optical depth straight up from p, through the cell above it. Zero above the cell and outside it.
pub fn column_od(c: &StormCfg, n: &Now, p: Vec3) -> f32 { od_along(c, n, p, Vec3::Y, c.fx.column_m) }

/// Ground light at a ground point: sun transmittance x sky AO (the ambient term of §5.4).
/// Open sky gives 1; a point under a cell is shadowed by its column.
pub fn ground_light(c: &StormCfg, n: &Now, p: Vec3, sun: Vec3) -> f32 {
    (-sun_od(c, n, p, sun)).exp() * (-c.fx.amb_k * column_od(c, n, p)).exp()
}

/// What the storms do at one ground point: their rain (curtain), the cloud cover over it, the gust-front wind (m/s, x z) and the
/// sky light left under their columns (1 = open sky). The weather here takes the wetter, cloudier of the map and this.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StormLocal { pub precip: f32, pub cover: f32, pub outflow: Vec2, pub ambient: f32 }
impl Default for StormLocal { fn default() -> Self { Self { precip: 0.0, cover: 0.0, outflow: Vec2::ZERO, ambient: 1.0 } } }

/// [`StormLocal`] at p from the selected storms: rain and cover are the strongest storm's, gusts add, sky light multiplies.
/// Nothing when `couple` is off.
pub fn local(c: &StormCfg, sel: &[(Storm, Now)], p: Vec3) -> StormLocal {
    if !c.couple { return StormLocal::default(); }
    sel.iter().fold(StormLocal::default(), |l, (_, n)| {
        let od = column_od(c, n, p);
        StormLocal { precip: l.precip.max(precip_at(c, n, p)), cover: l.cover.max(1.0 - (-c.fx.cover_k * od).exp()),
                     outflow: l.outflow + outflow_at(c, n, p), ambient: l.ambient * (-c.fx.amb_k * od).exp() }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tcfg() -> StormCfg { StormCfg { enabled: true, ..StormCfg::default() } }
    use crate::storm::Kind;

    /// A mature cell at the origin, moving +x, with an explicit gust front.
    fn mature(core_r: f32, gust_r: f32, precip: f32, outflow: f32) -> Now {
        Now { a: Vec2::ZERO, b: Vec2::ZERO, dir: Vec2::X, stage: Stage::Mature, age: 1000.0, base: 2800.0,
              core_r, top: 12000.0, overshoot: 0.0, anvil_r: 0.0, anvil_shift: Vec2::ZERO, anvil_base: 11000.0,
              precip, gust_r, shelf_low: 2800.0, outflow_mps: outflow }
    }
    fn cell(life_s: f32) -> Storm {
        Storm { id: 0x1234_5678_9abc, kind: Kind::Cell, born_s: 0.0, life_s, origin: Vec2::ZERO, vel: Vec2::ZERO,
                half_axis: Vec2::ZERO, core_r: 6000.0, base: 2800.0, top: 12000.0, anvil_r: 0.0, intensity: 1.0 }
    }

    #[test]
    fn precip_curtain() {
        let n = mature(6000.0, 20000.0, 5.0, 20.0);
        let at = |x: f32, y: f32| precip_at(&tcfg(), &n, Vec3::new(x, y, 0.0));
        assert!(at(0.0, 0.0) >= 0.8 * n.precip, "core {}", at(0.0, 0.0));
        assert!(at(0.6 * 6000.0 * 0.9, 0.0) >= 0.8 * n.precip, "inner core");
        assert_eq!(at(1.5 * 6000.0, 0.0), 0.0, "outside curtain");
        assert_eq!(at(0.0, 3000.0), 0.0, "above base: no curtain");
        assert_eq!(at(0.0, -1.0), 0.0, "below ground");
    }

    #[test]
    fn outflow_away_and_peaks_behind_front() {
        let n = mature(6000.0, 20000.0, 5.0, 20.0);
        let mut best = (0.0f32, 0.0f32);
        for i in 0..=240 {
            let d = i as f32 * 100.0; // 0 .. 24 km along +x
            let p = Vec3::new(d, 0.0, 0.0);
            let o = outflow_at(&tcfg(), &n, p);
            assert!(o.dot(Vec2::new(d, 0.0)) >= 0.0, "not away from downdraft at d={d}");
            if o.length() > best.0 { best = (o.length(), d); }
        }
        assert!(best.0 > 0.0, "no outflow at all");
        assert!(best.1 >= 0.6 * 20000.0 && best.1 < 20000.0, "peak at {} not behind front", best.1);
        assert_eq!(outflow_at(&tcfg(), &n, Vec3::new(22000.0, 0.0, 0.0)), Vec2::ZERO, "ahead of front");
    }

    #[test]
    fn sun_od_monotonic_in_path() {
        let n = mature(6000.0, 20000.0, 5.0, 20.0);
        let sun = Vec3::new(0.0, 1.0, 0.0);
        let inside = Vec3::new(0.0, 500.0, 0.0);
        let mut prev = 0.0f32;
        for i in 0..=40 {
            let od = od_along(&tcfg(), &n, inside, sun, i as f32 * 1000.0);
            assert!(od >= prev, "od fell at {} m: {od} < {prev}", i * 1000);
            prev = od;
        }
        assert!(prev > 0.0, "a ray up through the cell has no optical depth");
        assert_eq!(sun_od(&tcfg(), &n, Vec3::new(50000.0, 500.0, 0.0), sun), 0.0, "far from the cell");
        assert!(sun_od(&tcfg(), &n, inside, sun) > 0.0);
    }

    #[test]
    fn flash_schedule_deterministic_and_rate() {
        let life = 100_000.0f32;
        let s = cell(life);
        let c = tcfg();
        let a = flashes(&s, &c, 0.0, life as f64);
        assert_eq!(a, flashes(&s, &c, 0.0, life as f64), "not deterministic");
        let split = (life as f64) * 0.37;
        let mut b = flashes(&s, &c, 0.0, split);
        b.extend(flashes(&s, &c, split, life as f64));
        assert_eq!(a, b, "window split changes the schedule");
        // Poisson, mature stage only: expected = rate x mature duration (DEV_END..DISS_START = 0.4 of life).
        let lambda = 0.4 * life as f64 / 30.0;
        let sigma = lambda.sqrt();
        let n = a.len() as f64;
        assert!((n - lambda).abs() <= 3.0 * sigma, "count {n} vs expected {lambda} (3 sigma = {})", 3.0 * sigma);
        for f in &a { assert!(f.t_s >= 0.3 * life as f64 && f.t_s < 0.7 * life as f64, "flash outside mature at {}", f.t_s); }
    }

    #[test]
    fn local_weather_under_a_storm() {
        let s = cell(100_000.0);
        let n = mature(6000.0, 20000.0, 0.8, 20.0);
        let sel = vec![(s, n.clone())];
        let at = |c: &StormCfg, x: f32| local(c, &sel, Vec3::new(x, 0.0, 0.0));
        let under = at(&tcfg(), 0.0);
        assert!((under.precip - n.precip).abs() < 1e-6, "rain under the core {} vs the cell's {}", under.precip, n.precip);
        assert!(under.cover > 0.9, "cover under the core {}", under.cover);
        assert!(under.ambient < 0.6, "sky light under the core {}", under.ambient);
        let front = at(&tcfg(), 15000.0);
        assert_eq!(front.precip, 0.0, "rain outside the core");
        assert!(front.outflow.x > 1.0, "no gust behind the front: {:?}", front.outflow);
        assert_eq!(at(&tcfg(), 60000.0), StormLocal::default(), "far from the storm");
        // Red arm: decoupled storms leave the weather here alone.
        assert_eq!(at(&StormCfg { couple: false, ..tcfg() }, 0.0), StormLocal::default());
    }

    #[test]
    fn thunder_delay_is_distance_over_343() {
        assert_eq!(thunder_delay_s(&tcfg(), 3430.0), 10.0);
        for d in [1.0f32, 812.5, 5000.0, 17150.0] { assert_eq!(thunder_delay_s(&tcfg(), d), d / 343.0); }
    }

    #[test]
    fn column_and_ground_light_under_cell() {
        let n = mature(6000.0, 20000.0, 5.0, 20.0);
        let mut prev = f32::MAX;
        for i in 0..=40 {
            let y = 2800.0 + i as f32 * 230.0; // base .. ~12 km
            let c = column_od(&tcfg(), &n, Vec3::new(0.0, y, 0.0));
            assert!(c > 0.0 || y >= 12000.0, "no column above core at y={y}");
            assert!(c <= prev + 1e-3, "column rose going up at y={y}: {c} > {prev}");
            prev = c;
        }
        assert_eq!(column_od(&tcfg(), &n, Vec3::new(0.0, 13000.0, 0.0)), 0.0, "above the top");
        assert_eq!(column_od(&tcfg(), &n, Vec3::new(30000.0, 5000.0, 0.0)), 0.0, "outside the cell");
        let under = ground_light(&tcfg(), &n, Vec3::new(0.0, 0.0, 0.0), Vec3::Y);
        let open = ground_light(&tcfg(), &n, Vec3::new(30000.0, 0.0, 0.0), Vec3::Y);
        assert!(open >= 0.99, "open sky not 1: {open}");
        assert!(under <= 0.3 * open, "under cell {under} > 0.3 x open {open}");
        assert!(under > 0.0, "under cell fully black, not a gradient");
    }
}
