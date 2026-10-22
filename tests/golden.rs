//! Golden still gate: the cloud sky of the example scene against the pack in tests/golden/ (poses golden_n15 / ne25 / nw25 at 17.5 and 18 h,
//! still, plus a 3-frame sway sequence). A render change that moves the default look fails here even when the structural gates stay green.
//! Bound (tests/golden/noise.json `bound`), derived from a budget, not from a noise measurement: the gate must catch the smallest look change we call real,
//! a 20% change of the cloud light gain (`MUST_CATCH_GAIN`, default 2.0 untuned), and a gate that trips on harmless drift is as useless as one that
//! misses a change. So the bound is half that change's own score against the pack (`BOUND_FRACTION`): a real change reads at least 2x the bound, and
//! the pack's slack below the bound absorbs driver/compiler drift. A 10% change is below the bound by design; do not tighten the bound to chase it,
//! strengthen the pack instead.
//! Why the pack does not depend on the frame phase: at the default `clouds.update_div` 4 a frozen clock still never stops changing within a test's frames.
//! The resolve's per-pixel running mean keeps converging (up to `resolve.conf_max` 255 visits, 16 frames apart: 0.17/255 per 16 frames at settle 60,
//! 0.10 at settle 600, lavapipe), so a one-frame shift of when the history starts (a pipeline compiled one frame later, a system added) moves a
//! single-frame golden by 0.02-0.2/255. So the pack is rendered by the reference renderer (`clouds.update_div=1`: every pixel marched every frame,
//! converged by settle 300: frame-to-frame and frame-to-16-later both 0.086, no drift), and each still is the mean of 64 consecutive frames (the per-frame
//! jitter averages down; 8 frames already gave 0.046). The div-4 resolve's own look is gated against the same div-1 reference by G7-G14 in tests/clouds.rs.
//! The phase is checked too: the stills render again at settle `PHASE_SHIFT` and the worst shift may use at most half the bound.
//! Goldens are per GPU/driver: regenerate elsewhere. On a software renderer (lavapipe) `golden_cloud_sky` skips with a printed reason;
//! NEPHTHENE_FORCE_GOLDEN=1 runs the comparison there anyway.
//! Regenerate after a look change someone approved: `NEPHTHENE_UPDATE_GOLDEN=1 cargo test --release --test golden`.
mod common;
use common::*;
use image::RgbImage;
use std::{path::PathBuf, process::Command, sync::OnceLock};

/// Sky rows only (top 80%): the horizon and ground say nothing about the cloud look.
fn mad(a: &RgbImage, b: &RgbImage) -> f64 {
    assert_eq!(a.dimensions(), b.dimensions(), "golden size mismatch");
    let n = a.as_raw().len() * 4 / 5;
    let s: u64 = a.as_raw()[..n].iter().zip(&b.as_raw()[..n]).map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs() as u64).sum();
    s as f64 / n as f64
}

/// A 20% light-gain change over the default 2.0 (untuned): the smallest look change the gate must catch (see the module doc).
const MUST_CATCH_GAIN: &str = "2.4";
/// The bound is this fraction of the must-catch change's score, so that change reads at least 1/BOUND_FRACTION x the bound.
const BOUND_FRACTION: f64 = 0.5;
/// The reference renderer: `clouds.update_div=1` marches every pixel every frame. Index 3 is its settle (converged by 300, see the module doc).
const REFERENCE: [&str; 4] = ["--set", "clouds.update_div=1", "--settle", "300"];
/// Consecutive frames averaged per still: the reference's per-frame jitter averages down with the count.
const MEAN_FRAMES: usize = 64;
/// The settle of the phase check: 7 frames later is a different jitter and update phase of the same look.
const PHASE_SHIFT: &str = "307";
/// Per-shot timeout for the reference shots, overriding the example's 240 s default. A settle-300 div-1 run averaging 64 frames takes
/// minutes on a software renderer (lavapipe ran a single case to ~244 s, just over the default, so the gate died of the clock rather
/// than of the picture). This is a hang guard, not a speed budget: only a stuck run should ever reach it.
const TIMEOUT_S: &str = "1800";

/// (shot label, pose, hour, sway)
const CASES: [(&str, &str, &str, bool); 7] = [
    ("n15_175", "golden_n15", "17.5", false), ("n15_180", "golden_n15", "18", false), ("ne25_175", "golden_ne25", "17.5", false), ("ne25_180", "golden_ne25", "18", false),
    ("nw25_175", "golden_nw25", "17.5", false), ("nw25_180", "golden_nw25", "18", false), ("nw25_175_sway", "golden_nw25", "17.5", true),
];

