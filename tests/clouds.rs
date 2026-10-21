//! Cloud gates G7-G14 (headless; every shot is the `sky` example in `--shot` mode). Manual gates are #[ignore]:
//! `cargo test --release --test clouds -- --ignored --nocapture` runs them all, `... g11` one. Metrics print before asserting.
//! Every threshold is a budget (reference run's own noise plus a stated allowance), not a fit to a measurement.
//! Red arms use config switches (`--set`): resolve.no_reproject, clouds.temporal_alpha = 1 and the update_div / steps references.
//! Each gate that proves a fix says which config change reproduces the old behaviour.
mod common;
use common::*;
use image::RgbImage;

fn blur(v: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let r = (sigma * 3.0) as i32;
    let k: Vec<f32> = (-r..=r).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let norm: f32 = k.iter().sum();
    let mut tmp = vec![0f32; v.len()];
    for y in 0..h { for x in 0..w {
        tmp[y * w + x] = (-r..=r).map(|i| v[y * w + (x as i32 + i).clamp(0, w as i32 - 1) as usize] * k[(i + r) as usize]).sum::<f32>() / norm;
    } }
    let mut out = vec![0f32; v.len()];
    for y in 0..h { for x in 0..w {
        out[y * w + x] = (-r..=r).map(|i| tmp[(y as i32 + i).clamp(0, h as i32 - 1) as usize * w + x] * k[(i + r) as usize]).sum::<f32>() / norm;
    } }
    out
}

/// Luma of the sky rows (top 80%).
fn sky_luma(im: &RgbImage) -> (Vec<f32>, usize, usize) {
    let (w, h) = (im.width() as usize, im.height() as usize * 4 / 5);
    ((0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| luma(im.get_pixel(x as u32, y as u32))).collect(), w, h)
}

// ---------------------------------------------------------------- metrics

/// Worst per-frame mean |luma| difference over the top 45% rows (G7).
fn g7_diff(a: &[RgbImage], b: &[RgbImage]) -> f32 {
    a.iter().zip(b).map(|(x, y)| {
        let rows = x.height() * 45 / 100;
        let mut s = 0f64;
        for py in 0..rows { for px in 0..x.width() { s += (luma(x.get_pixel(px, py)) - luma(y.get_pixel(px, py))).abs() as f64; } }
        (s / (rows * x.width()) as f64) as f32
    }).fold(0.0, f32::max)
}

/// Mean |luma| difference over the sky rows 40..460 of a 540-row image, scaled to the image height (G8).
fn g8_diff(a: &RgbImage, b: &RgbImage) -> f32 {
    let (y0, y1) = (a.height() * 40 / 540, a.height() * 460 / 540);
    let mut s = 0f64;
    for y in y0..y1 { for x in 0..a.width() { s += (luma(a.get_pixel(x, y)) - luma(b.get_pixel(x, y))).abs() as f64; } }
    (s / ((y1 - y0) * a.width()) as f64) as f32
}

/// RMS of the row means (64-px wide strips) of the luma difference to the reference after removing a 9-row box average (G9).
fn g9_stripe(a: &RgbImage, b: &RgbImage) -> f32 {
    let (mut s, mut n) = (0f64, 0u32);
    let (y0, y1) = (a.height() * 40 / 540, a.height() * 420 / 540);
    for x0 in (0..a.width().saturating_sub(63)).step_by(64) {
        let rows: Vec<f32> = (y0..y1).map(|y| (x0..x0 + 64).map(|x| luma(a.get_pixel(x, y)) - luma(b.get_pixel(x, y))).sum::<f32>() / 64.0).collect();
        for i in 4..rows.len() - 4 {
            let avg = rows[i - 4..=i + 4].iter().sum::<f32>() / 9.0;
            s += ((rows[i] - avg) as f64).powi(2);
            n += 1;
        }
    }
    (s / n as f64).sqrt() as f32
}

