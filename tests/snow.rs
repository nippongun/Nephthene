//! Snow gate (headless; every shot is the `sky` example in `--shot` mode). Manual: `cargo test --release --test snow --
//! --ignored --nocapture --test-threads=1`. Metrics print before asserting; every threshold is a budget with a stated reason.
//! G27: cold air turns the rain streaks into round flakes.
mod common;
use common::*;
use image::RgbImage;

/// Annual mean -30 C: at the default clock (day 200, 17.5 h) the ground air is far below `air.snow_c`, so the air state's
/// precip_kind is 1 (snow).
const COLD: &str = "air.mean_c=-30.0";

/// Looks east at ground level, 17.5 h, with no wind (the rain streaks hang vertical) and the cold air, plus `sets`. Returns the
/// frame and the log (its AIR line carries the precip_kind).
fn shot(label: &str, sets: &[&str]) -> (RgbImage, String) {
    let mut args = vec!["--hour", "17.5", "--timeout", "900", "--set", "wind.speed_mps=0.0", "--set", COLD];
    for s in sets { args.extend(["--set", *s]); }
    let (dir, log) = shoot_log(label, "rain_near", &args);
    (read(&dir, "rain_near"), log)
}

/// Anisotropy of the change between `a` and `b` over the sky (top 50% rows, the wet ground is left out). D(x,y) = |luma(a) - luma(b)|.
/// Returns (ratio, share): ratio = sum |D(x+1,y) - D(x,y)| / max(sum |D(x,y+1) - D(x,y)|, 1e-6), the horizontal over the vertical
/// gradient of the particle layer; share = fraction of the sky pixels with D > 2.0.
fn aniso(a: &RgbImage, b: &RgbImage) -> (f32, f32) {
    let (w, rows) = (a.width(), a.height() * 50 / 100);
    let d = |x: u32, y: u32| (luma(a.get_pixel(x, y)) - luma(b.get_pixel(x, y))).abs();
    let (mut horiz, mut vert, mut hit) = (0f64, 0f64, 0usize);
    for y in 0..rows {
        for x in 0..w {
            if d(x, y) > 2.0 { hit += 1; }
            if x + 1 < w { horiz += (d(x + 1, y) - d(x, y)).abs() as f64; }
            if y + 1 < rows { vert += (d(x, y + 1) - d(x, y)).abs() as f64; }
        }
    }
    ((horiz / vert.max(1e-6)) as f32, hit as f32 / (rows * w) as f32)
}

/// A round sprite has equal horizontal and vertical gradients (ratio 1); a vertical streak many times longer than wide has a ratio
/// far above 1. A particle layer under 2 does not read as streaks.
const G27_ROUND: f32 = 2.0;
/// The rain scene's budget (tests/scenes.rs RAIN_BUDGET): precipitation that changes under 1% of the sky does not read.
const G27_SHARE: f32 = 0.01;
/// The snow end of the sleet band (crates/nephthene_core/src/air.rs tests use 0.9).
const G27_KIND: f32 = 0.9;

#[test]
#[ignore = "manual gate"]
fn g27_cold_air_falls_as_snow() {
    let (snow, log) = shot("g27_on", &["rain.force=1.0"]);
    let (red, _) = shot("g27_red", &["rain.force=1.0", "rain.kind_force=0.0"]);
    let (dry, _) = shot("g27_dry", &["rain.force=0.0"]);
    let air = log.lines().find(|l| l.starts_with("AIR ")).unwrap_or_else(|| panic!("g27_on: no AIR line\n{log}"));
    let kind: f32 = air.split_whitespace().skip_while(|t| *t != "precip_kind").nth(1)
        .and_then(|t| t.parse().ok()).unwrap_or_else(|| panic!("g27_on: no precip_kind in `{air}`\n{log}"));
    let (ratio, share) = aniso(&snow, &dry);
    let (red_ratio, red_share) = aniso(&red, &dry);
    println!("G27 air precip_kind {kind:.2} (budget >= {G27_KIND})");
    println!("G27 snow: ratio {ratio:.2} (budget < {G27_ROUND:.1}), sky share {share:.4} (budget >= {G27_SHARE})");
    println!("G27 red arm (rain.kind_force 0): ratio {red_ratio:.2} (must be >= {G27_ROUND:.1}), share {red_share:.4}");
    let mut f = vec![];
    if kind < G27_KIND { f.push(format!("precip_kind {kind:.2} < {G27_KIND}: the cold air does not give snow")); }
    if ratio >= G27_ROUND { f.push(format!("ratio {ratio:.2} >= {G27_ROUND}: the cold-air particles still read as streaks")); }
    if share < G27_SHARE { f.push(format!("share {share:.4} < {G27_SHARE}: the flakes do not read in the sky")); }
    if red_ratio < G27_ROUND { f.push(format!("red arm ratio {red_ratio:.2} < {G27_ROUND}: forced rain does not read as streaks, so the metric cannot tell them apart")); }
    finish("G27", f);
}
