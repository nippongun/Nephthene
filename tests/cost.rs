//! GPU cost gates (headless; every run is the `sky` example in `--shot` mode). Manual gates are #[ignore]:
//! `cargo test --release --test cost -- --ignored --nocapture --test-threads=1` (one at a time: parallel GPU work corrupts timing).
//! Run them on mains power with no other GPU work (a game, Blender, a shader compile): macOS lowers the GPU clock on battery and other
//! work shares the GPU, which moved a frozen scene from 6 to 11 ms inside one run.
//! Timing is the `FRAMEMS` line `clouds.frame_timing` prints (src/timing.rs): the whole frame's GPU span, read back without blocking. A
//! feature's cost is the frame with it minus the frame without it. Per-pass spans (PASSMS) are not used: on Metal every pass's begin
//! timestamp fires at the frame's start and independent passes overlap, so a span is "frame start to this pass's end".
//! Each gate runs its arms round-robin REPS times on a frozen scene (clock, wind, storms off); an arm's reading is the median of its runs,
//! a run's value the median of its 60-frame window medians after the first (warm-up) window. Apple GPUs change clock with load, so runs of
//! one arm spread 0.2-1.5 ms even on a quiet machine on mains. A difference carries an uncertainty: the sum of its two arms' spreads (max -
//! min of their runs). A gate decides only when the difference with its uncertainty lies wholly on one side of the budget; otherwise it
//! fails as "undecided" (re-run, or raise REPS), never as a pass.
//! Every gate skips on a software renderer (lavapipe/llvmpipe has no meaningful GPU timing) unless NEPHTHENE_COST_ON_CPU=1: then it renders
//! and prints but does not assert.
//! Budgets are the roadmap's proposed split (owner's call): clouds march+resolve 1.2 ms, cloud shadow 0.2, rain shafts 0.3, rain FX 0.3
//! (ground fog 0.8 is Phase 0's gate). Retina (2880x1620, ~2.2x the pixels) is met through each pass's render_scale, not a second budget:
//! the Retina runs set `clouds.render_scale` to the default 0.5 x 1920/2880 (0.3333) so they march the same pixel count. Rain FX has no
//! render_scale, so its Retina run is at full resolution under the same budget (a failure there means rain needs a scale knob).
mod common;
use common::*;
use std::{process::Command, sync::OnceLock};

/// Runs per arm. Three gives a median that one disturbed run cannot move.
const REPS: usize = 3;

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

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n == 0 { f64::NAN } else if n % 2 == 1 { v[n / 2] } else { 0.5 * (v[n / 2 - 1] + v[n / 2]) }
}

/// A run's frame GPU time: the median of its FRAMEMS window medians, the first (warm-up) window dropped. Panics with the log tail when
/// fewer than two windows were printed.
fn framems(log: &str) -> f64 {
    let meds: Vec<f64> = log.lines().filter(|l| l.starts_with("FRAMEMS ")).map(|l| {
        let mut w = l.split_whitespace();
        while let Some(k) = w.next() { if k == "med" { return w.next().and_then(|v| v.parse().ok()).expect("FRAMEMS med value"); } }
        panic!("FRAMEMS line without med: {l}")
    }).collect();
    if meds.len() < 2 {
        let start = log.lines().count().saturating_sub(40);
        panic!("fewer than two FRAMEMS lines in the log; tail:\n{}", log.lines().skip(start).collect::<Vec<_>>().join("\n"));
    }
    median(meds[1..].to_vec())
}

/// One arm's reading: the median of its runs and their spread.
#[derive(Clone, Copy)]
struct Reading { ms: f64, spread: f64 }

/// One run of the cost pose on a frozen scene: frame timing on, the cloud coverage and shafts fixed, `sets` on top. Retina sizing when
/// `retina`. Returns the run's frame GPU time.
fn run(label: &str, sets: &[&str], retina: bool) -> f64 {
    let mut a = vec!["--hour", "17.5", "--settle", "300", "--set", "clouds.frame_timing=true", "--set", "clouds.coverage_override=0.6",
        "--set", "clouds.shaft_scale=1.0", "--set", "clock.minutes_per_second=0", "--set", "wind.speed_mps=0", "--set", "wind.gust=0",
        "--set", "storms.enabled=false"];
    if retina { a.extend(["--size", "2880x1620", "--set", "clouds.render_scale=0.3333"]); }
    for s in sets { a.extend(["--set", s]); }
    let dir = root().join("shots").join(label);
    let _ = std::fs::remove_dir_all(&dir);
    let out = Command::new(example()).args(["--shot", "cost", "--out"]).arg(&dir).args(&a).env("CLAUDECODE", "1").output().expect("run the sky example");
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{label}: `--shot cost` exit {:?}\n{log}", out.status.code());
    framems(&log)
}