/// Dots per 1000 sky px: a dot is a pixel more than 3 levels off the median of its 3x3 (G10).
fn g10_dots(imgs: &[RgbImage]) -> f32 {
    let l = |im: &RgbImage, x: u32, y: u32| luma(im.get_pixel(x, y));
    let (mut dots, mut n) = (0u64, 0u64);
    for im in imgs {
        for y in 1..im.height() * 4 / 5 { for x in 1..im.width() - 1 {
            let mut w = [0f32; 9];
            for (i, (dx, dy)) in (0..9).map(|i| (i % 3, i / 3)).enumerate() { w[i] = l(im, x + dx - 1, y + dy - 1); }
            let c = w[4];
            w.sort_by(|a, b| a.partial_cmp(b).unwrap());
            if (c - w[4]).abs() > 3.0 { dots += 1; }
            n += 1;
        } }
    }
    dots as f32 * 1000.0 / n as f32
}

/// Step-edge energy at the worst phase of x % 8 (and y % 8) over the median phase, sky rows: 1 = no 8 px grid (G11).
fn g11_blockiness(im: &RgbImage) -> f32 {
    let (l, w, h) = sky_luma(im);
    let mut axes = [0f32; 2];
    for (a, axis) in axes.iter_mut().enumerate() {
        let mut ph = [0f64; 8];
        let mut n = [0u64; 8];
        let (len_u, len_v) = if a == 0 { (w, h) } else { (h, w) };
        for v in 0..len_v { for u in 0..len_u - 1 {
            let (i, j) = if a == 0 { (v * w + u, v * w + u + 1) } else { (u * w + v, (u + 1) * w + v) };
            ph[u % 8] += (l[j] - l[i]).abs() as f64;
            n[u % 8] += 1;
        } }
        let mut e: Vec<f64> = (0..8).map(|p| ph[p] / n[p] as f64).collect();
        let max = e.iter().cloned().fold(0.0, f64::max);
        e.sort_by(|a, b| a.partial_cmp(b).unwrap());
        *axis = (max / ((e[3] + e[4]) / 2.0)) as f32;
    }
    (axes[0] + axes[1]) / 2.0
}

/// RMS of the 8 px-scale (gaussian sigma 4) luma difference of two frames, sky rows (G12).
fn g12_coarse(a: &RgbImage, b: &RgbImage) -> f32 {
    let ((la, w, h), (lb, _, _)) = (sky_luma(a), sky_luma(b));
    let d: Vec<f32> = la.iter().zip(&lb).map(|(x, y)| x - y).collect();
    let lo = blur(&d, w, h, 4.0);
    (lo.iter().map(|v| v * v).sum::<f32>() / lo.len() as f32).sqrt()
}

/// Mean (d/dy)^2 / mean (d/dx)^2 of the band-pass (gaussian 2 minus 8 px) luma: 1 = isotropic grain, > 1 = horizontal streaks (G13).
fn g13_lines(im: &RgbImage) -> f32 {
    let (l, w, h) = sky_luma(im);
    let (a, b) = (blur(&l, w, h, 2.0), blur(&l, w, h, 8.0));
    let band: Vec<f32> = a.iter().zip(&b).map(|(x, y)| x - y).collect();
    let (mut gy, mut gx) = (0f64, 0f64);
    for y in 0..h - 1 { for x in 0..w - 1 {
        gy += ((band[(y + 1) * w + x] - band[y * w + x]) as f64).powi(2);
        gx += ((band[y * w + x + 1] - band[y * w + x]) as f64).powi(2);
    } }
    (gy / gx) as f32
}

