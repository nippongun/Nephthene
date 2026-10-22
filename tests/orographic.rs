//! Orographic cloud gate G31 (headless; the `sky` example in `--shot` mode over the `--terrain` test valley). Manual, #[ignore]:
//! `cargo test --release --test orographic -- --ignored --nocapture --test-threads=1`. Metrics print before asserting.
//! Every threshold is a budget with a stated reason, not a fit to a measurement. See docs/orographic.md.
mod common;
use common::*;
use image::RgbImage;

/// Mean |luma| difference of two frames.
fn mean_diff(a: &RgbImage, b: &RgbImage) -> f32 {
    a.pixels().zip(b.pixels()).map(|(p, q)| (luma(p) - luma(q)).abs()).sum::<f32>() / (a.width() * a.height()) as f32
}
/// Share of pixels whose luma differs by at least `levels`.
fn changed(a: &RgbImage, b: &RgbImage, levels: f32) -> f32 {
    a.pixels().zip(b.pixels()).filter(|(p, q)| (luma(p) - luma(q)).abs() >= levels).count() as f32 / (a.width() * a.height()) as f32
}

/// As G22: two renders of the same scene may differ by the renderer's run-to-run noise (measured each run) plus half a level for the driver.
const SAME_ALLOW: f32 = 0.5;
/// A pixel counts as changed cloud at 20 levels (8 % of the range): clearly visible, well above the renderer's noise.
const CLOUD_LEVELS: f32 = 20.0;
/// A visible cap: at least 1 % of the frame (about 1300 px at 480x270, a cloud a few degrees across) changes.
const CAP_AREA: f32 = 0.01;
/// The view toward the ridge must change at least this many times as much as the downwind view: the tile's cloud source is on the
/// ascent (cover about 0.7 over the windward slope) and only a faint lee wave (cover about 0.1) lies downwind (docs/orographic.md),
/// so a cap drawn on the wrong side (a flipped axis) fails.
const SIDE_RATIO: f32 = 1.5;

/// G31: the cloud shader reads the orographic tile. Poses oro_west and oro_east stand 4.4 km on the lee side of the test valley's
/// north-south ridge (x 2600 m), between its cap and its lee wave (about 2 pi U / N_m = 12.6 km downwind), and look west at the
/// ridge and east away from it. 10 h with the default air: about 5 K above its dew point, so the lifting condensation level (125 m
/// per K) is under the ridge's crest and above the valley's hills, and only the ridge raises cloud. A clear map (coverage 0.1) and a
/// 10 m/s westerly without gusts; storms, ground fog and rain off so only the clouds change. With `orographic.enabled`, at least
/// CAP_AREA of the west view must change by CLOUD_LEVELS against the red arm, and at least SIDE_RATIO x the share of the east view.
/// Red arm: `orographic.gain` 0 (the tile holds nothing): both views must render like orographic off, within run-to-run noise +
/// SAME_ALLOW (mean |luma| difference).
#[test]
#[ignore = "manual gate"]
fn g31_cap_cloud_over_the_ridge() {
    let base = ["--terrain", "--hour", "10", "--set", "map.default_coverage=0.1", "--set", "map.default_coverage_swing=0.0", "--set", "wind.from_deg=270",
        "--set", "wind.speed_mps=10", "--set", "wind.gust=0", "--set", "storms.enabled=false", "--set", "ground_fog.enabled=false", "--set", "rain.enabled=false"];
    let run = |label: &str, pose: &str, sets: &[&str]| {
        let mut args = base.to_vec();
        for s in sets { args.extend(["--set", s]); }
        let (dir, log) = shoot_log(label, pose, &args);
        (read(&dir, pose), log)
    };
    let on = ["orographic.enabled=true"];
    let red = ["orographic.enabled=true", "orographic.gain=0"];
    let (west_on, log) = run("g31_west_on", "oro_west", &on);
    let (west_red, _) = run("g31_west_red", "oro_west", &red);
    let (west_off, _) = run("g31_west_off", "oro_west", &[]);
    let (west_off2, _) = run("g31_west_off2", "oro_west", &[]);
    let (east_on, _) = run("g31_east_on", "oro_east", &on);
    let (east_red, _) = run("g31_east_red", "oro_east", &red);
    let (east_off, _) = run("g31_east_off", "oro_east", &[]);
    let noise = mean_diff(&west_off, &west_off2);
    let (west, east) = (changed(&west_on, &west_red, CLOUD_LEVELS), changed(&east_on, &east_red, CLOUD_LEVELS));
    let red_off = mean_diff(&west_red, &west_off).max(mean_diff(&east_red, &east_off));
    let budget = noise + SAME_ALLOW;
    println!("{}", log.lines().find(|l| l.starts_with("OROGRAPHIC")).unwrap_or("no OROGRAPHIC line"));
    println!("G31: share changed by {CLOUD_LEVELS}+ levels toward the ridge (west) {:.2} % (min {:.0} %), downwind (east) {:.2} % (west >= {SIDE_RATIO}x east); run-to-run noise {noise:.3}; red arm (gain 0) vs off {red_off:.3} (budget {budget:.3})", 100.0 * west, 100.0 * CAP_AREA, 100.0 * east);
    let mut f = vec![];
    if west < CAP_AREA { f.push(format!("west share {west:.4} < {CAP_AREA}: no visible cap cloud")); }
    if west < SIDE_RATIO * east { f.push(format!("west {west:.4} < {SIDE_RATIO} x east {east:.4}: the cloud is on the wrong side")); }
    if red_off > budget { f.push(format!("red arm vs off {red_off:.3} > {budget:.3}: gain 0 still changes the sky")); }
    finish("G31", f);
}
