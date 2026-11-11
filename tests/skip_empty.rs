//! G47 (headless; every shot is the `sky` example in `--shot` mode). Manual: `cargo build --release --example sky && cargo test
//! --release --test skip_empty -- --ignored --nocapture --test-threads=1`. `clouds.skip_empty` (the density lookup's early return) must
//! not change the picture: its saving is only fair if the pixels are the same. Its cost half is `cost_clouds_skip_empty` (tests/cost.rs).
mod common;
use common::*;
use image::RgbImage;

/// Noon, clouds from the default map, no storms or ground fog, frozen clock, plus `sets`.
fn shot(label: &str, pose: &str, sets: &[&str]) -> RgbImage {
    let mut args = vec!["--hour", "12.0", "--size", "960x540", "--timeout", "900", "--set", "storms.enabled=false", "--set", "ground_fog.enabled=false"];
    for s in sets { args.extend(["--set", *s]); }
    read(&shoot(label, pose, &args), pose)
}

/// Largest channel difference between two frames, in 8-bit levels.
fn max_diff(a: &RgbImage, b: &RgbImage) -> u8 {
    a.pixels().zip(b.pixels()).flat_map(|(p, q)| (0..3).map(move |c| p[c].abs_diff(q[c]))).max().unwrap_or(0)
}

/// One level: the early return compares a bound that rounds differently from the full lookup, so a sample within float rounding of the
/// coverage threshold may fall either way; that is far under one 8-bit level, and a frozen-clock shot repeats bit for bit.
const G47_LEVELS: u8 = 1;

/// Two views (toward the horizon, and up). Red arm: one march step fewer (`clouds.steps` 71), a change no eye sees, must read above the
/// budget, or the comparison could not see a real difference.
#[test]
#[ignore = "manual gate"]
fn g47_skip_empty_changes_no_pixel() {
    let mut f = vec![];
    for pose in ["sky_east", "sky_up"] {
        let on = shot(&format!("g47_on_{pose}"), pose, &[]);
        let off = shot(&format!("g47_off_{pose}"), pose, &["clouds.skip_empty=false"]);
        let red = shot(&format!("g47_red_{pose}"), pose, &["clouds.steps=71"]);
        let (d, r) = (max_diff(&on, &off), max_diff(&on, &red));
        println!("G47 {pose}: largest difference on vs off {d} levels (budget <= {G47_LEVELS}); red arm (steps 71) {r}");
        if d > G47_LEVELS { f.push(format!("{pose}: skip_empty changed the picture by {d} levels")); }
        if r <= G47_LEVELS { f.push(format!("{pose}: red arm {r} <= {G47_LEVELS}: the comparison cannot see a small change")); }
    }
    finish("G47", f);
}