/// The metrics must find what they claim, on synthetic images, before any shot is trusted.
#[test]
fn metric_selftest() {
    let (w, h) = (800u32, 450u32);
    let flat = RgbImage::from_fn(w, h, |_, _| image::Rgb([120, 120, 120]));
    // 8 px grid: alternating 8x8 tiles differ by 12 levels.
    let noise = RgbImage::from_fn(w, h, |x, y| { let v = 120 + ((x * 7 + y * 13) % 5) as u8; image::Rgb([v, v, v]) });
    let grid = RgbImage::from_fn(w, h, |x, y| { let v = 120 + ((x * 7 + y * 13) % 5) as u8 + if (x / 8 + y / 8) % 2 == 0 { 12 } else { 0 }; image::Rgb([v, v, v]) });
    assert!(g11_blockiness(&grid) > 1.5 && g11_blockiness(&noise) < 1.5, "blockiness grid {} noise {}", g11_blockiness(&grid), g11_blockiness(&noise));
    // Dots: one bright pixel in every 4x4 block.
    let dots = RgbImage::from_fn(w, h, |x, y| { let v = if x % 4 == 0 && y % 4 == 0 { 140 } else { 120 }; image::Rgb([v, v, v]) });
    assert!(g10_dots(&[dots.clone()]) > 30.0 && g10_dots(&[flat.clone()]) == 0.0);
    // Horizontal streaks: rows of alternating luma, 12 px period.
    let rows = RgbImage::from_fn(w, h, |_, y| { let v = if (y / 6) % 2 == 0 { 100 } else { 140 }; image::Rgb([v, v, v]) });
    let cols = RgbImage::from_fn(w, h, |x, _| { let v = if (x / 6) % 2 == 0 { 100 } else { 140 }; image::Rgb([v, v, v]) });
    assert!(g13_lines(&rows) > 3.0 && g13_lines(&cols) < 0.5, "lines rows {} cols {}", g13_lines(&rows), g13_lines(&cols));
    // Coarse error: a smooth 16 px blob difference reads, identical frames read 0.
    let blob = RgbImage::from_fn(w, h, |x, y| { let v = if ((x / 24) + (y / 24)) % 2 == 0 { 140 } else { 100 }; image::Rgb([v, v, v]) });
    assert!(g12_coarse(&blob, &flat) > 5.0 && g12_coarse(&flat, &flat) == 0.0);
    // Stripes: row-constant offsets show, a uniform offset does not.
    let striped = RgbImage::from_fn(w, h, |_, y| { let v = if y % 2 == 0 { 126 } else { 114 }; image::Rgb([v, v, v]) });
    assert!(g9_stripe(&striped, &flat) > 3.0 && g9_stripe(&noise, &noise) == 0.0);
    assert!(g7_diff(&[flat.clone()], &[flat.clone()]) == 0.0 && g8_diff(&grid, &flat) > 1.0);
}

// ---------------------------------------------------------------- G7 turning camera

/// G7: a swaying camera must not drag the cloud history. Six frames, 7 apart, under `--sway 3` (deg), once with the default update (div 4)
/// and once as the reference (`clouds.update_div` 1: every pixel marched every frame). Ghosting = worst per-frame mean |luma difference|
/// over the sky rows. Red arm: `resolve.no_reproject` (history sampled unreprojected). Limit 1.5.
const G7_LIMIT: f32 = 1.5;

fn g7_seq(tag: &str, set: &str) -> Vec<RgbImage> {
    seq(&format!("g7_{tag}"), "sky_west", 6, &["--hour", "18.5", "--set", "clouds.coverage_override=0.4", "--settle", "200", "--set", "clouds.update_div=4", "--sway", "3", "--seq", "6,7", "--set", set])
}

#[test]
#[ignore = "manual gate"]
fn g7_turning_ghost() {
    let reference = g7_seq("ref", "clouds.update_div=1");
    let shipped = g7_diff(&reference, &g7_seq("def", "clouds.update_div=4"));
    let red = g7_diff(&reference, &g7_seq("red", "resolve.no_reproject=true"));
    println!("G7 turning camera ghost: default {shipped:.2} (limit {G7_LIMIT}), no-reprojection red arm {red:.2}");
    let mut f = vec![];
    if shipped > G7_LIMIT { f.push(format!("G7 ghost {shipped:.2} > {G7_LIMIT}")); }
    if red <= G7_LIMIT { f.push(format!("G7 red arm {red:.2} not above {G7_LIMIT}; the check does not see ghosting")); }
    finish("G7", f);
}

// ---------------------------------------------------------------- G8 blocky edges

/// G8: at the default settle (60 frames) the div-4 cloud sky must already be close to the div-1 reference. A stale 4 px lattice of history
/// shows as a staircase on cloud edges. Score = mean |luma difference| over the sky rows, on the look and on the alpha mask
/// (`clouds.debug_view` 4). Limit 0.7.
const G8_LIMIT: f32 = 0.7;

