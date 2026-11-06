//! Per-pass GPU cost gates (headless; every run is the `sky` example in `--shot` mode). Manual gates are #[ignore]:
//! `cargo test --release --test cost -- --ignored --nocapture --test-threads=1` (one at a time: parallel GPU work corrupts timing).
//! Timing is the `PASSMS` line `clouds.pass_timing` prints: mean ms per frame over the last 60 frames of a run (`--settle 200` gives three
//! such means; the last one excludes pipeline warm-up).
//! Every gate skips on a software renderer (lavapipe/llvmpipe has no meaningful GPU timing) unless NEPHTHENE_COST_ON_CPU=1: then it renders
//! and prints but does not assert.
//! Budgets are the roadmap's proposed split (owner's call): clouds march+resolve 1.2 ms, cloud shadow 0.2, rain shafts 0.3, rain FX 0.3
//! (ground fog 0.8 is Phase 0's gate). Retina (2880x1620, ~2.2x the pixels) is met through each pass's render_scale, not a second budget:
//! the Retina runs set `clouds.render_scale` to the default 0.5 x 1920/2880 (0.3333) so they march the same pixel count. Rain FX has no
//! render_scale, so its Retina run is at full resolution under the same budget (a failure there means rain needs a scale knob).
mod common;
use common::*;
use std::{collections::HashMap, process::Command, sync::OnceLock};

/// True on a software renderer. One 64x36 probe run, cached: its log names the adapter (`device_type: Cpu`).
fn software() -> bool {
    static SW: OnceLock<bool> = OnceLock::new();
    *SW.get_or_init(|| {
        let out = Command::new(example()).args(["--shot", "cost", "--out"]).arg(root().join("shots/cost_probe"))
            .args(["--size", "64x36", "--settle", "2"]).env("CLAUDECODE", "1").output().expect("run the sky example");
        let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        log.contains("device_type: Cpu")
    })
}

/// The last `PASSMS` line of a log, if any.
fn passms_line(log: &str) -> Option<&str> { log.lines().rev().find(|l| l.starts_with("PASSMS ")) }

/// Mean ms per pass per frame, from the last `PASSMS` line of a log. Panics with the log tail when there is none.
fn passms(log: &str) -> HashMap<String, f64> {
    let line = passms_line(log).unwrap_or_else(|| {
        let start = log.lines().count().saturating_sub(40);
        panic!("no PASSMS line in the log; tail:\n{}", log.lines().skip(start).collect::<Vec<_>>().join("\n"))
    });
    line["PASSMS ".len()..].split(" | ").map(|kv| {
        let (name, ms) = kv.rsplit_once(' ').expect("PASSMS entry is `name value`");
        (name.to_string(), ms.parse().expect("PASSMS value is a number"))
    }).collect()
}

/// One timed run of the cost pose: pass timing on, the cloud coverage and shafts fixed, `sets` on top. Retina sizing when `retina`.
/// Prints the run's PASSMS line and returns its means.
fn run(label: &str, sets: &[&str], retina: bool) -> HashMap<String, f64> {
    let mut a = vec!["--hour", "17.5", "--settle", "200", "--set", "clouds.pass_timing=true", "--set", "clouds.coverage_override=0.6", "--set", "clouds.shaft_scale=1.0"];
    if retina { a.extend(["--size", "2880x1620", "--set", "clouds.render_scale=0.3333"]); }
    for s in sets { a.extend(["--set", s]); }
    let dir = root().join("shots").join(label);
    let _ = std::fs::remove_dir_all(&dir);
    let out = Command::new(example()).args(["--shot", "cost", "--out"]).arg(&dir).args(&a).env("CLAUDECODE", "1").output().expect("run the sky example");
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{label}: `--shot cost` exit {:?}\n{log}", out.status.code());
    let m = passms(&log);
    println!("COST {label}: {}", passms_line(&log).unwrap_or_default());
    m
}

