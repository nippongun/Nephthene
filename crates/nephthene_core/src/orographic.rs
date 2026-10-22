//! Precipitation and cloud from terrain and wind: Smith & Barstad (2004) linear orographic precipitation, one FFT pass.
//! Pure, no engine. Grid axes: column index = +u, row index = +v; `wind` is (u, v) m/s in those axes, the direction the air moves.
//! Result for one wavenumber (k, l), sigma = U k + V l:
//!   P^ = Cw * i sigma h^ / ((1 - i m Hw)(1 + i sigma tau_c)(1 + i sigma tau_f)),  m^2 = (N^2 - sigma^2)(k^2 + l^2) / sigma^2
//! (m imaginary when sigma^2 > N^2). Precip is the real part of the inverse transform, clipped at 0.
//! All constants in `OroParams::default()` are untuned.

use crate::{Local, air::vapour_hpa, config::OroCfg, rain::Ground, texel_axis};

#[derive(Clone, Copy, Debug)]
pub struct OroParams {
    /// Cell size, m.
    pub cell_m: f32,
    /// Air motion (u, v), m/s.
    pub wind: [f32; 2],
    /// Moisture gain Cw: output units per (m/s of forced uplift).
    pub gain: f32,
    /// Moist stability N_m, 1/s.
    pub n_moist: f32,
    /// Moisture depth scale height Hw, m.
    pub h_w: f32,
    /// Cloud-water conversion and hydrometeor fallout times, s.
    pub tau_c: f32,
    pub tau_f: f32,
}
impl Default for OroParams {
    // untuned
    fn default() -> Self { Self { cell_m: 16.0, wind: [10.0, 0.0], gain: 1.0, n_moist: 0.005, h_w: 2500.0, tau_c: 8.0, tau_f: 8.0 } }
}

/// Precip and cloud-water fields, same layout as the input heights. Both >= 0.
pub struct OroOut { pub precip: Vec<f32>, pub cloud: Vec<f32> }

type C = [f64; 2];
fn mul(a: C, b: C) -> C { [a[0] * b[0] - a[1] * b[1], a[0] * b[1] + a[1] * b[0]] }
fn div(a: C, b: C) -> C { let d = b[0] * b[0] + b[1] * b[1]; let n = mul(a, [b[0], -b[1]]); [n[0] / d, n[1] / d] }

/// In-place radix-2 FFT (len a power of two); `inv` conjugates the twiddles (unscaled).
fn fft(a: &mut [C], inv: bool) {
    let n = a.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 { j ^= bit; bit >>= 1; }
        j |= bit;
        if i < j { a.swap(i, j); }
    }
    let mut len = 2;
    while len <= n {
        let ang = std::f64::consts::TAU / len as f64 * if inv { 1.0 } else { -1.0 };
        let w = [ang.cos(), ang.sin()];
        for s in (0..n).step_by(len) {
            let mut t = [1.0, 0.0];
            for k in 0..len / 2 {
                let (u, v) = (a[s + k], mul(a[s + k + len / 2], t));
                a[s + k] = [u[0] + v[0], u[1] + v[1]];
                a[s + k + len / 2] = [u[0] - v[0], u[1] - v[1]];
                t = mul(t, w);
            }
        }
        len <<= 1;
    }
}

fn fft2(a: &mut [C], n: usize, inv: bool) {
    for r in 0..n { fft(&mut a[r * n..(r + 1) * n], inv); }
    let mut col = vec![[0.0; 2]; n];
    for c in 0..n {
        for r in 0..n { col[r] = a[r * n + c]; }
        fft(&mut col, inv);
        for r in 0..n { a[r * n + c] = col[r]; }
    }
}