#[test]
#[ignore = "manual gate"]
fn g8_blocky_edges() {
    let mut f = vec![];
    for (tag, view) in [("look", "0"), ("alpha", "4")] {
        let args = |div: &'static str| ["--hour", "18.5", "--set", "clouds.coverage_override=0.4", "--set", Box::leak(format!("clouds.debug_view={view}").into_boxed_str()) as &str, "--set", div];
        let ship = still(&format!("g8_{tag}"), "sky_west", &args("clouds.update_div=4"));
        let reference = still(&format!("g8_{tag}_ref"), "sky_west", &args("clouds.update_div=1"));
        let d = g8_diff(&ship, &reference);
        println!("G8 {tag}: mean |luma| vs full-res reference {d:.2} (limit {G8_LIMIT})");
        if d > G8_LIMIT { f.push(format!("G8 {tag} {d:.2} > {G8_LIMIT}")); }
    }
    finish("G8", f);
}

// ---------------------------------------------------------------- G9 stripes

/// G9: stacked horizontal bands inside cloud bodies with the camera still. Under the div-4 update a pixel is marched every 16th frame; a
/// per-visit jitter step that lands on a coherent lattice leaves each row at a fixed fraction of a step. Score = row-stripe RMS of the luma
/// difference to the div-1 reference. Limit 0.18.
const G9_LIMIT: f32 = 0.18;

#[test]
#[ignore = "manual gate"]
fn g9_stripes() {
    let mut f = vec![];
    for (tag, hour, cov, settle) in [("a", "18", "0.5", "300"), ("b", "18", "0.5", "120"), ("c", "17.5", "0.7", "300")] {
        let cov = format!("clouds.coverage_override={cov}");
        let args = |div: &'static str| vec!["--hour", hour, "--set", cov.as_str(), "--settle", settle, "--set", div];
        let ship = still(&format!("g9_{tag}"), "sky_east", &args("clouds.update_div=4"));
        let reference = still(&format!("g9_{tag}_ref"), "sky_east", &args("clouds.update_div=1"));
        let d = g9_stripe(&ship, &reference);
        println!("G9 {tag}: row-stripe RMS of (div 4 - reference) {d:.3} (limit {G9_LIMIT})");
        if d > G9_LIMIT { f.push(format!("G9 {tag} stripes {d:.3} > {G9_LIMIT}")); }
    }
    finish("G9", f);
}

// ---------------------------------------------------------------- G10 dot lattice while turning

/// G10: a turning camera at Retina size (2880x1620) must not show the div-4 update as a dot lattice. Eight frames 3 apart under `--sway 8`,
/// div 4 vs the div-1 reference. Budget: no more dots than the reference has (ratio <= 1). Red arm: `resolve.motion_half_px` 1 (the marched pixel counts fewer
/// visits while the view moves, so it takes its single sample while its 15 neighbours keep smooth history: a lattice).
const G10_RATIO: f32 = 1.0;

fn g10_seq(tag: &str, sets: &[&str]) -> Vec<RgbImage> {
    let mut a = vec!["--hour", "17.5", "--settle", "60", "--sway", "8", "--seq", "8,3"];
    for s in sets { a.extend(["--set", s]); }
    seq(&format!("g10_{tag}"), "golden_ne25_retina", 8, &a)
}

#[test]
#[ignore = "manual gate"]
fn g10_turning_dots() {
    let reference = g10_dots(&g10_seq("ref", &["clouds.update_div=1"]));
    let shipped = g10_dots(&g10_seq("def", &["clouds.update_div=4"]));
    let red = g10_dots(&g10_seq("red", &["clouds.update_div=4", "resolve.motion_half_px=1.0"]));
    println!("G10 dots per 1000 sky px while turning at Retina: default {shipped:.3}, reference {reference:.3}, ratio {:.2} (limit {G10_RATIO}); red arm (motion_half_px 1) {red:.3}", shipped / reference);
    let mut f = vec![];
    if shipped > reference * G10_RATIO { f.push(format!("G10 dot lattice: {shipped:.3} > {reference:.3} x {G10_RATIO}")); }
    if red <= reference * G10_RATIO { f.push(format!("G10 red arm {red:.3} not above {:.3}; the check does not see the lattice", reference * G10_RATIO)); }
    finish("G10", f);
}