#[test]
fn passms_parses() {
    let log = "warm-up\nPASSMS shadow 0.100 | march 0.200 | resolve 0.300 | shaft_march 0.400 | shaft_resolve 0.500 | transparent 0.600\n\
               PASSMS shadow 0.123 | march 0.456 | resolve 0.010 | shaft_march 0.000 | shaft_resolve 0.000 | transparent 0.321\ntrailing\n";
    let m = passms(log);
    assert_eq!(m.len(), 6, "all six passes read: {m:?}");
    assert_eq!(m["shadow"], 0.123);
    assert_eq!(m["march"], 0.456);
    assert_eq!(m["resolve"], 0.010);
    assert_eq!(m["shaft_march"], 0.0);
    assert_eq!(m["shaft_resolve"], 0.0);
    assert_eq!(m["transparent"], 0.321);
}

// ---------------------------------------------------------------- cloud shadow

/// Budget 0.2 ms: the roadmap's split for the cloud shadow span (owner's call).
const SHADOW_BUDGET_MS: f64 = 0.2;

/// Red arm, must read above budget: every band every frame (`clouds.shadow_every` 1) at 2048 texels (`clouds.shadow_res`, 16x the 512
/// default texels): 64x the shadow work. If it reads at or under budget the gate cannot fail.
#[test]
#[ignore = "manual gate"]
fn cost_cloud_shadow() {
    if software() && std::env::var("NEPHTHENE_COST_ON_CPU").is_err() { println!("COST skipped: software renderer"); return; }
    let shadow_1080 = run("cost_shadow_1080", &[], false)["shadow"];
    let shadow_retina = run("cost_shadow_retina", &[], true)["shadow"];
    let red = run("cost_shadow_red", &["clouds.shadow_every=1", "clouds.shadow_res=2048"], false)["shadow"];
    println!("COST cloud shadow: 1080p {shadow_1080:.3}, Retina {shadow_retina:.3} ms (budget {SHADOW_BUDGET_MS}); red arm {red:.3}");
    if software() { return; }
    let mut f = vec![];
    if shadow_1080 > SHADOW_BUDGET_MS { f.push(format!("1080p cloud shadow {shadow_1080:.3} ms > {SHADOW_BUDGET_MS}")); }
    if shadow_retina > SHADOW_BUDGET_MS { f.push(format!("Retina cloud shadow {shadow_retina:.3} ms > {SHADOW_BUDGET_MS}")); }
    if red <= SHADOW_BUDGET_MS { f.push(format!("red arm {red:.3} not above budget: the gate cannot fail, delete it and log it (roadmap rule)")); }
    finish("COST cloud shadow", f);
}

// ---------------------------------------------------------------- cloud shadow present

/// Budget 0.3 ms: a texture filter over the 512 x 512 cookie must stay under half of the shadow band pass it serves (about 0.6 ms on the
/// reference Mac).
const PRESENT_BUDGET_MS: f64 = 0.3;

/// The span reads about 0.58 ms on Metal whatever the pass does (1 to 18 texture reads per texel read the same), so the span alone is a
/// timer floor. The pass's own cost is the slope: (2048 span - 512 span) / 15, the span's growth per 512 x 512 of texels.
/// Red arm: `clouds.shadow_res` 2048 must read above the default, otherwise the timer does not see the pass and the slope is meaningless.
#[test]
#[ignore = "manual gate"]
fn cost_shadow_present() {
    if software() && std::env::var("NEPHTHENE_COST_ON_CPU").is_err() { println!("COST skipped: software renderer"); return; }
    let default = run("cost_present_512", &["clouds.coverage_override=0.3"], false)["shadow_present"];
    let big = run("cost_present_2048", &["clouds.coverage_override=0.3", "clouds.shadow_res=2048"], false)["shadow_present"];
    let slope = (big - default) / 15.0;
    println!("COST shadow present: {slope:.3} ms per 512^2 (budget {PRESENT_BUDGET_MS}); spans: 512 {default:.3}, red arm 2048 {big:.3}");
    if software() { return; }
    let mut f = vec![];
    if slope > PRESENT_BUDGET_MS { f.push(format!("shadow present {slope:.3} ms > {PRESENT_BUDGET_MS}")); }
    if big <= default { f.push(format!("red arm {big:.3} not above default {default:.3}: the gate cannot fail, delete it and log it (roadmap rule)")); }
    finish("COST shadow present", f);
}