/// `h` row-major `w` x `h_n` heights in metres. Zero-padded (after removing the mean) to a power-of-two square >= 2x the larger side, so the
/// periodic FFT does not wrap rain across the map.
pub fn compute(h: &[f32], w: usize, hn: usize, p: &OroParams) -> OroOut {
    assert_eq!(h.len(), w * hn);
    let n = (2 * w.max(hn)).next_power_of_two();
    let mean = h.iter().sum::<f32>() / h.len() as f32;
    let mut a = vec![[0.0f64; 2]; n * n];
    for y in 0..hn { for x in 0..w { a[y * n + x][0] = (h[y * w + x] - mean) as f64; } }
    fft2(&mut a, n, false);
    let (u, v, nm, hw) = (p.wind[0] as f64, p.wind[1] as f64, p.n_moist as f64, p.h_w as f64);
    let (tc, tf, gain) = (p.tau_c as f64, p.tau_f as f64, p.gain as f64);
    let dk = std::f64::consts::TAU / (n as f64 * p.cell_m as f64);
    let freq = |i: usize| if i <= n / 2 { i as f64 } else { i as f64 - n as f64 } * dk;
    let mut cloud = a.clone();
    for r in 0..n { for c in 0..n {
        let (k, l) = (freq(c), freq(r));
        let sigma = u * k + v * l;
        let i = r * n + c;
        let kk = k * k + l * l;
        if sigma.abs() < 1e-12 || kk == 0.0 { a[i] = [0.0; 2]; cloud[i] = [0.0; 2]; continue; }
        // 1 - i m Hw
        let d1 = if sigma * sigma < nm * nm {
            let m = sigma.signum() * ((nm * nm - sigma * sigma) * kk / (sigma * sigma)).sqrt();
            [1.0, -m * hw]
        } else {
            let s = ((sigma * sigma - nm * nm) * kk / (sigma * sigma)).sqrt();
            [1.0 + s * hw, 0.0]
        };
        let num = mul([0.0, gain * sigma], a[i]);
        let base = div(num, mul(d1, [1.0, sigma * tc]));
        cloud[i] = base;
        a[i] = div(base, [1.0, sigma * tf]);
    } }
    fft2(&mut a, n, true);
    fft2(&mut cloud, n, true);
    let s = 1.0 / (n * n) as f64;
    let pick = |f: &[C]| -> Vec<f32> { (0..w * hn).map(|i| (f[(i / w) * n + i % w][0] * s).max(0.0) as f32).collect() };
    OroOut { precip: pick(&a), cloud: pick(&cloud) }
}

/// Uplift sensitivity Cw (kg/m^3) of surface air at `temp_c` (deg C) under the environmental lapse rate `lapse_k_per_km` (Smith &
/// Barstad 2004: Cw = rho_s Gamma_m / gamma): the saturation vapour density times the moist adiabatic lapse rate over the environmental
/// one. ponytail: surface pressure 1000 hPa and the physical constants are fixed; a lapse rate under 1 K/km is floored there (Cw grows
/// without bound as the environment nears isothermal).
pub fn uplift_sensitivity(temp_c: f32, lapse_k_per_km: f32) -> f32 {
    // Gas constants of water vapour and dry air (J/kg/K), latent heat of vaporisation (J/kg), heat capacity of dry air (J/kg/K),
    // gravity (m/s^2), and their ratio Rd / Rv.
    const RV: f32 = 461.5;
    const RD: f32 = 287.04;
    const LV: f32 = 2.501e6;
    const CP: f32 = 1004.6;
    const G: f32 = 9.81;
    const EPS: f32 = 0.622;
    let t = temp_c + 273.15;
    let e = vapour_hpa(temp_c);
    let rho_s = e * 100.0 / (RV * t);
    let r = EPS * e / (1000.0 - e);
    let gamma_m = G * (1.0 + LV * r / (RD * t)) / (CP + LV * LV * r * EPS / (RD * t * t));
    rho_s * gamma_m / (lapse_k_per_km.max(1.0) / 1000.0)
}

/// A baked orographic tile, fixed to the terrain: `n` x `n` cells of `cell_m` from (`min_x`, `max_z`) (rows north to south, cell
/// centres like the weather map's texels) with precipitation and cover 0..1, and the camera centre, mean wind and air temperature it
/// was baked for.
#[derive(Clone, Debug, PartialEq)]
pub struct Tile { pub min_x: f32, pub max_z: f32, pub cell_m: f32, pub n: u32, pub precip: Vec<f32>, pub cover: Vec<f32>, pub centre: [f32; 2], pub wind: [f32; 2], pub temp_c: f32 }

