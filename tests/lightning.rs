//! Lightning gates (headless; every shot is the `sky` example in `--shot` mode). Manual: `cargo test --release --test lightning --
//! --ignored --nocapture --test-threads=1`. Metrics print before asserting; every threshold is a budget with a stated reason.
//! Scene: an authored mature cell 8 km west at 22 h (night), looking west from the ground (pose `storm_bolt`). The flash is held
//! at 5 ms age (`storms.lightning.force_age_s`, frozen clock) and strikes on the storm axis (`strike_r_frac` 0), so the channel
//! spans the cloud base (2800 m at 8 km: 19 deg, about 23% of the 540 rows at 75 deg fov) down to the horizon.
//! Reference frame: the same scene with `storms.lightning.enabled` false.
//! G30: the flash lights its cloud (cloud light on vs `cloud_gain` 0).
mod common;
use common::*;
use image::RgbImage;

const CELL: &str = r#"storms.authored=[{"name":"bolt_cell","kind":"cell","bearing_deg":270,"dist_m":8000,"born_hour":20.5}]"#;

fn frame(label: &str, sets: &[&str]) -> (RgbImage, String) {
    let mut args = vec!["--hour", "22.0", "--set", CELL, "--set", "storms.force=\"bolt_cell\"", "--set", "storms.lightning.enabled=true",
                        "--set", "storms.lightning.force_age_s=0.005", "--set", "storms.lightning.strike_r_frac=0.0"];
    for s in sets { args.extend(["--set", s]); }
    let (dir, log) = shoot_log(label, "storm_bolt", &args);
    (read(&dir, "storm_bolt"), log)
}

/// Share of image rows holding at least one pixel whose luma differs from `b` by more than `levels`.
fn rows_hit(a: &RgbImage, b: &RgbImage, levels: f32) -> f32 {
    let hit = (0..a.height()).filter(|&y| (0..a.width()).any(|x| (luma(a.get_pixel(x, y)) - luma(b.get_pixel(x, y))).abs() > levels)).count();
    hit as f32 / a.height() as f32
}

/// Mean luma of `a` minus `b` over the bottom quarter of the image (ground within a few metres of the camera).
fn ground_gain(a: &RgbImage, b: &RgbImage) -> f32 {
    let y0 = a.height() * 3 / 4;
    let px: Vec<_> = (y0..a.height()).flat_map(|y| (0..a.width()).map(move |x| (x, y))).collect();
    px.iter().map(|&(x, y)| luma(a.get_pixel(x, y)) - luma(b.get_pixel(x, y))).sum::<f32>() / px.len() as f32
}

/// G23: the channel reads as one line from the cloud base to the ground: the rows it lights (by more than 60 levels, a bright
/// channel on a night sky) cover at least half its projected span (23% of the rows): 11%. Half allows for the fade above the
/// base and the bends of the channel. The flash light is off in both arms so only the channel counts.
/// Red arm `min_px` 0: the 3 m channel is a tenth of a pixel wide at 8 km, so it breaks into scattered dots.
const BOLT_LEVELS: f32 = 60.0;
const BOLT_ROWS: f32 = 0.11;

#[test]
#[ignore = "manual gate"]
fn g23_bolt_reads_as_a_line() {
    let (reference, _) = frame("g23_ref", &["storms.lightning.enabled=false"]);
    let (on, log) = frame("g23_on", &["storms.lightning.light_lm=0.0"]);
    let (red, _) = frame("g23_red", &["storms.lightning.light_lm=0.0", "storms.lightning.min_px=0.0"]);
    let (score, red_score) = (rows_hit(&on, &reference, BOLT_LEVELS), rows_hit(&red, &reference, BOLT_LEVELS));
    println!("G23 rows lit by the channel: {score:.4} (budget >= {BOLT_ROWS}); red arm (min_px 0) {red_score:.4} (must be < {BOLT_ROWS})");
    let mut f = vec![];
    if !log.contains("LIGHTNING cg=true") { f.push("no `LIGHTNING cg=true` in the log: the forced flash did not start".to_string()); }
    if score < BOLT_ROWS { f.push(format!("{score:.4} < {BOLT_ROWS}: the channel does not read from base to ground")); }
    if red_score >= BOLT_ROWS { f.push(format!("red arm {red_score:.4} >= {BOLT_ROWS}: a sub-pixel channel passes")); }
    finish("G23", f);
}