/// Run every arm `(label, sets, retina)` REPS times round-robin (so a slow drift hits every arm alike) and return their readings.
fn arms(arms: &[(&str, &[&str], bool)]) -> Vec<Reading> {
    let mut runs = vec![Vec::new(); arms.len()];
    for _ in 0..REPS { for (i, (label, sets, retina)) in arms.iter().enumerate() { runs[i].push(run(label, sets, *retina)); } }
    arms.iter().zip(runs).map(|((label, ..), r)| {
        let spread = r.iter().copied().fold(f64::MIN, f64::max) - r.iter().copied().fold(f64::MAX, f64::min);
        let ms = median(r.clone());
        println!("COST {label}: {ms:.3} ms (runs {})", r.iter().map(|v| format!("{v:.3}")).collect::<Vec<_>>().join(" "));
        Reading { ms, spread }
    }).collect()
}

/// A measured cost and its uncertainty (ms).
#[derive(Clone, Copy)]
struct Cost { ms: f64, unc: f64 }
impl std::fmt::Display for Cost {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "{:.3} +- {:.3}", self.ms, self.unc) }
}
/// `with` minus `without`, uncertainty the sum of their spreads.
fn diff(with: Reading, without: Reading) -> Cost { Cost { ms: with.ms - without.ms, unc: with.spread + without.spread } }
/// A cost scaled by a constant (a slope, a share).
fn scale(c: Cost, k: f64) -> Cost { Cost { ms: c.ms * k, unc: c.unc * k.abs() } }

/// `what` must cost at most `limit`: pushes a failure when it is surely over, an "undecided" when the limit lies inside its uncertainty.
fn at_most(f: &mut Vec<String>, what: &str, c: Cost, limit: Cost) {
    let unc = c.unc + limit.unc;
    if c.ms - unc > limit.ms { f.push(format!("{what} {c} ms > {limit}")); }
    else if c.ms + unc > limit.ms { f.push(format!("undecided: {what} {c} ms against {limit} (re-run, or raise REPS)")); }
}
/// A red arm must surely cost more than `limit`, otherwise the gate cannot fail (or cannot tell).
fn red_above(f: &mut Vec<String>, red: Cost, limit: Cost) {
    let unc = red.unc + limit.unc;
    if red.ms + unc <= limit.ms { f.push(format!("red arm {red} not above {limit}: the gate cannot fail, delete it and log it (roadmap rule)")); }
    else if red.ms - unc <= limit.ms { f.push(format!("undecided: red arm {red} against {limit} (re-run, or raise REPS)")); }
}
/// An exact limit (a budget).
fn exact(ms: f64) -> Cost { Cost { ms, unc: 0.0 } }

/// True when the gate must skip: a software renderer without NEPHTHENE_COST_ON_CPU.
fn skip() -> bool {
    let skip = software() && std::env::var("NEPHTHENE_COST_ON_CPU").is_err();
    if skip { println!("COST skipped: software renderer"); }
    skip
}

#[test]
fn framems_parses() {
    let log = "warm-up\nFRAMEMS gpu 9.000 med 9.500 p10 8.0 max 12.0 | wall 10.0 | n 60\n\
               FRAMEMS gpu 6.0 med 6.100 p10 5.9 max 7.0 | wall 6.5 | n 60\n\
               FRAMEMS gpu 6.0 med 6.300 p10 5.9 max 7.0 | wall 6.5 | n 58\n\
               FRAMEMS gpu 6.0 med 6.200 p10 5.9 max 7.0 | wall 6.5 | n 60\ntrailing\n";
    assert_eq!(framems(log), 6.2, "median of the windows after the first");
    assert_eq!(median(vec![3.0, 1.0, 2.0, 10.0]), 2.5);
    let (mut f, c) = (vec![], |ms, unc| Cost { ms, unc });
    at_most(&mut f, "a", c(1.0, 0.1), exact(1.2));
    assert!(f.is_empty(), "clearly under passes: {f:?}");
    at_most(&mut f, "b", c(1.15, 0.1), exact(1.2));
    assert!(f[0].starts_with("undecided"), "budget inside the uncertainty is undecided: {f:?}");
    at_most(&mut f, "c", c(1.5, 0.1), exact(1.2));
    assert!(f[1].starts_with("c "), "clearly over fails: {f:?}");
    red_above(&mut f, c(1.0, 0.1), exact(1.2));
    assert!(f[2].contains("cannot fail"), "{f:?}");
}