// ---------------------------------------------------------------- cloud march + resolve

/// Budget 1.2 ms: the roadmap's split for cloud march plus resolve (owner's call).
const CLOUDS_BUDGET_MS: f64 = 1.2;

/// Red arm, must read above budget: `clouds.update_div` 1 (every pixel marched every frame, 16x the marched pixels per frame) at
/// `clouds.render_scale` 1.0 (4x the resolution): 64x the march.
#[test]
#[ignore = "manual gate"]
fn cost_clouds() {
    if software() && std::env::var("NEPHTHENE_COST_ON_CPU").is_err() { println!("COST skipped: software renderer"); return; }
    let clouds = |m: &HashMap<String, f64>| m["march"] + m["resolve"];
    let clouds_1080 = clouds(&run("cost_clouds_1080", &[], false));
    let clouds_retina = clouds(&run("cost_clouds_retina", &[], true));
    let red = clouds(&run("cost_clouds_red", &["clouds.update_div=1", "clouds.render_scale=1.0"], false));
    println!("COST cloud march+resolve: 1080p {clouds_1080:.3}, Retina {clouds_retina:.3} ms (budget {CLOUDS_BUDGET_MS}); red arm {red:.3}");
    if software() { return; }
    let mut f = vec![];
    if clouds_1080 > CLOUDS_BUDGET_MS { f.push(format!("1080p cloud march+resolve {clouds_1080:.3} ms > {CLOUDS_BUDGET_MS}")); }
    if clouds_retina > CLOUDS_BUDGET_MS { f.push(format!("Retina cloud march+resolve {clouds_retina:.3} ms > {CLOUDS_BUDGET_MS}")); }
    if red <= CLOUDS_BUDGET_MS { f.push(format!("red arm {red:.3} not above budget: the gate cannot fail, delete it and log it (roadmap rule)")); }
    finish("COST cloud march+resolve", f);
}

// ---------------------------------------------------------------- cloud edge LOD

/// Share 0.9: with `clouds.edge_lod` on, cloud march+resolve at 1080p costs at most 0.9 x the same run with it off. A saving under 10%
/// is within the noise of a 60-frame mean and would not pay for the softer edges.
const EDGE_LOD_SHARE: f64 = 0.9;

/// Edge LOD earns its cost. Red arm: on, with no reduction (`step_scale` 1, detail radii past the corners at 1.41), must read above
/// 0.9 x off; if it does not, the gate cannot fail: delete it and log it (roadmap rule).
#[test]
#[ignore = "manual gate"]
fn cost_clouds_edge_lod() {
    if software() && std::env::var("NEPHTHENE_COST_ON_CPU").is_err() { println!("COST skipped: software renderer"); return; }
    let clouds = |m: &HashMap<String, f64>| m["march"] + m["resolve"];
    let off = clouds(&run("cost_edge_off", &[], false));
    let on = clouds(&run("cost_edge_on", &["clouds.edge_lod.enabled=true"], false));
    let red = clouds(&run("cost_edge_red", &["clouds.edge_lod.enabled=true", "clouds.edge_lod.step_scale=1.0", "clouds.edge_lod.detail_r0=2.0", "clouds.edge_lod.detail_r1=2.1"], false));
    println!("COST cloud edge LOD: off {off:.3} ms, on {on:.3} ms (budget <= {EDGE_LOD_SHARE} x off = {:.3}); red arm (no reduction) {red:.3}", EDGE_LOD_SHARE * off);
    if software() { return; }
    let mut f = vec![];
    if on > EDGE_LOD_SHARE * off { f.push(format!("edge LOD on {on:.3} ms > {EDGE_LOD_SHARE} x off {off:.3}")); }
    if red <= EDGE_LOD_SHARE * off { f.push(format!("red arm {red:.3} not above {EDGE_LOD_SHARE} x off: the gate cannot fail, delete it and log it (roadmap rule)")); }
    finish("COST cloud edge LOD", f);
}