/// The pack this gate reads and writes. NEPHTHENE_GOLDEN_DIR generates a pack for another renderer without touching the reference pack.
fn golden_dir() -> PathBuf { std::env::var("NEPHTHENE_GOLDEN_DIR").map(PathBuf::from).unwrap_or_else(|_| root().join("tests/golden")) }

/// CASES, or only the labels in NEPHTHENE_GOLDEN_CASES (comma-separated): mechanics checks on slow software renderers. Never set it for the real gate.
fn cases() -> Vec<(&'static str, &'static str, &'static str, bool)> {
    let keep = std::env::var("NEPHTHENE_GOLDEN_CASES").ok();
    let v: Vec<_> = CASES.into_iter().filter(|c| keep.as_ref().is_none_or(|k| k.split(',').any(|l| l.trim() == c.0))).collect();
    assert!(!v.is_empty(), "NEPHTHENE_GOLDEN_CASES matches no case");
    v
}

/// The reference renderer's args at `settle`, with the hang-guard timeout.
fn reference(settle: &str) -> Vec<&str> { vec![REFERENCE[0], REFERENCE[1], REFERENCE[2], settle, "--timeout", TIMEOUT_S] }

/// Per-pixel, per-channel mean of equal-size frames, rounded to the nearest level.
fn mean_frames(v: &[RgbImage]) -> RgbImage {
    let (w, h) = v[0].dimensions();
    assert!(v.iter().all(|im| im.dimensions() == (w, h)), "golden frame size mismatch");
    let mut acc = vec![0u32; v[0].as_raw().len()];
    for im in v { for (a, x) in acc.iter_mut().zip(im.as_raw()) { *a += *x as u32; } }
    let n = v.len() as u32;
    RgbImage::from_raw(w, h, acc.iter().map(|a| ((a + n / 2) / n) as u8).collect()).unwrap()
}

/// A still as the mean of MEAN_FRAMES consecutive reference frames at `settle`.
fn mean_still(label: &str, pose: &str, args: &[&str], settle: &str) -> RgbImage {
    let every = format!("{MEAN_FRAMES},1");
    let mut a = args.to_vec();
    a.extend(reference(settle));
    a.extend(["--seq", every.as_str()]);
    mean_frames(&seq(label, pose, MEAN_FRAMES, &a))
}

/// The must-catch shot: the cloud light gain moved by 20% at 17.5 h, as a reference mean.
fn must_catch(label: &str) -> RgbImage {
    mean_still(label, "golden_ne25", &["--hour", "17.5", "--set", &format!("clouds.light_gain={MUST_CATCH_GAIN}")], REFERENCE[3])
}

/// The frames of one case at `settle`, keyed by golden file name. Sway frames move the camera, so they are single frames, not means.
fn render(tag: &str, (label, pose, hour, sway): (&str, &str, &str, bool), settle: &str) -> Vec<(String, RgbImage)> {
    let l = format!("golden_{tag}_{label}");
    if sway {
        let mut a = vec!["--hour", hour, "--sway", "3", "--seq", "3,20"];
        a.extend(reference(settle));
        seq(&l, pose, 3, &a).into_iter().enumerate().map(|(i, im)| (format!("{label}_{i:02}"), im)).collect()
    } else {
        vec![(label.to_string(), mean_still(&l, pose, &["--hour", hour], settle))]
    }
}

/// True on a software renderer. One 64x36 probe run, cached: its log names the adapter (`device_type: Cpu`).
fn software() -> bool {
    static SW: OnceLock<bool> = OnceLock::new();
    *SW.get_or_init(|| {
        let out = Command::new(example()).args(["--shot", "cost", "--out"]).arg(root().join("shots/golden_probe"))
            .args(["--size", "64x36", "--settle", "2"]).env("CLAUDECODE", "1").output().expect("run the sky example");
        let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        log.contains("device_type: Cpu")
    })
}

/// True when this gate must not run here: both gates in this file measure against the committed pack, which is Apple Silicon / Metal,
/// and a software renderer's own difference from it already exceeds the bound. NEPHTHENE_FORCE_GOLDEN=1 runs them anyway, which is how
/// the skip is shown to be a skip and not a deletion.
fn skip_on_software(what: &str) -> bool {
    if std::env::var("NEPHTHENE_FORCE_GOLDEN").as_deref() == Ok("1") || !software() { return false; }
    println!("GOLDEN {what} skipped: the pack is renderer-specific (Apple Silicon / Metal) and a software renderer does not match it; NEPHTHENE_FORCE_GOLDEN=1 compares anyway");
    true
}

