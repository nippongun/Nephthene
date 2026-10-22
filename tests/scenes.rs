//! Rain and storm scene gates (headless; every shot is the `sky` example in `--shot` mode). Manual gates are #[ignore]:
//! `cargo test --release --test scenes -- --ignored --nocapture --test-threads=1` runs both. Metrics print before asserting.
//! The two scenes are also the look-review renders for rain and storms: their PNGs land in shots/ (rain_on, rain_dry, rain_red,
//! storm_ref, storm_on, storm_red). Every threshold is a budget with a stated reason, not a fit to a measurement.
//! Red arms: rain drawn invisibly (`rain.streak_alpha` and `rain.splash_alpha` 0) and a storm with `storms.sky_scale` 0.1.
//! Both checks are RED on the current defaults, on purpose (see docs/bugs.md): forced full rain shows only a handful of streaks
//! (~0.1% of the sky vs the 1% budget, a look call), and the 40 km cell renders as a ~10 px horizon lens, not a tower (Phase 2).
//! The budgets are not lowered to pass; they stay as the target until the look or the storm fields change.
//! Both metrics read sky rows only (rows 0 .. h * 50 / 100): the horizon sits near row 297 of 540, and below it wet ground (rain)
//! and the storm's horizon smudge dominate the change.
mod common;
use common::*;
use image::RgbImage;
use std::process::Command;

/// Share of pixels in the top `pct`% rows whose luma differs by more than `levels` between two frames.
/// Callers pass 50: the sky above the horizon, not the ground below it.
fn share(a: &RgbImage, b: &RgbImage, pct: u32, levels: f32) -> f32 {
    let rows = a.height() * pct / 100;
    let n = rows * a.width();
    let k = (0..rows).flat_map(|y| (0..a.width()).map(move |x| (x, y)))
        .filter(|&(x, y)| (luma(a.get_pixel(x, y)) - luma(b.get_pixel(x, y))).abs() > levels).count();
    k as f32 / n as f32
}

/// One shot run at `hour` with `sets` on top, keeping its log (the STORMS line). Returns the frame and the log.
fn frame(label: &str, pose: &str, hour: &str, sets: &[&str]) -> (RgbImage, String) {
    let dir = root().join("shots").join(label);
    let _ = std::fs::remove_dir_all(&dir);
    let mut cmd = Command::new(example());
    cmd.args(["--shot", pose, "--out"]).arg(&dir).args(["--hour", hour]);
    for s in sets { cmd.args(["--set", s]); }
    let out = cmd.env("CLAUDECODE", "1").output().expect("run the sky example");
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{label}: `--shot {pose}` exit {:?}\n{log}", out.status.code());
    (read(&dir, pose), log)
}

// ---------------------------------------------------------------- rain

/// Streaks at full intensity change at least 1% of the sky (top 50%, sky rows only; the wet ground is left out). Looking east at
/// ground level, 17.5 h. Red arm: streaks drawn invisibly must read below the budget (the metric does not count falling drops it cannot see).
const RAIN_BUDGET: f32 = 0.01;

#[test]
#[ignore = "manual gate"]
fn rain_scene() {
    let (wet, _) = frame("rain_on", "rain_near", "17.5", &["rain.force=1.0"]);
    let (dry, _) = frame("rain_dry", "rain_near", "17.5", &["rain.force=0.0"]);
    let (red, _) = frame("rain_red", "rain_near", "17.5", &["rain.force=1.0", "rain.streak_alpha=0.0", "rain.splash_alpha=0.0"]);
    let (score, red_score) = (share(&wet, &dry, 50, 2.0), share(&red, &dry, 50, 2.0));
    println!("rain: share of sky px changed by > 2 levels: rain {score:.4} (budget >= {RAIN_BUDGET}); red arm (streaks invisible) {red_score:.4} (must be < {RAIN_BUDGET})");
    let mut f = vec![];
    if score < RAIN_BUDGET { f.push(format!("rain: {score:.4} < {RAIN_BUDGET}: streaks do not read in the sky")); }
    if red_score >= RAIN_BUDGET { f.push(format!("rain red arm {red_score:.4} >= {RAIN_BUDGET}: invisible streaks count")); }
    finish("rain scene", f);
}

// ---------------------------------------------------------------- storms

/// An authored cumulonimbus cell 40 km west, mature at 14 h (storm clock: born 12.5 h, 1.5 h = 1350 of 2700 storm seconds at
/// clock_scale 0.25, mid-life; 13.0 h gave one third). Looking west at ground level. Its sky (top 50%, sky rows only: the horizon
/// smudge is left out) must change at least 5% (more than 4 levels) against the same frame without storms: a cell that spans tens
/// of degrees. Red arm: the same cell with `storms.sky_scale` 0.1 (towers a tenth as tall: the flat-lens look) must read below the
/// budget. The authored `intensity` is not used: the render ignores it (docs/bugs.md).
const STORM_BUDGET: f32 = 0.05;
const STORM_CELL: &str = r#"[{"name":"scene_cell","kind":"cell","bearing_deg":270,"dist_m":40000,"born_hour":12.5}]"#;

#[test]
#[ignore = "manual gate"]
fn storm_scene() {
    let cell = format!("storms.authored={STORM_CELL}");
    let (reference, _) = frame("storm_ref", "storm_west", "14.0", &[]);
    let (storm, log) = frame("storm_on", "storm_west", "14.0", &[&cell, "storms.force=\"scene_cell\""]);
    let (red, _) = frame("storm_red", "storm_west", "14.0", &[&cell, "storms.force=\"scene_cell\"", "storms.sky_scale=0.1"]);
    let (score, red_score) = (share(&storm, &reference, 50, 4.0), share(&red, &reference, 50, 4.0));
    println!("storm: share of sky px changed by > 4 levels: storm {score:.4} (budget >= {STORM_BUDGET}); red arm (sky_scale 0.1) {red_score:.4} (must be < {STORM_BUDGET})");
    let mut f = vec![];
    if !log.contains("STORMS n=1") { f.push("storm: the log has no `STORMS n=1`: the authored cell was not selected".to_string()); }
    if score < STORM_BUDGET { f.push(format!("storm: {score:.4} < {STORM_BUDGET}: the mature cell does not change the sky")); }
    if red_score >= STORM_BUDGET { f.push(format!("storm red arm {red_score:.4} >= {STORM_BUDGET}: a flat (sky_scale 0.1) cell changes the sky")); }
    finish("storm scene", f);
}