// ---------------------------------------------------------------- rain shafts

/// Budget 0.3 ms: the roadmap's split for the rain shafts (shaft_march plus shaft_resolve, owner's call).
const SHAFTS_BUDGET_MS: f64 = 0.3;

/// Red arm, must read above budget: `clouds.render_scale` 1.0 (4x the shaft pixels) and `shafts.steps` 320 (4x the steps): 16x the work.
#[test]
#[ignore = "manual gate"]
fn cost_shafts() {
    if software() && std::env::var("NEPHTHENE_COST_ON_CPU").is_err() { println!("COST skipped: software renderer"); return; }
    let shafts = |m: &HashMap<String, f64>| m["shaft_march"] + m["shaft_resolve"];
    let shafts_1080 = shafts(&run("cost_shafts_1080", &[], false));
    let shafts_retina = shafts(&run("cost_shafts_retina", &[], true));
    let red = shafts(&run("cost_shafts_red", &["clouds.render_scale=1.0", "shafts.steps=320.0"], false));
    println!("COST rain shafts: 1080p {shafts_1080:.3}, Retina {shafts_retina:.3} ms (budget {SHAFTS_BUDGET_MS}); red arm {red:.3}");
    if software() { return; }
    let mut f = vec![];
    if shafts_1080 > SHAFTS_BUDGET_MS { f.push(format!("1080p rain shafts {shafts_1080:.3} ms > {SHAFTS_BUDGET_MS}")); }
    if shafts_retina > SHAFTS_BUDGET_MS { f.push(format!("Retina rain shafts {shafts_retina:.3} ms > {SHAFTS_BUDGET_MS}")); }
    if red <= SHAFTS_BUDGET_MS { f.push(format!("red arm {red:.3} not above budget: the gate cannot fail, delete it and log it (roadmap rule)")); }
    finish("COST rain shafts", f);
}

// ---------------------------------------------------------------- rain FX

/// Budget 0.3 ms: the roadmap's split for the rain FX (the transparent pass with rain forced on minus the same frame with rain forced off,
/// owner's call). Rain FX has no render_scale: its Retina run is at full resolution.
const RAIN_BUDGET_MS: f64 = 0.3;

/// Red arm, must read above budget (1080p): `rain.max_streaks` 384000 and `rain.max_splashes` 96000, 16x the defaults (24000 / 6000).
#[test]
#[ignore = "manual gate"]
fn cost_rain() {
    if software() && std::env::var("NEPHTHENE_COST_ON_CPU").is_err() { println!("COST skipped: software renderer"); return; }
    let transparent = |m: &HashMap<String, f64>| m["transparent"];
    let off_1080 = transparent(&run("cost_rain_1080_off", &["rain.force=0.0"], false));
    let on_1080 = transparent(&run("cost_rain_1080_on", &["rain.force=1.0"], false));
    let off_retina = transparent(&run("cost_rain_retina_off", &["rain.force=0.0"], true));
    let on_retina = transparent(&run("cost_rain_retina_on", &["rain.force=1.0"], true));
    let red_on = transparent(&run("cost_rain_red_on", &["rain.force=1.0", "rain.max_streaks=384000", "rain.max_splashes=96000"], false));
    let (rain_1080, rain_retina, red) = (on_1080 - off_1080, on_retina - off_retina, red_on - off_1080);
    println!("COST rain FX: 1080p {rain_1080:.3}, Retina {rain_retina:.3} ms (budget {RAIN_BUDGET_MS}); red arm (16x streaks and splashes) {red:.3}");
    if software() { return; }
    let mut f = vec![];
    if rain_1080 > RAIN_BUDGET_MS { f.push(format!("1080p rain FX {rain_1080:.3} ms > {RAIN_BUDGET_MS}")); }
    if rain_retina > RAIN_BUDGET_MS { f.push(format!("Retina rain FX {rain_retina:.3} ms > {RAIN_BUDGET_MS}")); }
    if red <= RAIN_BUDGET_MS { f.push(format!("red arm {red:.3} not above budget: the gate cannot fail, delete it and log it (roadmap rule)")); }
    finish("COST rain FX", f);
}

