//! Snow cover gate (headless; every shot is the `sky` example in `--shot` mode). Manual: `cargo test --release --test snow_cover --
//! --ignored --nocapture --test-threads=1`. Metrics print before asserting; every threshold is a budget with a stated reason.
//! G29: snow covers the ground.
mod common;
use common::*;
use image::RgbImage;

/// Annual mean -30 C: at the default clock (day 200, 17.5 h) the ground air is far below `air.snow_c`, so the air state's
/// precip_kind is 1 (snow).
const COLD: &str = "air.mean_c=-30.0";

/// Looks east at ground level, 17.5 h, with no wind and no ground fog, the cold air, plus `sets`. Returns the frame and the log
/// (its AIR line carries the precip_kind).
fn shot(label: &str, sets: &[&str]) -> (RgbImage, String) {
    let mut args = vec!["--hour", "17.5", "--timeout", "900", "--set", "wind.speed_mps=0.0", "--set", "ground_fog.enabled=false", "--set", COLD];
    for s in sets { args.extend(["--set", *s]); }
    let (dir, log) = shoot_log(label, "rain_near", &args);
    (read(&dir, "rain_near"), log)
}

/// Mean luma over the ground in front of the camera (rows 70% .. 100% of the frame; the horizon is near row 297 of 540).
fn ground_luma(a: &RgbImage) -> f32 {
    let (w, h) = (a.width(), a.height());
    let (y0, n) = (h * 70 / 100, ((h - h * 70 / 100) * w) as f64);
    let mut sum = 0f64;
    for y in y0..h { for x in 0..w { sum += luma(a.get_pixel(x, y)) as f64; } }
    (sum / n) as f32
}

/// Snow (albedo ~0.9) over the brown ground (~0.5 x 0.46) is far brighter; a mean rise under 20 of 255 levels over the near
/// ground is not a white ground.
const G29_RISE: f32 = 20.0;
/// The snow end of the sleet band (crates/nephthene_core/src/air.rs tests use 0.9).
const G29_KIND: f32 = 0.9;

#[test]
#[ignore = "manual gate"]
fn g29_snow_covers_the_ground() {
    let (snow, log) = shot("g29_on", &["rain.force=1.0"]);
    let (bare, _) = shot("g29_bare", &["rain.force=0.0"]);
    let (red, _) = shot("g29_red", &["rain.force=1.0", "rain.cover_force=0.0"]);
    let air = log.lines().find(|l| l.starts_with("AIR ")).unwrap_or_else(|| panic!("g29_on: no AIR line\n{log}"));
    let kind: f32 = air.split_whitespace().skip_while(|t| *t != "precip_kind").nth(1)
        .and_then(|t| t.parse().ok()).unwrap_or_else(|| panic!("g29_on: no precip_kind in `{air}`\n{log}"));
    let (on, off, r) = (ground_luma(&snow), ground_luma(&bare), ground_luma(&red));
    println!("G29 air precip_kind {kind:.2} (budget >= {G29_KIND})");
    println!("G29 ground luma: snow {on:.1}, bare {off:.1}, rise {:.1} (budget >= {G29_RISE})", on - off);
    println!("G29 red arm (rain.cover_force 0): {r:.1}, rise {:.1} (must be < {G29_RISE})", r - off);
    let mut f = vec![];
    if kind < G29_KIND { f.push(format!("precip_kind {kind:.2} < {G29_KIND}: the cold air does not give snow")); }
    if on - off < G29_RISE { f.push(format!("snow does not cover the ground: rise {:.1} < {G29_RISE}", on - off)); }
    if r - off >= G29_RISE { f.push(format!("red arm: the ground whitens without snow cover: rise {:.1} >= {G29_RISE}", r - off)); }
    finish("G29", f);
}