// ---------------------------------------------------------------- cloud shadow

/// Budget 0.2 ms: the roadmap's split for the cloud shadow span (owner's call).
const SHADOW_BUDGET_MS: f64 = 0.2;
/// Work of the red arm over the default: every band every frame (`shadow_every` 1 vs 4) at 2048 texels (`shadow_res`, 16x the 512 default).
const SHADOW_RED_WORK: f64 = 64.0;

/// No config switch removes the shadow pass (the panel's toggle zeroes its strength only), so its cost is a slope: the frame with the red
/// arm (64x the shadow work) minus the default frame is 63x the default shadow cost. Red arm: that difference must read above the budget,
/// otherwise the timer does not see the pass and the gate cannot fail.
#[test]
#[ignore = "manual gate"]
fn cost_cloud_shadow() {
    if skip() { return; }
    let red_sets: &[&str] = &["clouds.shadow_every=1", "clouds.shadow_res=2048"];
    let r = arms(&[("cost_shadow_1080", &[], false), ("cost_shadow_red", red_sets, false), ("cost_shadow_retina", &[], true), ("cost_shadow_retina_red", red_sets, true)]);
    let red = diff(r[1], r[0]);
    let (s1080, retina) = (scale(red, 1.0 / (SHADOW_RED_WORK - 1.0)), scale(diff(r[3], r[2]), 1.0 / (SHADOW_RED_WORK - 1.0)));
    println!("COST cloud shadow: 1080p {s1080}, Retina {retina} ms (budget {SHADOW_BUDGET_MS}); red arm {red}");
    if software() { return; }
    let mut f = vec![];
    at_most(&mut f, "1080p cloud shadow", s1080, exact(SHADOW_BUDGET_MS));
    at_most(&mut f, "Retina cloud shadow", retina, exact(SHADOW_BUDGET_MS));
    red_above(&mut f, red, exact(SHADOW_BUDGET_MS));
    finish("COST cloud shadow", f);
}

// ---------------------------------------------------------------- cloud shadow present

/// Budget 0.3 ms: a texture filter over the 512 x 512 cookie must stay under half of the shadow band pass it serves (about 0.6 ms on the
/// reference Mac).
const PRESENT_BUDGET_MS: f64 = 0.3;

/// The present pass's own cost is a slope: (frame at `shadow_res` 2048 - frame at 512) / 15, the growth per 512 x 512 of texels. The
/// shadow pass grows with the texels too, so this reads high (both passes): a pass here is a pass for the present alone.
/// Red arm: 2048 must read surely above 512, otherwise the timer does not see the pass and the slope is meaningless.
#[test]
#[ignore = "manual gate"]
fn cost_shadow_present() {
    if skip() { return; }
    let r = arms(&[("cost_present_512", &["clouds.coverage_override=0.3"], false), ("cost_present_2048", &["clouds.coverage_override=0.3", "clouds.shadow_res=2048"], false)]);
    let red = diff(r[1], r[0]);
    let slope = scale(red, 1.0 / 15.0);
    println!("COST shadow present: {slope} ms per 512^2 (budget {PRESENT_BUDGET_MS}); red arm (2048 over 512) {red}");
    if software() { return; }
    let mut f = vec![];
    at_most(&mut f, "shadow present", slope, exact(PRESENT_BUDGET_MS));
    red_above(&mut f, red, exact(0.0));
    finish("COST shadow present", f);
}

// ---------------------------------------------------------------- clouds

/// Budget 1.4 ms: the roadmap's split for cloud march plus resolve (1.2) plus the cloud shadow (0.2), because turning clouds off removes
/// every cloud pass: march, resolve, composite, shadow and its present.
const CLOUDS_BUDGET_MS: f64 = 1.4;

