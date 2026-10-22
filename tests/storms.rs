//! Storm rain gates (headless; every shot is the `sky` example in `--shot` mode). Manual: `cargo test --release --test storms --
//! --ignored --nocapture --test-threads=1`. Metrics print before asserting; every threshold is a budget with a stated reason.
//! G25 reads the storm's rain at the camera (the `STORMHERE` line); G26 reads the rain shafts under a cell 8 km west (pixels).
mod common;
use common::*;
use image::RgbImage;

/// A mature intensity-1 cell centred on the camera: born 12.5 h, mid-life at 14 h (storms.clock_scale 0.25).
const G25_CELL: &str = r#"storms.authored=[{"name":"over","kind":"cell","bearing_deg":0,"dist_m":0,"born_hour":12.5}]"#;

/// Looks up at `sky_up` at 14 h with the cell on the camera (`--timeout` 900 s: the llvmpipe shot runs past the 240 s default). Returns (precip, cover, sky) from the last `STORMHERE` line, and the log.
fn here(label: &str, extra: &[&str]) -> ((f32, f32, f32), String) {
    let mut args = vec!["--hour", "14.0", "--timeout", "900", "--set", G25_CELL, "--set", "storms.force=\"over\""];
    for s in extra { args.extend(["--set", *s]); }
    let (_, log) = shoot_log(label, "sky_up", &args);
    let line = log.lines().rev().find(|l| l.starts_with("STORMHERE precip ")).unwrap_or_else(|| panic!("{label}: no STORMHERE line\n{log}"));
    let t: Vec<&str> = line.split_whitespace().collect();
    let v = |i: usize| t[i].parse::<f32>().unwrap_or_else(|e| panic!("{label}: `{}`: {e}", t[i]));
    ((v(2), v(4), v(6)), log)
}

/// G25: the rain falls on the camera. A mature intensity-1 cell rains at full strength inside storms.fx.curtain_full (0.6) of its
/// core radius, and the camera is on its axis; the budget is precip >= 0.5, because half of full strength allows for the lifecycle
/// ramp. Red arm `storms.couple` false (storms that only draw clouds, no rain) must stay under the budget.
const G25_PRECIP: f32 = 0.5;

#[test]
#[ignore = "manual gate"]
fn g25_storm_rains_on_the_camera() {
    let ((precip, cover, sky), log) = here("g25_on", &[]);
    let ((red_precip, red_cover, red_sky), _) = here("g25_red", &["storms.couple=false"]);
    println!("G25 on: precip {precip:.2} cover {cover:.2} sky {sky:.2} (budget precip >= {G25_PRECIP})");
    println!("G25 red arm (storms.couple false): precip {red_precip:.2} cover {red_cover:.2} sky {red_sky:.2} (must be < {G25_PRECIP})");
    let mut f = vec![];
    if !log.contains("STORMS n=1") { f.push("no `STORMS n=1` in the log: the authored cell was not selected".to_string()); }
    if precip < G25_PRECIP { f.push(format!("precip {precip:.2} < {G25_PRECIP}: the storm does not rain on the camera")); }
    if red_precip >= G25_PRECIP { f.push(format!("red arm precip {red_precip:.2} >= {G25_PRECIP}: the storm rains without coupling")); }
    finish("G25", f);
}

/// G26 scene: the same cell 8 km west, looking west level from the ground (pose `storm_shaft`, horizon at row 270). The shafts fall
/// from the cloud base (clouds.bottom_m 1500 m) to the ground: at 8 km that spans 10.6 deg above the horizon, rows ~204..270 at
/// 75 deg fov, and the full-strength curtain (0.6 x the 6 km core) spans +-24 deg, columns ~322..638. The rectangle below is that
/// region with a margin (rows 215..262, columns 360..600, end exclusive). A curtain that changes under half of it by a visible
/// 4 levels is not a visible rain curtain, so the budget is 0.5. Red arm: `storms.couple` false (the reference itself); the
/// run-to-run noise (a second red-arm shot against the first) must stay under the same 0.5.
const G26_CELL: &str = r#"storms.authored=[{"name":"west","kind":"cell","bearing_deg":270,"dist_m":8000,"born_hour":12.5}]"#;
const G26_ROWS: std::ops::Range<u32> = 215..262;
const G26_COLS: std::ops::Range<u32> = 360..600;
const G26_LEVELS: f32 = 4.0;
const G26_SHARE: f32 = 0.5;

fn shaft(label: &str, couple: bool) -> RgbImage {
    let mut args = vec!["--hour", "14.0", "--timeout", "900", "--set", G26_CELL, "--set", "storms.force=\"west\""];
    if !couple { args.extend(["--set", "storms.couple=false"]); }
    let (dir, _) = shoot_log(label, "storm_shaft", &args);
    read(&dir, "storm_shaft")
}

/// Share of pixels in the G26 rectangle whose luma differs between `a` and `b` by more than G26_LEVELS.
fn shaft_share(a: &RgbImage, b: &RgbImage) -> f32 {
    let px: Vec<_> = G26_ROWS.clone().flat_map(|y| G26_COLS.clone().map(move |x| (x, y))).collect();
    let hit = px.iter().filter(|&&(x, y)| (luma(a.get_pixel(x, y)) - luma(b.get_pixel(x, y))).abs() > G26_LEVELS).count();
    hit as f32 / px.len() as f32
}

#[test]
#[ignore = "manual gate"]
fn g26_rain_shafts_under_a_storm() {
    let on = shaft("g26_on", true);
    let red = shaft("g26_red", false);
    let red2 = shaft("g26_red2", false);
    let (score, noise) = (shaft_share(&on, &red), shaft_share(&red2, &red));
    println!("G26 shaft share (rect rows 215..262, cols 360..600) changed by > 4 levels: on vs red {score:.4} (budget >= {G26_SHARE})");
    println!("G26 red arm run-to-run noise (red2 vs red): {noise:.4} (must be < {G26_SHARE})");
    let mut f = vec![];
    if score < G26_SHARE { f.push(format!("{score:.4} < {G26_SHARE}: the rain shafts do not change the curtain region")); }
    if noise >= G26_SHARE { f.push(format!("red arm noise {noise:.4} >= {G26_SHARE}: run-to-run noise passes the gate")); }
    finish("G26", f);
}
