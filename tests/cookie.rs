//! Cloud shadow cookie gates (headless; every shot is the `sky` example in `--shot` mode). Manual: `cargo test --release --test cookie --
//! --ignored --nocapture --test-threads=1`. Metrics print before asserting; every threshold is a budget with a stated reason.
//! Scene: pose `cookie_patch` (640x360, ground only, flat ground so ground luma is the cookie) at 17.5 h, live clock with a fixed camera
//! (`--glide 0,0`). Coverage 0.3, ground fog, storms, rays and sun occlusion off: the air's own cloud shadow changes every frame and
//! would hide the cookie.
//! G38a: the shown cookie moves every frame, not in 15 Hz steps. G38b: shadow edges are soft (a tent of one texel, not a staircase).
mod common;
use common::*;
use image::RgbImage;

/// The 8x8 blocks of the frame, the same grid the motion metric reduces to.
const BLOCK: u32 = 8;
/// Keep the 20 percent most active blocks: the shadow edges, where a cookie step shows. Untuned: the metric's own sample choice.
const ACTIVE_FRAC: f32 = 0.2;
/// The cycle length of `clouds.shadow_every` (default 4): the phase of the frame-to-frame motion.
const PHASES: usize = 4;
/// Motion spread evenly over the cycle gives 1; 2 means one frame still carries as much motion as two others, which is the staircase
/// the eye reads.
const STEP_RATIO_MAX: f32 = 2.0;
/// A tent of radius 1 texel about doubles the width of a one-texel bilinear ramp, so a real edge's slope roughly halves; 0.8 leaves room
/// for edges already softer than a texel.
const SOFT_RATIO_MAX: f32 = 0.8;
/// Below this the frame has no edge to measure (99.9th percentile of the neighbour difference, in luma levels).
const EDGE_MIN: f32 = 2.0;

/// The shared scene: 17.5 h, live clock, fixed camera, coverage 0.3, no fog, storms, rays or sun occlusion, then `extra` (later `--set`s
/// win, so `extra` can override the base).
fn base<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut a = vec!["--hour", "17.5", "--live", "--glide", "0,0", "--timeout", "900", "--set", "clouds.coverage_override=0.3", "--set", "ground_fog.enabled=false", "--set", "storms.enabled=false", "--set", "rays.enabled=false", "--set", "sky.sun_occlusion=false"];
    a.extend_from_slice(extra);
    a
}

/// Mean luma per 8x8 block, row-major.
fn block_means(f: &RgbImage) -> Vec<f32> {
    let (bw, bh) = (f.width() / BLOCK, f.height() / BLOCK);
    let mut out = Vec::with_capacity((bw * bh) as usize);
    for by in 0..bh {
        for bx in 0..bw {
            let mut s = 0f32;
            for y in 0..BLOCK { for x in 0..BLOCK { s += luma(f.get_pixel(bx * BLOCK + x, by * BLOCK + y)); } }
            out.push(s / (BLOCK * BLOCK) as f32);
        }
    }
    out
}

/// Max phase over min phase of the mean |frame-to-frame| motion, over the most active blocks. 1 is even motion across the cycle.
fn phase_ratio(frames: &[RgbImage]) -> f32 {
    let means: Vec<Vec<f32>> = frames.iter().map(block_means).collect();
    let (nb, nt) = (means[0].len(), frames.len() - 1);
    let diff: Vec<Vec<f32>> = (0..nt).map(|t| (0..nb).map(|b| (means[t + 1][b] - means[t][b]).abs()).collect()).collect();
    let mut act: Vec<(f32, usize)> = (0..nb).map(|b| ((0..nt).map(|t| diff[t][b]).sum::<f32>() / nt as f32, b)).collect();
    act.sort_by(|a, b| b.0.total_cmp(&a.0));
    let keep = ((nb as f32) * ACTIVE_FRAC).ceil() as usize;
    let (mut sum, mut n) = ([0f64; PHASES], [0usize; PHASES]);
    for t in 0..nt {
        for &(_, b) in &act[..keep] { sum[t % PHASES] += diff[t][b] as f64; n[t % PHASES] += 1; }
    }
    let phase: Vec<f32> = (0..PHASES).map(|p| (sum[p] / n[p].max(1) as f64) as f32).collect();
    let max = phase.iter().cloned().fold(f32::MIN, f32::max);
    let min = phase.iter().cloned().fold(f32::MAX, f32::min);
    max / min.max(1e-6)
}