// ---------------------------------------------------------------- G11 flat block mosaic, still camera

/// G11: camera still, clock and wind running, Retina: a mosaic of flat 8 px squares (the div-4 march block at half-res) on cloud edges and
/// thin cloud, never converging, if the history clamp box over single grainy samples collapses to empty sky and pins whole blocks.
/// Eight frames 40 apart (`--live`, 60 settle) at 2880x1620, div 4 vs the div-1 reference. Blockiness = step-edge energy at the worst phase
/// over the median phase: 1 = no grid. Budget: the grid excess (blockiness - 1) of the last four frames may be at most twice the
/// reference's own excess. Red arm: `resolve.still_px` 0 (the 5x5 box is never used: always the 3x3 box).
const G11_EXCESS_RATIO: f32 = 2.0;

fn g11_seq(tag: &str, sets: &[&str]) -> Vec<RgbImage> {
    let mut a = vec!["--hour", "17.5", "--live", "--settle", "60", "--seq", "8,40"];
    for s in sets { a.extend(["--set", s]); }
    seq(&format!("g11_{tag}"), "golden_ne25_retina", 8, &a)
}

#[test]
#[ignore = "manual gate"]
fn g11_still_blocks() {
    let last = |v: &[RgbImage]| v[4..].iter().map(g11_blockiness).sum::<f32>() / 4.0;
    let reference = last(&g11_seq("ref", &["clouds.update_div=1"]));
    let shipped = last(&g11_seq("def", &["clouds.update_div=4"]));
    let red = last(&g11_seq("red", &["clouds.update_div=4", "resolve.still_px=0.0"]));
    let (es, er, ered) = (shipped - 1.0, reference - 1.0, red - 1.0);
    println!("G11 still camera, live weather, Retina: blockiness (last 4 frames) default {shipped:.3}, reference {reference:.3}, red arm (still_px 0) {red:.3}; grid excess {es:.3} vs {er:.3}, ratio {:.1} (limit {G11_EXCESS_RATIO})", es / er);
    let mut f = vec![];
    if es > er * G11_EXCESS_RATIO { f.push(format!("G11 block mosaic: excess {es:.3} > {er:.3} x {G11_EXCESS_RATIO}")); }
    if ered <= er * G11_EXCESS_RATIO { f.push(format!("G11 red arm excess {ered:.3} not above {:.3}; the check does not see the mosaic", er * G11_EXCESS_RATIO)); }
    finish("G11", f);
}

// ---------------------------------------------------------------- G12 coarse chunks after a cut

/// G12: after a teleport or a hard turn the cloud picture shows coarse 8 px chunks (the bilinear upsample of one jittered sample per 4x4 block)
/// that take seconds to fizzle out. Retina pose, frozen weather, `--turn 120,1` (a cut), div 4 vs the div-1 reference. Score = rms of the
/// 8 px-scale luma difference of the two, sky rows. Budget: one update cycle (16 frames) after the cut the div-4 picture must have halved the
/// error of the first frame after the cut (the first frame is the unavoidable bilinear upsample): ratio <= 0.5.
/// Red arm: no cut detection (`resolve.cut_deg` 180, `resolve.cut_m` 1e9) and `resolve.fresh` 15 (a revealed pixel trusts its marched visits at weight 1/16 only, the global alpha floor): the fill-in waits for ~10 visits.
const G12_RATIO: f32 = 0.5;

fn g12_seq(tag: &str, sets: &[&str]) -> Vec<RgbImage> {
    let mut a = vec!["--hour", "17.5", "--settle", "60", "--turn", "120,1", "--at", "2,16"];
    for s in sets { a.extend(["--set", s]); }
    seq(&format!("g12_{tag}"), "golden_ne25_retina", 3, &a)
}

