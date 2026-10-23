//! Test terrain for looking at terrain-driven weather: a mountain range. Pure, deterministic, no engine. A fixture like
//! `fog::test_valley`, not a tuned look: its shape constants are fixed here, not in `Config`.

use crate::rain::HeightGrid;

/// Integer lattice hash to 0..1 (a multiply-xorshift mix; any well-mixed hash works).
fn hash(i: i32, j: i32) -> f32 {
    let mut h = (i as u32).wrapping_mul(0x8da6_b343) ^ (j as u32).wrapping_mul(0xd816_3841);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 0x00ff_ffff as f32
}

/// Perlin gradient noise at (x, z) in lattice units, about -0.7..0.7: a hashed unit gradient per lattice point, quintic fade.
fn noise(x: f32, z: f32) -> f32 {
    let (i, j) = (x.floor(), z.floor());
    let (fx, fz) = (x - i, z - j);
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (sx, sz) = (fade(fx), fade(fz));
    let g = |di: i32, dj: i32| {
        let a = hash(i as i32 + di, j as i32 + dj) * std::f32::consts::TAU;
        a.cos() * (fx - di as f32) + a.sin() * (fz - dj as f32)
    };
    let (ab, cd) = (g(0, 0) + (g(1, 0) - g(0, 0)) * sx, g(0, 1) + (g(1, 1) - g(0, 1)) * sx);
    ab + (cd - ab) * sz
}

/// Ridged multifractal 0..1: six octaves of 1 - |noise| from a 9 km wavelength down, squared into sharp crests, each octave
/// weighted by the one before so detail gathers on the ridges and the valleys stay smooth.
fn ridged(x: f32, z: f32) -> f32 {
    let (mut sum, mut norm, mut amp, mut f, mut w) = (0.0, 0.0, 1.0, 1.0 / 9000.0, 1.0f32);
    for o in 0..6 {
        let v = (1.0 - noise(x * f + o as f32 * 17.3, z * f - o as f32 * 9.1).abs() * 1.4).clamp(0.0, 1.0);
        let v = v * v * w;
        w = (v * 1.5).clamp(0.0, 1.0);
        sum += amp * v;
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    sum / norm
}

/// A mountain range on a 48 km square at 100 m cells (480 x 480, the test valley's sample count), centred on the origin: a range
/// about 14 km wide running north-south, its crest wandering 1.5 km east and west, with peaks up to about 2.9 km, saddles near 1 km,
/// side valleys and spurs (ridged noise under a Gaussian envelope across x), on plains 0-60 m high either side. The default
/// westerly crosses it: the west plain is upwind, the east plain the lee.
pub fn mountains() -> HeightGrid {
    let (n, cell) = (480u32, 100.0f32);
    let (min_x, max_z) = (-(n as f32) * cell / 2.0, n as f32 * cell / 2.0);
    let y = (0..n * n).map(|k| {
        let (x, z) = (min_x + ((k % n) as f32 + 0.5) * cell, max_z - ((k / n) as f32 + 0.5) * cell);
        let crest_x = 1500.0 * (z / 9000.0).sin();
        let envelope = (-((x - crest_x) / 7000.0).powi(2)).exp();
        3000.0 * envelope * ridged(x, z) + 30.0 * (noise(x / 4000.0, z / 4000.0) + 0.7)
    }).collect();
    HeightGrid { min_x, max_z, cell_m: cell, w: n, h: n, y }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rain::Ground;

    // A fixture check, not a gate: the range stands where the poses (examples/poses.json mtn_*) look, its plains are low enough to
    // stand on, and it is the same every run. Budgets: the highest peak 2.2-3.2 km (a range that pierces the default cloud base,
    // 1.5 km); every 2 km band of the crest line (|x| < 6 km) tops 800 m (no gap the range vanishes through); the plains at
    // x = -15 and +15 km stay under 150 m along z = -6..6 km (the mtn_west and mtn_east cameras stand at 250 m there).
    #[test]
    fn a_range_on_low_plains() {
        let g = mountains();
        assert_eq!(g.y, mountains().y, "deterministic");
        assert!(g.y.iter().all(|v| v.is_finite() && *v >= 0.0), "finite, at or above 0");
        let top = g.y.iter().cloned().fold(0.0, f32::max);
        let band_low = (-11..11).map(|b| {
            let z0 = b as f32 * 2000.0;
            (0..20).flat_map(|i| (-60..=60).map(move |j| (z0 + i as f32 * 100.0, j as f32 * 100.0)))
                .map(|(z, x)| g.height(x, z)).fold(0.0, f32::max)
        }).fold(f32::INFINITY, f32::min);
        let g = &g;
        let plain = [-15000.0f32, 15000.0].iter().flat_map(|&x| (-60..=60).map(move |j| g.height(x, j as f32 * 100.0))).fold(0.0, f32::max);
        println!("mountains: top {top:.0} m, lowest 2 km crest band {band_low:.0} m, plains at x = +-15 km max {plain:.0} m (budget 2200-3200, >= 800, < 150)");
        assert!((2200.0..=3200.0).contains(&top) && band_low >= 800.0 && plain < 150.0);
    }
}