/// Cost = frame with clouds (shafts off) minus frame with `clouds.enabled` false. Red arm, must read above budget: `clouds.update_div` 1
/// (16x the marched pixels per frame) at `clouds.render_scale` 1.0 (4x the resolution): 64x the march.
#[test]
#[ignore = "manual gate"]
fn cost_clouds() {
    if skip() { return; }
    let r = arms(&[
        ("cost_clouds_none", &["clouds.enabled=false"], false), ("cost_clouds_1080", &["clouds.shaft_scale=0"], false),
        ("cost_clouds_red", &["clouds.shaft_scale=0", "clouds.update_div=1", "clouds.render_scale=1.0"], false),
        ("cost_clouds_retina_none", &["clouds.enabled=false"], true), ("cost_clouds_retina", &["clouds.shaft_scale=0"], true),
    ]);
    let (c1080, red, retina) = (diff(r[1], r[0]), diff(r[2], r[0]), diff(r[4], r[3]));
    println!("COST clouds: 1080p {c1080}, Retina {retina} ms (budget {CLOUDS_BUDGET_MS}); red arm {red}");
    if software() { return; }
    let mut f = vec![];
    at_most(&mut f, "1080p clouds", c1080, exact(CLOUDS_BUDGET_MS));
    at_most(&mut f, "Retina clouds", retina, exact(CLOUDS_BUDGET_MS));
    red_above(&mut f, red, exact(CLOUDS_BUDGET_MS));
    finish("COST clouds", f);
}

// ---------------------------------------------------------------- cloud edge LOD

/// Share 0.9: with `clouds.edge_lod` on, the clouds' cost at 1080p is at most 0.9 x the same with it off. A saving under 10% would not pay
/// for the softer edges.
const EDGE_LOD_SHARE: f64 = 0.9;

/// Edge LOD earns its cost (clouds' cost = frame minus the frame with clouds off, shafts off in every arm). Red arm: on, with no reduction
/// (`step_scale` 1, detail radii past the corners), must read above 0.9 x off; if it does not, the gate cannot fail: delete it and log it.
#[test]
#[ignore = "manual gate"]
fn cost_clouds_edge_lod() {
    if skip() { return; }
    let r = arms(&[
        ("cost_edge_none", &["clouds.enabled=false"], false), ("cost_edge_off", &["clouds.shaft_scale=0"], false),
        ("cost_edge_on", &["clouds.shaft_scale=0", "clouds.edge_lod.enabled=true"], false),
        ("cost_edge_red", &["clouds.shaft_scale=0", "clouds.edge_lod.enabled=true", "clouds.edge_lod.step_scale=1.0", "clouds.edge_lod.detail_r0=2.0", "clouds.edge_lod.detail_r1=2.1"], false),
    ]);
    // Every arm shares the clouds-off frame, so its spread cancels between on and off: compare the frames directly.
    let (off, on, red) = (diff(r[1], r[0]), diff(r[2], r[0]), diff(r[3], r[0]));
    let limit = Cost { ms: r[0].ms + EDGE_LOD_SHARE * off.ms, unc: r[0].spread + EDGE_LOD_SHARE * r[1].spread };
    println!("COST cloud edge LOD: off {off}, on {on} ms (budget <= {EDGE_LOD_SHARE} x off); red arm (no reduction) {red}");
    if software() { return; }
    let mut f = vec![];
    at_most(&mut f, "edge LOD on (frame)", Cost { ms: r[2].ms, unc: r[2].spread }, limit);
    red_above(&mut f, Cost { ms: r[3].ms, unc: r[3].spread }, limit);
    finish("COST cloud edge LOD", f);
}

// ---------------------------------------------------------------- distant cloud LOD

/// Share 0.7: with `clouds.far_lod` on, the clouds' cost at 1080p is at most 0.7 x the same with it off. A saving under 30% would not
/// pay for a change G40 lets reach 12 levels in the far band.
const FAR_LOD_SHARE: f64 = 0.7;

/// The far LOD earns its change (clouds' cost = frame minus the frame with clouds off, shafts off in every arm). Red arm: on, with the
/// start at infinity (`start_m` 1e9, the full march), must read above 0.7 x off; if it does not, the gate cannot fail.
#[test]
#[ignore = "manual gate"]
fn cost_clouds_far_lod() {
    if skip() { return; }
    let r = arms(&[
        ("cost_far_none", &["clouds.enabled=false"], false), ("cost_far_off", &["clouds.shaft_scale=0"], false),
        ("cost_far_on", &["clouds.shaft_scale=0", "clouds.far_lod.enabled=true"], false),
        ("cost_far_red", &["clouds.shaft_scale=0", "clouds.far_lod.enabled=true", "clouds.far_lod.start_m=1e9"], false),
    ]);
    let (off, on, red) = (diff(r[1], r[0]), diff(r[2], r[0]), diff(r[3], r[0]));
    let limit = scale(off, FAR_LOD_SHARE);
    println!("COST cloud far LOD: off {off}, on {on} ms (budget <= {FAR_LOD_SHARE} x off = {limit}); red arm (start at infinity) {red}");
    if software() { return; }
    let mut f = vec![];
    at_most(&mut f, "far LOD on", on, limit);
    red_above(&mut f, red, limit);
    finish("COST cloud far LOD", f);
}