#[test]
#[ignore = "manual gate"]
fn g12_cut_converge() {
    let reference = g12_seq("ref", &["clouds.update_div=1"]);
    let ratio = |s: &[RgbImage]| { let (e1, e16) = (g12_coarse(&s[1], &reference[1]), g12_coarse(&s[2], &reference[2])); (e1, e16, e16 / e1) };
    let (e1, e16, r) = ratio(&g12_seq("def", &["clouds.update_div=4"]));
    let (re1, re16, rred) = ratio(&g12_seq("red", &["clouds.update_div=4", "resolve.cut_deg=180.0", "resolve.cut_m=1e9", "resolve.fresh=15.0"]));
    println!("G12 coarse 8 px error vs div 1 after a cut: +1 frame {e1:.2}, +15 frames {e16:.2}, ratio {r:.2} (limit {G12_RATIO}); red arm +1 {re1:.2}, +15 {re16:.2}, ratio {rred:.2}");
    let mut f = vec![];
    if r > G12_RATIO { f.push(format!("G12 slow fill-in after a cut: ratio {r:.2} > {G12_RATIO}")); }
    if rred <= G12_RATIO { f.push(format!("G12 red arm ratio {rred:.2} not above {G12_RATIO}; the check does not see the slow fill-in")); }
    finish("G12", f);
}

// ---------------------------------------------------------------- G13 horizontal streak lines

/// G13: short horizontal streak lines stacked at ~20 px in lit cloud bodies, present at div 1 too, so not the resolve: the sun light march
/// shared its jitter with the eye march (correlated, altitude-dependent bias); more steps hide them. Score = row/column band energy of the
/// sky rows, Retina pose, still, frozen weather. Budget: no more streaks than the same picture marched at three times the step budget with
/// every pixel every frame (`clouds.steps` 216, `clouds.update_div` 1): ratio <= 1. Red arm: `resolve.light_jitter_shared`.
const G13_RATIO: f32 = 1.0;

#[test]
#[ignore = "manual gate"]
fn g13_streak_lines() {
    let shot = |tag: &str, sets: &[&str]| { let mut a = vec!["--hour", "17.5", "--settle", "200"]; for s in sets { a.extend(["--set", s]); } g13_lines(&still(tag, "golden_ne25_retina", &a)) };
    let ship = shot("g13_def", &["clouds.update_div=4", "clouds.steps=72"]);
    let reference = shot("g13_ref", &["clouds.update_div=1", "clouds.steps=216"]);
    let red = shot("g13_red", &["clouds.update_div=4", "clouds.steps=72", "resolve.light_jitter_shared=true"]);
    println!("G13 row/col band energy, still Retina: default {ship:.3}, reference (216 steps, div 1) {reference:.3}, ratio {:.2} (limit {G13_RATIO}); red arm (shared jitter) {red:.3}", ship / reference);
    let mut f = vec![];
    if ship > reference * G13_RATIO { f.push(format!("G13 streak lines: {ship:.3} > {reference:.3} x {G13_RATIO}")); }
    if red <= reference * G13_RATIO { f.push(format!("G13 red arm {red:.3} not above {:.3}; the check does not see the streaks", reference * G13_RATIO)); }
    finish("G13", f);
}

// ---------------------------------------------------------------- G14 continuous turn, revealed side

/// G14: a pixel is marched once per 16 frames, so on a continuously turning view the strip revealed in the last ~12 frames (the leading 15% of the
/// width at 45 deg/s) held its first bilinear upsample, a different one in each frame's band: hard vertical seams and 8 px blotches. Retina pose,
/// live weather, continuous yaw 45 deg/s (`--turn 750,1000`), 8 frames 20 apart, div 4 vs div 1 at the same frame. Score = rms of the 8 px-scale
/// (gaussian sigma 4) luma difference over CLOUD pixels (red - blue > -25 in the reference, dilated), leading 15% columns over the rest, mean over
/// the frames. Budget: the revealed strip may be half again as wrong as the settled interior (the resolve's own error): ratio <= 1.5.
/// Red arm: `resolve.upsample_mean_frames` 0 and `resolve.wide_upsample` false (no running mean, plain bilinear upsample).
const G14_RATIO: f32 = 1.5;