/// Bake the tile centred on world `centre` (snapped to whole cells) over ground `g`, for the mean wind `wind_xz` (m/s, world XZ, the
/// direction the air moves) and surface air at `temp_c`. Precipitation (kg/m^2/s x 3600 = mm/h) and the cloud source rate map to 0..1
/// by `full_mm_h`.
pub fn bake<G: Ground + ?Sized>(c: &OroCfg, lapse_k_per_km: f32, g: &G, centre: [f32; 2], wind_xz: [f32; 2], temp_c: f32) -> Tile {
    let n = c.tile_px.max(2);
    let snap = |v: f32| (v / c.cell_m).round() * c.cell_m;
    let half = 0.5 * n as f32 * c.cell_m;
    let (min_x, max_z) = (snap(centre[0]) - half, snap(centre[1]) + half);
    let h: Vec<f32> = (0..n * n).map(|k| g.height(min_x + ((k % n) as f32 + 0.5) * c.cell_m, max_z - ((k / n) as f32 + 0.5) * c.cell_m)).collect();
    // Grid axes: columns run east (+x), rows run south (-z), so the wind's (u, v) is (x, -z).
    let p = OroParams { cell_m: c.cell_m, wind: [wind_xz[0], -wind_xz[1]], gain: c.gain * uplift_sensitivity(temp_c, lapse_k_per_km),
        n_moist: c.n_moist, h_w: c.h_w, tau_c: c.tau_c, tau_f: c.tau_f };
    let out = compute(&h, n as usize, n as usize, &p);
    // ponytail: the floor only guards a zero `full_mm_h`.
    let unit = |v: &f32| (v * 3600.0 / c.full_mm_h.max(1e-6)).min(1.0);
    Tile { min_x, max_z, cell_m: c.cell_m, n, precip: out.precip.iter().map(unit).collect(), cover: out.cloud.iter().map(unit).collect(), centre, wind: wind_xz, temp_c }
}

impl Tile {
    /// [precip, cover] 0..1 at world (x, z): bilinear inside the tile, nothing outside it.
    pub fn sample(&self, x: f32, z: f32) -> [f32; 2] {
        let size = self.n as f32 * self.cell_m;
        if x < self.min_x || x > self.min_x + size || z > self.max_z || z < self.max_z - size { return [0.0; 2]; }
        let (x0, x1, tx) = texel_axis((x - self.min_x) / self.cell_m - 0.5, self.n, false);
        let (z0, z1, tz) = texel_axis((self.max_z - z) / self.cell_m - 0.5, self.n, false);
        let read = |f: &[f32]| {
            let at = |i: u32, j: u32| f[(j * self.n + i) as usize];
            let (a, b) = (at(x0, z0) + (at(x1, z0) - at(x0, z0)) * tx, at(x0, z1) + (at(x1, z1) - at(x0, z1)) * tx);
            a + (b - a) * tz
        };
        [read(&self.precip), read(&self.cover)]
    }

    /// Whether the tile no longer fits: the camera `centre` is over `rebake_m` from the tile's, the mean wind has turned over
    /// `rebake_deg` or changed speed by over `rebake_frac`, or the air has changed over `rebake_k`.
    pub fn stale(&self, c: &OroCfg, centre: [f32; 2], wind_xz: [f32; 2], temp_c: f32) -> bool {
        let (a, b) = (glam::Vec2::from(self.wind), glam::Vec2::from(wind_xz));
        // ponytail: 0.1 m/s counts as calm for the turn test and floors the speed ratio.
        let turned = a.length() > 0.1 && b.length() > 0.1 && a.angle_to(b).abs().to_degrees() > c.rebake_deg;
        glam::Vec2::from(self.centre).distance(glam::Vec2::from(centre)) > c.rebake_m || turned
            || (b.length() - a.length()).abs() > c.rebake_frac * a.length().max(0.1) || (temp_c - self.temp_c).abs() > c.rebake_k
    }
}

