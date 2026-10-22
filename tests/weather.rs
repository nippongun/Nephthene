//! Weather map gate G22 (headless; every shot is the `sky` example in `--shot` mode). Manual, #[ignore]:
//! `cargo test --release --test weather -- --ignored --nocapture --test-threads=1`. Metrics print before asserting.
//! Every threshold is a budget with a stated reason, not a fit to a measurement. See docs/weather_map.md.
mod common;
use common::*;
use image::RgbImage;

/// Mean |luma| difference of two frames over the whole image.
fn mean_diff(a: &RgbImage, b: &RgbImage) -> f32 {
    a.pixels().zip(b.pixels()).map(|(p, q)| (luma(p) - luma(q)).abs()).sum::<f32>() / (a.width() * a.height()) as f32
}

/// The CPU coverage at the camera, from the run's `MAP ... here coverage C` line.
fn here_coverage(log: &str) -> f32 {
    let line = log.lines().find(|l| l.starts_with("MAP shift")).unwrap_or_else(|| panic!("no MAP line in:\n{log}"));
    line.rsplit(' ').next().unwrap().parse().unwrap_or_else(|_| panic!("bad MAP line: {line}"))
}

// A shift that brings the clear trough of the built-in map overhead: its coverage swell (product of two sines) bottoms out near map
// (-67 km, -36 km), coverage ~0.25, against ~0.53 at the origin.
const SHIFT: &str = "map.start_shift_m=[67000.0,36000.0]";
const OFFSET: &str = "map.world_offset_m=[-67000.0,-36000.0]";
const STATIC: &str = "map.static_gpu=true";
/// Two renders of the same map frame may differ by the renderer's own run-to-run noise (measured each run: the default pose twice)
/// plus half a level of allowance for the driver.
const SAME_ALLOW: f32 = 0.5;
/// A real change of the weather overhead: with the coverage nearly halved, whole clouds give way to blue sky (tens of levels where
/// they do). 5 levels mean is the least a visible change of cloud cover can be.
const MOVED_MIN: f32 = 5.0;

/// G22: the clouds and the weather here ride the same shifted map. Shifting the map by S must render the same sky as moving the world
/// offset by -S (the GPU sees the shift), both must differ from the unshifted sky by a visible change (the shift moved real weather),
/// and the CPU coverage at the camera must agree across the two (the CPU sees it too). Pose sky_up at 480x270, 14 h.
/// Red arm: `map.static_gpu` (the clouds keep the unshifted map while the CPU weather moves): its sky must not match the offset sky.
#[test]
#[ignore = "manual gate"]
fn g22_map_shift_moves_the_clouds() {
    let run = |label: &str, sets: &[&str]| {
        let mut args = vec!["--size", "480x270", "--hour", "14"];
        for s in sets { args.extend(["--set", s]); }
        let (dir, log) = shoot_log(label, "sky_up", &args);
        (read(&dir, "sky_up"), here_coverage(&log))
    };
    let (base, base_cov) = run("g22_base", &[]);
    let (base2, _) = run("g22_base2", &[]);
    let (shifted, shifted_cov) = run("g22_shift", &[SHIFT]);
    let (offset, offset_cov) = run("g22_offset", &[OFFSET]);
    let (red, red_cov) = run("g22_red", &[SHIFT, STATIC]);
    let noise = mean_diff(&base, &base2);
    let same = mean_diff(&shifted, &offset);
    let moved = mean_diff(&shifted, &base);
    let red_same = mean_diff(&red, &offset);
    let budget = noise + SAME_ALLOW;
    println!("G22: run-to-run noise {noise:.3}; shift vs offset {same:.3} (budget {budget:.3}); shift vs unshifted {moved:.2} (min {MOVED_MIN}); red arm (static_gpu) vs offset {red_same:.2} (must exceed {budget:.3})");
    println!("G22: CPU coverage here: unshifted {base_cov:.3}, shift {shifted_cov:.3}, offset {offset_cov:.3}, red arm {red_cov:.3}");
    let mut f = vec![];
    if same > budget { f.push(format!("shift vs offset {same:.3} > {budget:.3}: the GPU does not see the shift")); }
    if moved < MOVED_MIN { f.push(format!("shift vs unshifted {moved:.2} < {MOVED_MIN}: the shift did not move the weather")); }
    if (shifted_cov - offset_cov).abs() > 0.002 { f.push(format!("CPU coverage {shifted_cov} vs {offset_cov}: the CPU does not see the shift")); }
    if red_same <= budget { f.push(format!("red arm {red_same:.3} <= {budget:.3}: the gate cannot tell a static GPU map")); }
    finish("G22", f);
}