fn g14_lead_rest(a: &[RgbImage], b: &[RgbImage]) -> (f32, f32) {
    let (mut lead, mut rest) = (0f32, 0f32);
    for (x, r) in a.iter().zip(b) {
        let ((la, w, h), (lb, _, _)) = (sky_luma(x), sky_luma(r));
        let d: Vec<f32> = la.iter().zip(&lb).map(|(p, q)| p - q).collect();
        let lo = blur(&d, w, h, 4.0);
        let cloud: Vec<f32> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| { let p = r.get_pixel(x as u32, y as u32); if p[0] as i32 - p[2] as i32 > -25 { 1.0 } else { 0.0 } }).collect();
        let near = blur(&cloud, w, h, 8.0);
        let (mut sl, mut nl, mut sr, mut nr) = (0f32, 0f32, 0f32, 0f32);
        for i in 0..w * h { if near[i] > 0.1 {
            if i % w < w * 15 / 100 { sl += lo[i] * lo[i]; nl += 1.0 } else { sr += lo[i] * lo[i]; nr += 1.0 }
        } }
        lead += (sl / nl.max(1.0)).sqrt(); rest += (sr / nr.max(1.0)).sqrt();
    }
    (lead / a.len() as f32, rest / a.len() as f32)
}

fn g14_seq(tag: &str, sets: &[&str]) -> Vec<RgbImage> {
    let mut a = vec!["--hour", "17.5", "--live", "--settle", "60", "--turn", "750,1000", "--at", "20,40,60,80,100,120,140,160"];
    for s in sets { a.extend(["--set", s]); }
    seq(&format!("g14_{tag}"), "golden_ne25_retina", 8, &a)
}

#[test]
#[ignore = "manual gate"]
fn g14_turn_reveal() {
    let reference = g14_seq("ref", &["clouds.update_div=1"]);
    let score = |s: &[RgbImage]| { let (l, r) = g14_lead_rest(s, &reference); (l, r, l / r) };
    let (lead, rest, ratio) = score(&g14_seq("def", &["clouds.update_div=4"]));
    let (rl, rr, rred) = score(&g14_seq("red", &["clouds.update_div=4", "resolve.upsample_mean_frames=0.0", "resolve.wide_upsample=false"]));
    println!("G14 cloud 8 px error vs div 1, turning 45 deg/s: leading 15% {lead:.2}, rest {rest:.2}, ratio {ratio:.2} (limit {G14_RATIO}); red arm (no mean, no wide) {rl:.2}, {rr:.2}, ratio {rred:.2}");
    let mut f = vec![];
    if ratio > G14_RATIO { f.push(format!("G14 revealed strip: ratio {ratio:.2} > {G14_RATIO}")); }
    if rred <= G14_RATIO { f.push(format!("G14 red arm ratio {rred:.2} not above {G14_RATIO}; the check does not see the strip")); }
    finish("G14", f);
}

// ---------------------------------------------------------------- G3-style resolve red arm (temporal_alpha 1)

/// With `clouds.temporal_alpha` 1 there is no history at all: only the 1/16 pixels marched this frame exist and the rest are empty, so the
/// default update shows as a lattice. The default must be far smoother (dots per 1000 sky px).
#[test]
#[ignore = "manual gate"]
fn resolve_history_matters() {
    let d = |sets: &[&str]| { let mut a = vec!["--hour", "19", "--settle", "300", "--set", "clouds.coverage_override=0.4"]; for s in sets { a.extend(["--set", s]); } g10_dots(&[still("hist", "sky_east", &a)]) };
    let (with, without) = (d(&[]), d(&["clouds.temporal_alpha=1.0"]));
    println!("history: dots per 1000 sky px with history {with:.3}, without (alpha 1) {without:.3}");
    finish("history", if without > 3.0 * with.max(0.01) { vec![] } else { vec![format!("alpha-1 arm {without:.3} not above 3x default {with:.3}; the resolve is not gated")] });
}