/// G24: the flash lights the night landscape: the near ground (bottom quarter) brightens by at least 10 levels on average. 10
/// levels is about 4% of the display range, plainly visible, and twenty times the shots' run-to-run noise (~0.5 levels, golden pack).
/// Red arm `light_lm` 0 (the channel without its light) must stay under it.
const GROUND_LEVELS: f32 = 10.0;

#[test]
#[ignore = "manual gate"]
fn g24_flash_lights_the_ground() {
    let (reference, _) = frame("g24_ref", &["storms.lightning.enabled=false"]);
    let (on, _) = frame("g24_on", &[]);
    let (red, _) = frame("g24_red", &["storms.lightning.light_lm=0.0"]);
    let (score, red_score) = (ground_gain(&on, &reference), ground_gain(&red, &reference));
    println!("G24 near-ground gain: {score:.2} levels (budget >= {GROUND_LEVELS}); red arm (light_lm 0) {red_score:.2} (must be < {GROUND_LEVELS})");
    let mut f = vec![];
    if score < GROUND_LEVELS { f.push(format!("{score:.2} < {GROUND_LEVELS}: the flash does not light the ground")); }
    if red_score >= GROUND_LEVELS { f.push(format!("red arm {red_score:.2} >= {GROUND_LEVELS}: the ground brightens without the light")); }
    finish("G24", f);
}

/// Share of pixels in the top half of the image (the sky and the storm cloud) brighter in `a` than in `b` by more than `levels`.
fn sky_lit(a: &RgbImage, b: &RgbImage, levels: f32) -> f32 {
    let rows = a.height() / 2;
    let hit = (0..rows).flat_map(|y| (0..a.width()).map(move |x| (x, y))).filter(|&(x, y)| luma(a.get_pixel(x, y)) - luma(b.get_pixel(x, y)) > levels).count();
    hit as f32 / (rows * a.width()) as f32
}

/// G30: the flash lights its cloud. Against the same flash with the cloud light off (`cloud_gain` 0: channel and ground light are in
/// both arms), at least 10% of the top half brightens by more than 10 levels (G24's visibility). The cell's base spans 19 deg at 8 km
/// and its tower rises above the frame, so its lit body fills far more than a tenth of the sky rows. Red arm: `cloud_gain` 0 against
/// itself on a second run (run-to-run noise) must stay under it.
const CLOUD_LEVELS: f32 = 10.0;
const CLOUD_SHARE: f32 = 0.1;

#[test]
#[ignore = "manual gate"]
fn g30_flash_lights_the_cloud() {
    let (on, _) = frame("g30_on", &[]);
    let (off, _) = frame("g30_off", &["storms.lightning.cloud_gain=0.0"]);
    let (off2, _) = frame("g30_off2", &["storms.lightning.cloud_gain=0.0"]);
    let (score, noise) = (sky_lit(&on, &off, CLOUD_LEVELS), sky_lit(&off2, &off, CLOUD_LEVELS));
    println!("G30 sky share lit by the flash (> {CLOUD_LEVELS} levels): {score:.4} (budget >= {CLOUD_SHARE}); red arm (cloud_gain 0, second run) {noise:.4} (must be < {CLOUD_SHARE})");
    let mut f = vec![];
    if score < CLOUD_SHARE { f.push(format!("{score:.4} < {CLOUD_SHARE}: the flash does not light its cloud")); }
    if noise >= CLOUD_SHARE { f.push(format!("red arm {noise:.4} >= {CLOUD_SHARE}: the metric passes without cloud light")); }
    finish("G30", f);
}
