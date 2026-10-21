//! Golden still gate: the cloud sky of the example scene against the pack in tests/golden/ (poses golden_n15 / ne25 / nw25 at 17.5 and 18 h,
//! still, plus a 3-frame sway sequence). A render change that moves the default look fails here even when the structural gates stay green.
//! Bound (tests/golden/noise.json `bound`), derived from a budget, not from a noise measurement: the gate must catch the smallest look change we call real,
//! a 20% change of the cloud light gain (`MUST_CATCH_GAIN`, default 2.0 untuned), and a gate that trips on harmless drift is as useless as one that
//! misses a change. So the bound is half that change's own score against the pack (`BOUND_FRACTION`): a real change reads at least 2x the bound, and
//! the pack's slack below the bound absorbs driver/compiler drift. Measured with the committed pack (Apple Silicon, Metal): re-runs are bit-identical
//! across 6 cold processes incl. one after a rebuild (spread 0), but the render sits a fixed 0.04-0.21 /255 off the pack (worst ne25_180 0.206); the
//! 20% change scores 0.64 on ne25_175, so the bound is 0.32 (drift uses 64% of it). A 10% change (0.335) is below the bound by design; do not tighten the
//! bound to chase it, strengthen the pack instead. Goldens are per GPU/driver: regenerate elsewhere.
//! Regenerate after a look change someone approved: `NEPHTHENE_UPDATE_GOLDEN=1 cargo test --release --test golden`.
mod common;
use common::*;
use image::RgbImage;

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

/// (shot label, pose, hour, sway)
const CASES: [(&str, &str, &str, bool); 7] = [
    ("n15_175", "golden_n15", "17.5", false), ("n15_180", "golden_n15", "18", false), ("ne25_175", "golden_ne25", "17.5", false), ("ne25_180", "golden_ne25", "18", false),
    ("nw25_175", "golden_nw25", "17.5", false), ("nw25_180", "golden_nw25", "18", false), ("nw25_175_sway", "golden_nw25", "17.5", true),
];

/// The frames of one case, keyed by golden file name.
fn render(tag: &str, (label, pose, hour, sway): (&str, &str, &str, bool)) -> Vec<(String, RgbImage)> {
    let l = format!("golden_{tag}_{label}");
    if sway {
        seq(&l, pose, 3, &["--hour", hour, "--sway", "3", "--seq", "3,20"]).into_iter().enumerate().map(|(i, im)| (format!("{label}_{i:02}"), im)).collect()
    } else {
        vec![(label.to_string(), still(&l, pose, &["--hour", hour]))]
    }
}

#[test]
fn golden_cloud_sky() {
    let dir = root().join("tests/golden");
    if std::env::var("NEPHTHENE_UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(&dir).unwrap();
        let mut worst = 0f64;
        let mut diffs = vec![];
        for c in CASES {
            let (a, b) = (render("a", c), render("b", c));
            for ((name, ia), (_, ib)) in a.iter().zip(&b) {
                ia.save(dir.join(format!("{name}.png"))).unwrap();
                let d = mad(ia, ib);
                worst = worst.max(d);
                diffs.push(format!("{name}: {d:.4}"));
            }
        }
        let effect = mad(&still("golden_look", "golden_ne25", &["--hour", "17.5", "--set", &format!("clouds.light_gain={MUST_CATCH_GAIN}")]), &image::open(dir.join("ne25_175.png")).unwrap().to_rgb8());
        let bound = BOUND_FRACTION * effect;
        std::fs::write(dir.join("noise.json"), serde_json::to_string_pretty(&serde_json::json!({ "rerun_diffs": diffs, "must_catch_effect": effect, "bound": bound })).unwrap()).unwrap();
        println!("GOLDEN written, rerun worst {worst:.4}, must-catch effect {effect:.3}, bound {bound:.3}");
        return;
    }
    let noise: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("noise.json")).expect("tests/golden/noise.json (run with NEPHTHENE_UPDATE_GOLDEN=1)")).unwrap();
    let bound = noise["bound"].as_f64().unwrap();
    println!("GOLDEN bound {bound:.3}/255");
    let mut worst = (0f64, String::new());
    for c in CASES {
        for (name, img) in render("t", c) {
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
    let dir = root().join("tests/golden");
    let bound = serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(dir.join("noise.json")).unwrap()).unwrap()["bound"].as_f64().unwrap();
    let img = still("golden_red", "golden_ne25", &["--hour", "17.5", "--set", &format!("clouds.light_gain={MUST_CATCH_GAIN}")]);
    let gold = image::open(dir.join("ne25_175.png")).unwrap().to_rgb8();
    let d = mad(&img, &gold);
    println!("GOLDEN red arm (light_gain {MUST_CATCH_GAIN}): mean |diff| {d:.3}/255 (bound {bound:.3})");
    assert!(d > bound, "a 20% light-gain change reads {d:.3} <= bound {bound:.3}: the gate is blind");
}