/// The weather at a place with the terrain's share `oro` ([precip, cover] from [`Tile::sample`]): precipitation is the larger of the
/// map's and the tile's, cover the map's plus `cover_weight` x the tile's (at most 1).
pub fn combine(c: &OroCfg, map: Local, oro: [f32; 2]) -> Local {
    Local { precip: map.precip.max(oro[0]), coverage: (map.coverage + c.cover_weight * oro[1]).min(1.0), ..map }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Gaussian ridge along v (varies in u only), crest at column w/2, 400 m high, sigma 8 cells.
    fn ridge(w: usize, hn: usize) -> Vec<f32> {
        (0..w * hn).map(|i| { let x = (i % w) as f32 - w as f32 / 2.0; 400.0 * (-x * x / (2.0 * 8.0 * 8.0)).exp() }).collect()
    }
    /// Sum over rows of the columns left (windward for +u wind) and right of the crest.
    fn sides(f: &[f32], w: usize) -> (f32, f32) {
        let (mut l, mut r) = (0.0, 0.0);
        for (i, v) in f.iter().enumerate() { let x = i % w; if x < w / 2 { l += v } else { r += v } }
        (l, r)
    }
    fn run(wind: [f32; 2], h: &[f32]) -> OroOut { compute(h, 64, 64, &OroParams { wind, ..Default::default() }) }

    #[test]
    fn windward_gets_more_than_lee_and_it_flips() {
        let h = ridge(64, 64);
        let (a, b) = sides(&run([10.0, 0.0], &h).precip, 64);
        assert!(a > 1.5 * b, "+u wind: windward {a} lee {b}");
        let (a, b) = sides(&run([-10.0, 0.0], &h).precip, 64);
        assert!(b > 1.5 * a, "-u wind: windward {b} lee {a}");
        let (ca, cb) = sides(&run([10.0, 0.0], &h).cloud, 64);
        assert!(ca > cb, "cloud windward {ca} lee {cb}");
    }
    #[test]
    fn non_negative_and_degenerate_inputs_are_quiet() {
        let h = ridge(64, 64);
        let o = run([7.0, -3.0], &h);
        assert!(o.precip.iter().chain(&o.cloud).all(|v| *v >= 0.0 && v.is_finite()));
        assert!(o.precip.iter().cloned().fold(0.0, f32::max) > 0.0);
        let z = run([0.0, 0.0], &h);
        assert!(z.precip.iter().chain(&z.cloud).all(|v| *v == 0.0), "zero wind");
        let f = run([10.0, 5.0], &vec![123.0; 64 * 64]);
        assert!(f.precip.iter().chain(&f.cloud).all(|v| v.abs() < 1e-4), "flat map");
    }
    #[test]
    fn fft_round_trips() {
        let mut a: Vec<C> = (0..16).map(|i| [(i * i % 7) as f64, 0.0]).collect();
        let b = a.clone();
        fft2(&mut a, 4, false); fft2(&mut a, 4, true);
        for (x, y) in a.iter().zip(&b) { assert!((x[0] / 16.0 - y[0]).abs() < 1e-9); }
    }
    // Budget: saturation vapour density at 10 C is 9.4 g/m^3 (standard tables), and Gamma_m / gamma at 10 C under 6.5 K/km is about
    // 0.8, so Cw is 7-8 g/m^3; 6-9 allowed.
    #[test]
    fn uplift_sensitivity_follows_the_air() {
        let c = uplift_sensitivity(10.0, 6.5);
        println!("Cw at 10 C: {c:.5} kg/m^3 (budget 0.006-0.009)");
        assert!((0.006..=0.009).contains(&c), "Cw(10 C) = {c} kg/m^3, budget 0.006-0.009");
        assert!(uplift_sensitivity(20.0, 6.5) > c && c > uplift_sensitivity(0.0, 6.5), "warmer air holds more water");
        assert!(uplift_sensitivity(10.0, 0.0).is_finite(), "no lapse rate is floored, not infinite");
    }

    /// A 1000 m ridge (sigma 3 km) over 25.6 km at 100 m cells, running north-south (crest x = 0) or east-west (crest z = 0).
    fn ridge_grid(east_west: bool) -> crate::rain::HeightGrid {
        let (n, cell) = (256u32, 100.0f32);
        let y = (0..n * n).map(|k| {
            let (x, z) = (-12800.0 + ((k % n) as f32 + 0.5) * cell, 12800.0 - ((k / n) as f32 + 0.5) * cell);
            let d = if east_west { z } else { x };
            1000.0 * (-d * d / (2.0 * 3000.0 * 3000.0)).exp()
        }).collect();
        crate::rain::HeightGrid { min_x: -12800.0, max_z: 12800.0, cell_m: cell, w: n, h: n, y }
    }

    // Gate (roadmap, orographic precipitation): a 10 m/s wind at 10 C over the ridge, the map dry (precip 0, cover 0.5). In Smith &
    // Barstad's theory the air condenses on the ascent, and the fallout delays (tau_c + tau_f = 2000 s) carry the rain about U x tau
    // downwind, so on a ridge this narrow it peaks just past the crest. Budgets: 3 km upwind of the crest it rains (precip at least
    // `rain.precip_lo`) under added cloud (cover above the cover 3 km downwind, where the air descends), and 4.5 km downwind gets at
    // least 1.5x the rain of 4.5 km upwind (the drift; about 3x measured). Both tile axes run (a westerly over a north-south ridge, a
    // southerly over an east-west one): a flipped axis swaps upwind and downwind and fails. Red arm `gain` 0.
    #[test]
    fn rain_condenses_upwind_and_drifts_downwind() {
        let map = Local { coverage: 0.5, precip: 0.0, dust: 0.0 };
        let lo = crate::config::RainCfg::default().precip_lo;
        // The weather `k` metres along the wind from the crest (negative = upwind).
        let profile = |c: &OroCfg, ew: bool| {
            let t = bake(c, 6.5, &ridge_grid(ew), [0.0, 0.0], if ew { [0.0, 10.0] } else { [10.0, 0.0] }, 10.0);
            let at = |k: f32| { let [x, z] = if ew { [0.0, k] } else { [k, 0.0] }; combine(c, map, t.sample(x, z)) };
            (at(-3000.0), at(3000.0), at(-4500.0), at(4500.0))
        };
        let ok = |c: &OroCfg| [false, true].iter().all(|&ew| {
            let (up3, down3, up45, down45) = profile(c, ew);
            up3.precip >= lo && up3.coverage > down3.coverage && down45.precip > 0.0 && down45.precip >= 1.5 * up45.precip
        });
        for ew in [false, true] {
            let (up3, down3, up45, down45) = profile(&OroCfg::default(), ew);
            println!("{}: 3 km upwind precip {:.3} cover {:.3}, 3 km downwind cover {:.3}; precip 4.5 km upwind {:.3}, downwind {:.3} (budget precip >= {lo}, cover upwind > downwind, downwind >= 1.5x upwind)",
                if ew { "southerly over an east-west ridge" } else { "westerly over a north-south ridge" }, up3.precip, up3.coverage, down3.coverage, up45.precip, down45.precip);
        }
        assert!(ok(&OroCfg::default()), "the ascent must rain under added cloud and the rain must drift downwind");
        assert!(!ok(&OroCfg { gain: 0.0, ..OroCfg::default() }), "red arm (gain 0) passes: the gate cannot fail");
    }

    #[test]
    fn tiles_combine_sample_and_go_stale() {
        let c = OroCfg::default();
        let t = bake(&c, 6.5, &ridge_grid(false), [0.0, 0.0], [10.0, 0.0], 10.0);
        assert_eq!(t.sample(1.0e5, 0.0), [0.0, 0.0], "outside the tile adds nothing");
        let m = Local { coverage: 0.8, precip: 0.6, dust: 0.2 };
        let k = combine(&c, m, [0.3, 0.5]);
        assert_eq!((k.precip, k.coverage, k.dust), (0.6, 1.0, 0.2));
        assert_eq!(combine(&c, m, [0.9, 0.0]).precip, 0.9);
        assert!(!t.stale(&c, [100.0, 0.0], [10.0, 0.0], 10.5), "small moves keep the tile");
        assert!(t.stale(&c, [c.rebake_m + 1.0, 0.0], [10.0, 0.0], 10.0), "camera moved");
        assert!(t.stale(&c, [0.0, 0.0], [0.0, 10.0], 10.0), "wind turned 90 deg");
        assert!(t.stale(&c, [0.0, 0.0], [15.0, 0.0], 10.0), "wind 50 % faster");
        assert!(t.stale(&c, [0.0, 0.0], [10.0, 0.0], 13.0), "air 3 K warmer");
    }
}