// ---------------------------------------------------------------- empty-space early return

/// Share 0.9: with `clouds.skip_empty` on, the clouds' cost at 1080p is at most 0.9 x the same with it off. It changes no pixel, so any
/// saving pays; 10% is what this gate can tell from run-to-run spread, and a switch that saves less is dead weight to delete.
const SKIP_EMPTY_SHARE: f64 = 0.9;

/// The early return earns its switch (clouds' cost = frame minus the frame with clouds off, shafts off in every arm). The red arm is
/// the off arm itself (`skip_empty` false, every tap taken): by construction 1.0 x off, above the limit.
#[test]
#[ignore = "manual gate"]
fn cost_clouds_skip_empty() {
    if skip() { return; }
    let r = arms(&[
        ("cost_skip_none", &["clouds.enabled=false"], false), ("cost_skip_off", &["clouds.shaft_scale=0", "clouds.skip_empty=false"], false),
        ("cost_skip_on", &["clouds.shaft_scale=0"], false),
    ]);
    let (off, on) = (diff(r[1], r[0]), diff(r[2], r[0]));
    // Both costs subtract the same clouds-off frame, so compare the frames: on <= none + share x (off - none) = (1 - share) x none +
    // share x off, which carries only a tenth of the clouds-off frame's spread.
    let limit = Cost { ms: (1.0 - SKIP_EMPTY_SHARE) * r[0].ms + SKIP_EMPTY_SHARE * r[1].ms, unc: (1.0 - SKIP_EMPTY_SHARE) * r[0].spread + SKIP_EMPTY_SHARE * r[1].spread };
    println!("COST cloud empty-space return: off {off}, on {on} ms; frame on {:.3} +- {:.3} (budget <= {limit}, clouds at {SKIP_EMPTY_SHARE} x off)", r[2].ms, r[2].spread);
    if software() { return; }
    let mut f = vec![];
    at_most(&mut f, "skip_empty on (frame)", Cost { ms: r[2].ms, unc: r[2].spread }, limit);
    finish("COST cloud empty-space return", f);
}

// ---------------------------------------------------------------- rain shafts

/// Budget 0.3 ms: the roadmap's split for the rain shafts (owner's call).
const SHAFTS_BUDGET_MS: f64 = 0.3;

/// Cost = frame with shafts (`clouds.shaft_scale` 1) minus frame without (0). Red arm, must read above budget: `clouds.render_scale` 1.0
/// (4x the shaft pixels) and `shafts.steps` 320 (4x the steps): 16x the work.
#[test]
#[ignore = "manual gate"]
fn cost_shafts() {
    if skip() { return; }
    let r = arms(&[
        ("cost_shafts_off", &["clouds.shaft_scale=0"], false), ("cost_shafts_1080", &[], false),
        ("cost_shafts_red_off", &["clouds.shaft_scale=0", "clouds.render_scale=1.0"], false), ("cost_shafts_red", &["clouds.render_scale=1.0", "shafts.steps=320.0"], false),
        ("cost_shafts_retina_off", &["clouds.shaft_scale=0"], true), ("cost_shafts_retina", &[], true),
    ]);
    let (s1080, red, retina) = (diff(r[1], r[0]), diff(r[3], r[2]), diff(r[5], r[4]));
    println!("COST rain shafts: 1080p {s1080}, Retina {retina} ms (budget {SHAFTS_BUDGET_MS}); red arm {red}");
    if software() { return; }
    let mut f = vec![];
    at_most(&mut f, "1080p rain shafts", s1080, exact(SHAFTS_BUDGET_MS));
    at_most(&mut f, "Retina rain shafts", retina, exact(SHAFTS_BUDGET_MS));
    red_above(&mut f, red, exact(SHAFTS_BUDGET_MS));
    finish("COST rain shafts", f);
}

// ---------------------------------------------------------------- rain FX

/// Budget 0.3 ms: the roadmap's split for the rain FX (frame with rain forced on minus the same frame forced off, owner's call). Rain FX
/// has no render_scale: its Retina run is at full resolution.
const RAIN_BUDGET_MS: f64 = 0.3;