/// 99.9th percentile over the pixels of the larger of the right and the down neighbour luma difference.
fn edge_p999(img: &RgbImage) -> f32 {
    let (w, h) = img.dimensions();
    let l = |x: u32, y: u32| luma(img.get_pixel(x, y));
    let mut v = Vec::with_capacity(((w - 1) * (h - 1)) as usize);
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            let c = l(x, y);
            v.push((l(x + 1, y) - c).abs().max((l(x, y + 1) - c).abs()));
        }
    }
    v.sort_by(f32::total_cmp);
    let idx = ((v.len() as f64 * 0.999).ceil() as usize).max(1) - 1;
    v[idx.min(v.len() - 1)]
}

/// G38a: 41 frames at every frame (`--seq 41,1`) after a 120-frame settle. Red arm: `clouds.shadow_blend` false (the cookie steps every
/// `shadow_every` frames).
#[test]
#[ignore = "manual gate"]
fn g38_cookie_moves_every_frame() {
    let on = seq("g38a_on", "cookie_patch", 41, &base(&["--settle", "120", "--seq", "41,1"]));
    let red = seq("g38a_red", "cookie_patch", 41, &base(&["--set", "clouds.shadow_blend=false", "--settle", "120", "--seq", "41,1"]));
    let (r_on, r_red) = (phase_ratio(&on), phase_ratio(&red));
    println!("G38a motion phase ratio: on {r_on:.3} (budget <= {STEP_RATIO_MAX}), red arm (shadow_blend false) {r_red:.3} (must be > {STEP_RATIO_MAX})");
    let mut f = vec![];
    if r_on > STEP_RATIO_MAX { f.push(format!("cookie steps: phase ratio {r_on:.3} > {STEP_RATIO_MAX}")); }
    if r_red <= STEP_RATIO_MAX { f.push(format!("red arm phase ratio {r_red:.3} <= {STEP_RATIO_MAX}: the metric cannot see the step")); }
    finish("G38a", f);
}

/// G38b: one still at 1280x720 after a 120-frame settle, soft edges on (`clouds.shadow_soft_px` 1.0) and off (0.0, the red arm).
#[test]
#[ignore = "manual gate"]
fn g38_shadow_edges_are_soft() {
    let soft = still("g38b_soft", "cookie_patch", &base(&["--settle", "120", "--size", "1280x720", "--set", "clouds.shadow_soft_px=1.0"]));
    let hard = still("g38b_hard", "cookie_patch", &base(&["--settle", "120", "--size", "1280x720", "--set", "clouds.shadow_soft_px=0.0"]));
    let (p_soft, p_hard) = (edge_p999(&soft), edge_p999(&hard));
    println!("G38b edge 99.9th percentile: soft {p_soft:.3}, red arm (soft_px 0) {p_hard:.3} (budget <= {SOFT_RATIO_MAX} x red = {:.3})", SOFT_RATIO_MAX * p_hard);
    let mut f = vec![];
    if p_hard < EDGE_MIN { f.push(format!("no edge in frame: red arm {p_hard:.3} < {EDGE_MIN}")); }
    if p_soft > SOFT_RATIO_MAX * p_hard { f.push(format!("shadow edges still stepped: soft {p_soft:.3} > {SOFT_RATIO_MAX} x red {p_hard:.3}")); }
    finish("G38b", f);
}