#[test]
fn golden_cloud_sky() {
    if skip_on_software("still pack") { return; }
    let dir = golden_dir();
    if std::env::var("NEPHTHENE_UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(&dir).unwrap();
        let mut worst = 0f64;
        let mut diffs = vec![];
        let mut pworst = (0f64, String::new());
        let mut pdiffs = vec![];
        for c in cases() {
            let (a, b) = (render("a", c, REFERENCE[3]), render("b", c, REFERENCE[3]));
            for ((name, ia), (_, ib)) in a.iter().zip(&b) {
                ia.save(dir.join(format!("{name}.png"))).unwrap();
                let d = mad(ia, ib);
                worst = worst.max(d);
                diffs.push(format!("{name}: {d:.4}"));
            }
            // Sway frames are not phase-checked: their camera path reads the settle (`cam_motion`), so a shifted settle moves the camera, not the frame phase.
            if c.3 { continue; }
            for ((name, ia), (_, ip)) in a.iter().zip(render("p", c, PHASE_SHIFT)) {
                let d = mad(ia, &ip);
                if d > pworst.0 { pworst = (d, name.clone()); }
                pdiffs.push(format!("{name}: {d:.4}"));
            }
        }
        let effect = mad(&must_catch("golden_look"), &image::open(dir.join("ne25_175.png")).unwrap().to_rgb8());
        let bound = BOUND_FRACTION * effect;
        std::fs::write(dir.join("noise.json"), serde_json::to_string_pretty(&serde_json::json!({
            "rerun_diffs": diffs, "phase_diffs": pdiffs, "must_catch_effect": effect, "bound": bound,
            "method": "update_div 1, settle 300, mean of 64 frames (stills)" })).unwrap()).unwrap();
        println!("GOLDEN written, rerun worst {worst:.4}, phase worst {:.4} ({}), must-catch effect {effect:.3}, bound {bound:.3}", pworst.0, pworst.1);
        assert!(pworst.0 <= bound / 2.0, "pack is phase-sensitive: {} {:.3} > half the bound {:.3}; the frame phase may use at most half the bound, the rest is the driver/compiler allowance",
            pworst.1, pworst.0, bound / 2.0);
        return;
    }
    let noise: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("noise.json")).expect("tests/golden/noise.json (run with NEPHTHENE_UPDATE_GOLDEN=1)")).unwrap();
    let bound = noise["bound"].as_f64().unwrap();
    println!("GOLDEN bound {bound:.3}/255");
    let mut worst = (0f64, String::new());
    for c in cases() {
        for (name, img) in render("t", c, REFERENCE[3]) {
            let gold = image::open(dir.join(format!("{name}.png"))).unwrap_or_else(|e| panic!("{name}: {e}")).to_rgb8();
            let d = mad(&img, &gold);
            println!("GOLDEN {name}: mean |diff| {d:.3}/255");
            if d > worst.0 { worst = (d, name); }
        }
    }
    assert!(worst.0 <= bound, "look drifted from the golden pack: {} {:.3} > bound {bound:.3}", worst.1, worst.0);
}

/// Red arm: the same pose with the cloud light gain moved by 20% (the must-catch change) must exceed the bound, or the golden gate does not see look changes.
#[test]
fn golden_sees_a_look_change() {
    // Skipped on a software renderer for the same reason as the pack gate, and for a sharper one: this test asserts the must-catch
    // change reads ABOVE the bound against the Apple pack, and a software renderer's own difference from that pack is already above it
    // (0.603 vs bound 0.321 at head 5fa116b). It would pass here without measuring the look change at all.
    if skip_on_software("red arm") { return; }
    let dir = golden_dir();
    let bound = serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(dir.join("noise.json")).unwrap()).unwrap()["bound"].as_f64().unwrap();
    let img = must_catch("golden_red");
    let gold = image::open(dir.join("ne25_175.png")).unwrap().to_rgb8();
    let d = mad(&img, &gold);
    println!("GOLDEN red arm (light_gain {MUST_CATCH_GAIN}): mean |diff| {d:.3}/255 (bound {bound:.3})");
    assert!(d > bound, "a 20% light-gain change reads {d:.3} <= bound {bound:.3}: the gate is blind");
}
