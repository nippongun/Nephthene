//! Precipitation and cloud from terrain and wind: Smith & Barstad (2004) linear orographic precipitation, one FFT pass.
//! Pure, no engine. Grid axes: column index = +u, row index = +v; `wind` is (u, v) m/s in those axes, the direction the air moves.
//! Result for one wavenumber (k, l), sigma = U k + V l:
//!   P^ = Cw * i sigma h^ / ((1 - i m Hw)(1 + i sigma tau_c)(1 + i sigma tau_f)),  m^2 = (N^2 - sigma^2)(k^2 + l^2) / sigma^2
//! (m imaginary when sigma^2 > N^2). Precip is the real part of the inverse transform, clipped at 0.
//! All constants in `OroParams::default()` are untuned.

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
}