/// Share 0.25: rain at intensity 0.05 costs at most a quarter of rain at intensity 1 (transparent span over rain forced off, 1080p).
/// The prefix meshes draw at most 2 x 0.05 = 10% of the quads at 0.05, so a quarter leaves room for the pass's fixed overhead.
const RAIN_SCALE_SHARE: f64 = 0.25;

/// Cost follows intensity: light rain draws a small prefix mesh. Red arm: `rain.draw_levels` 0 (every quad at any intensity, the old
/// cost) must read above the share; if it does not, vertex work was never the cost and the gate cannot fail: log it (roadmap rule).
#[test]
#[ignore = "manual gate"]
fn cost_rain_follows_intensity() {
    if software() && std::env::var("NEPHTHENE_COST_ON_CPU").is_err() { println!("COST skipped: software renderer"); return; }
    let transparent = |m: &HashMap<String, f64>| m["transparent"];
    let off = transparent(&run("cost_scale_off", &["rain.force=0.0"], false));
    let full = transparent(&run("cost_scale_full", &["rain.force=1.0"], false)) - off;
    let light = transparent(&run("cost_scale_light", &["rain.force=0.05"], false)) - off;
    let red = transparent(&run("cost_scale_red", &["rain.force=0.05", "rain.draw_levels=0"], false)) - off;
    println!("COST rain scale: intensity 1 {full:.3} ms, 0.05 {light:.3} ms (budget <= {RAIN_SCALE_SHARE} x full = {:.3}); red arm (draw_levels 0) {red:.3}", RAIN_SCALE_SHARE * full);
    if software() { return; }
    let mut f = vec![];
    if light > RAIN_SCALE_SHARE * full { f.push(format!("light rain {light:.3} ms > {RAIN_SCALE_SHARE} x full {full:.3}")); }
    if red <= RAIN_SCALE_SHARE * full { f.push(format!("red arm {red:.3} not above {RAIN_SCALE_SHARE} x full: the gate cannot fail, delete it and log it (roadmap rule)")); }
    finish("COST rain scale", f);
}

// ---------------------------------------------------------------- rain off switch

/// Noise band 0.05 ms (one sixth of the rain FX budget): the smallest rain cost this 60-frame mean is meant to resolve. A switched-off
/// rain must read within it of the scene with no rain; the forced-on arm must read beyond it. Untuned: the run-to-run spread on Apple
/// Silicon has not been measured yet.
const RAIN_OFF_BAND_MS: f64 = 0.05;

/// Off-switch gate, 1080p: `rain.enabled` false with `rain.force` 1 must cost the transparent span of the same scene with no rain at all
/// (`rain.enabled` false alone), within RAIN_OFF_BAND_MS. Red arm: `rain.enabled` true with the same force must read above the band.
#[test]
#[ignore = "manual gate"]
fn cost_rain_off_switch() {
    if software() && std::env::var("NEPHTHENE_COST_ON_CPU").is_err() { println!("COST skipped: software renderer"); return; }
    let transparent = |m: &HashMap<String, f64>| m["transparent"];
    let none = transparent(&run("cost_rain_switch_none", &["rain.enabled=false"], false));
    let off = transparent(&run("cost_rain_switch_off", &["rain.force=1.0", "rain.enabled=false"], false)) - none;
    let red = transparent(&run("cost_rain_switch_red", &["rain.force=1.0", "rain.enabled=true"], false)) - none;
    println!("COST rain off switch: forced off {off:.3} ms over no rain (band {RAIN_OFF_BAND_MS}); red arm forced on {red:.3}");
    if software() { return; }
    let mut f = vec![];
    if off > RAIN_OFF_BAND_MS { f.push(format!("forced rain with rain.enabled=false costs {off:.3} ms over no rain > {RAIN_OFF_BAND_MS}")); }
    if red <= RAIN_OFF_BAND_MS { f.push(format!("red arm {red:.3} not above band: the gate cannot fail, delete it and log it (roadmap rule)")); }
    finish("COST rain off switch", f);
}
