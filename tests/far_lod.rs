//! Distant cloud LOD gate (headless; every shot is the `sky` example in `--shot` mode). Manual: `cargo test --release --test far_lod --
//! --ignored --nocapture --test-threads=1`. Metrics print before asserting; every threshold is a budget with a stated reason.
//! G40: `clouds.far_lod` changes the frame by little. Scenes: pose `cost` at 17.5 h (low sun, the far deck lit from behind) and pose
//! `sky_east` at noon, coverage 0.6, no storms, frozen clock and wind, god rays off (they are screen-space and spread a small sky change
//! over the ground), 1920x1080. The renders are deterministic: two shots of one arm read 0.000 apart (measured 2026-11-09, M5 Max).
//! The cost half is tests/cost.rs `cost_clouds_far_lod`.
mod common;
use common::*;
use image::RgbImage;

/// The fixed scene plus `sets`. Returns the frame.
fn shot(label: &str, pose: &str, hour: &str, sets: &[&str]) -> RgbImage {
    let mut args = vec!["--hour", hour, "--size", "1920x1080", "--timeout", "900", "--set", "clouds.coverage_override=0.6", "--set", "storms.enabled=false",
        "--set", "clock.minutes_per_second=0", "--set", "wind.speed_mps=0", "--set", "wind.gust=0", "--set", "rays.enabled=false"];
    for s in sets { args.extend(["--set", *s]); }
    read(&shoot(label, pose, &args), pose)
}

/// Mean |luma(a) - luma(b)| over the frame, and its 99th percentile.
fn diff(a: &RgbImage, b: &RgbImage) -> (f32, f32) {
    let mut d: Vec<f32> = a.pixels().zip(b.pixels()).map(|(p, q)| (luma(p) - luma(q)).abs()).collect();
    let mean = d.iter().sum::<f32>() / d.len() as f32;
    d.sort_by(f32::total_cmp);
    (mean, d[(d.len() - 1) * 99 / 100])
}

/// Mean budget, 1 level: a frame-wide shift under one 8-bit level is below what a viewer sees as a change in a smooth sky.
const MEAN_ALLOW: f32 = 1.0;
/// p99 budget, 12 levels: the most-changed 1% of pixels (the far cloud band) may move by less than 5% of the range: a softer far
/// edge, not a cloud that appears or vanishes (that reads 30+ levels: the red arm).
const P99_ALLOW: f32 = 12.0;

#[test]
#[ignore = "manual gate"]
fn g40_far_lod_changes_little() {
    let mut f = vec![];
    for (pose, hour) in [("cost", "17.5"), ("sky_east", "12.0")] {
        let off = shot(&format!("g40_{pose}_off"), pose, hour, &[]);
        let on = shot(&format!("g40_{pose}_on"), pose, hour, &["clouds.far_lod.enabled=true"]);
        // Red arm: one long step from 2 km (a LOD far too coarse) must break both budgets' p99.
        let red = shot(&format!("g40_{pose}_red"), pose, hour, &["clouds.far_lod.enabled=true", "clouds.far_lod.start_m=2000", "clouds.far_lod.samples=1"]);
        let ((m, p), (rm, rp)) = (diff(&on, &off), diff(&red, &off));
        println!("G40 {pose}: on mean {m:.3} p99 {p:.1} (budgets {MEAN_ALLOW} / {P99_ALLOW}); red arm mean {rm:.3} p99 {rp:.1}");
        if m > MEAN_ALLOW { f.push(format!("{pose}: mean {m:.3} > {MEAN_ALLOW}")); }
        if p > P99_ALLOW { f.push(format!("{pose}: p99 {p:.1} > {P99_ALLOW}")); }
        if rp <= P99_ALLOW { f.push(format!("{pose}: red arm p99 {rp:.1} <= {P99_ALLOW}: the gate cannot fail")); }
    }
    finish("G40", f);
}