/// Red arm, must read above budget (1080p): `rain.max_streaks` 384000 and `rain.max_splashes` 96000, 16x the defaults (24000 / 6000).
#[test]
#[ignore = "manual gate"]
fn cost_rain() {
    if skip() { return; }
    let r = arms(&[
        ("cost_rain_1080_off", &["rain.force=0.0"], false), ("cost_rain_1080_on", &["rain.force=1.0"], false),
        ("cost_rain_red_on", &["rain.force=1.0", "rain.max_streaks=384000", "rain.max_splashes=96000"], false),
        ("cost_rain_retina_off", &["rain.force=0.0"], true), ("cost_rain_retina_on", &["rain.force=1.0"], true),
    ]);
    let (rain_1080, red, rain_retina) = (diff(r[1], r[0]), diff(r[2], r[0]), diff(r[4], r[3]));
    println!("COST rain FX: 1080p {rain_1080}, Retina {rain_retina} ms (budget {RAIN_BUDGET_MS}); red arm (16x streaks and splashes) {red}");
    if software() { return; }
    let mut f = vec![];
    at_most(&mut f, "1080p rain FX", rain_1080, exact(RAIN_BUDGET_MS));
    at_most(&mut f, "Retina rain FX", rain_retina, exact(RAIN_BUDGET_MS));
    red_above(&mut f, red, exact(RAIN_BUDGET_MS));
    finish("COST rain FX", f);
}

/// Share 0.25: rain at intensity 0.05 costs at most a quarter of rain at intensity 1 (frame over rain forced off, 1080p). The prefix
/// meshes draw at most 2 x 0.05 = 10% of the quads at 0.05, so a quarter leaves room for the pass's fixed overhead.
const RAIN_SCALE_SHARE: f64 = 0.25;

/// Cost follows intensity: light rain draws a small prefix mesh. Red arm: `rain.draw_levels` 0 (every quad at any intensity, the old
/// cost) must read above the share; if it does not, vertex work was never the cost and the gate cannot fail: log it (roadmap rule).
#[test]
#[ignore = "manual gate"]
fn cost_rain_follows_intensity() {
    if skip() { return; }
    let r = arms(&[
        ("cost_scale_off", &["rain.force=0.0"], false), ("cost_scale_full", &["rain.force=1.0"], false),
        ("cost_scale_light", &["rain.force=0.05"], false), ("cost_scale_red", &["rain.force=0.05", "rain.draw_levels=0"], false),
    ]);
    let (full, light, red) = (diff(r[1], r[0]), diff(r[2], r[0]), diff(r[3], r[0]));
    let limit = scale(full, RAIN_SCALE_SHARE);
    println!("COST rain scale: intensity 1 {full} ms, 0.05 {light} ms (budget <= {RAIN_SCALE_SHARE} x full = {limit}); red arm (draw_levels 0) {red}");
    if software() { return; }
    let mut f = vec![];
    at_most(&mut f, "light rain", light, limit);
    red_above(&mut f, red, limit);
    finish("COST rain scale", f);
}

// ---------------------------------------------------------------- rain off switch

/// Band 0.05 ms (one sixth of the rain FX budget): the most a switched-off rain may cost over a scene with no rain.
const RAIN_OFF_BAND_MS: f64 = 0.05;

/// Off-switch gate, 1080p: `rain.enabled` false with `rain.force` 1 must cost what the same scene with no rain at all costs (`rain.enabled`
/// false alone), within RAIN_OFF_BAND_MS. Red arm: `rain.enabled` true with the same force must read above the band.
#[test]
#[ignore = "manual gate"]
fn cost_rain_off_switch() {
    if skip() { return; }
    let r = arms(&[
        ("cost_rain_switch_none", &["rain.enabled=false"], false), ("cost_rain_switch_off", &["rain.force=1.0", "rain.enabled=false"], false),
        ("cost_rain_switch_red", &["rain.force=1.0", "rain.enabled=true"], false),
    ]);
    let (off, red) = (diff(r[1], r[0]), diff(r[2], r[0]));
    println!("COST rain off switch: forced off {off} ms over no rain (band {RAIN_OFF_BAND_MS}); red arm forced on {red}");
    if software() { return; }
    let mut f = vec![];
    at_most(&mut f, "forced rain with rain.enabled=false over no rain", off, exact(RAIN_OFF_BAND_MS));
    red_above(&mut f, red, exact(RAIN_OFF_BAND_MS));
    finish("COST rain off switch", f);
}
