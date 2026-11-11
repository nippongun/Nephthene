//! Cloud formation gates (headless; every shot is the `sky` example in `--shot` mode). Manual: `cargo build --release --example sky &&
//! cargo test --release --test formation -- --ignored --nocapture --test-threads=1`. Metrics print before asserting; every threshold is
//! a budget with a stated reason. Each knob is off by default, so each gate's red arm is the default.
//! G44: `clouds.formation.streets` lines the clouds up in rows along the wind.
//! `clouds.formation.type_var` has no gate yet (docs/bugs.md): no probe reads cloud-top height through the air's haze from above.
//! G46: `clouds.look.ambient_occlusion` shades the inside of a cloud, so a thick cloud is not one flat tone.
mod common;
use common::*;
use image::RgbImage;

/// Noon, no storms or ground fog, a fixed coverage, plus `sets`.
fn shot(label: &str, pose: &str, sets: &[&str]) -> RgbImage {
    let mut args = vec!["--hour", "12.0", "--size", "960x540", "--timeout", "900", "--set", "clouds.coverage_override=0.5", "--set", "storms.enabled=false", "--set", "ground_fog.enabled=false"];
    for s in sets { args.extend(["--set", *s]); }
    read(&shoot(label, pose, &args), pose)
}

fn mean_std(v: &[f32]) -> (f32, f32) {
    let n = v.len().max(1) as f64;
    let m = v.iter().map(|x| *x as f64).sum::<f64>() / n;
    (m as f32, (v.iter().map(|x| (*x as f64 - m).powi(2)).sum::<f64>() / n).sqrt() as f32)
}

/// Mean |difference| of the image's red channel between pixels `STRIDE` apart along x, over the same along y. The cloud-alpha probe is
/// grey, so red is the alpha. Rows running up the screen put their edges on the x axis: above 1.
fn row_ratio(im: &RgbImage) -> f32 {
    /// 8 px at 960x540 looking up through a 90 degree lens is about 25 m at the cloud base: under any cloud, over the resolve's grain.
    const STRIDE: u32 = 8;
    let (w, h) = (im.width(), im.height());
    let a = |x: u32, y: u32| im.get_pixel(x, y)[0] as f64;
    let (mut sx, mut sy) = (0f64, 0f64);
    for y in 0..h - STRIDE {
        for x in 0..w - STRIDE {
            sx += (a(x + STRIDE, y) - a(x, y)).abs();
            sy += (a(x, y + STRIDE) - a(x, y)).abs();
        }
    }
    (sx / sy.max(1e-9)) as f32
}

/// Rows must put at least half again as many cloud edges across the wind as along it: under that the eye finds no rows in the field.
const G44_ROWS: f32 = 1.5;

/// Pose `sky_up` looks up with the top of the frame toward -z; the wind blows along z (`wind.from_deg` 180), so the rows run up the
/// screen. Cloud alpha probe (`clouds.debug_view` 4). Red arm: streets 0, the plain field, must read under the budget.
#[test]
#[ignore = "manual gate"]
fn g44_streets_line_clouds_up_along_the_wind() {
    let base = ["clouds.debug_view=4", "wind.from_deg=180"];
    let on = row_ratio(&shot("g44_on", "sky_up", &[&base[..], &["clouds.formation.streets=0.8", "clouds.formation.street_stretch=2.5"]].concat()));
    let red = row_ratio(&shot("g44_red", "sky_up", &base));
    println!("G44 edges across / along the wind: streets {on:.2}, red arm (streets 0) {red:.2}, budget >= {G44_ROWS}");
    let mut f = vec![];
    if on < G44_ROWS { f.push(format!("no rows: {on:.2} < {G44_ROWS}")); }
    if red >= G44_ROWS { f.push(format!("red arm {red:.2} >= {G44_ROWS}: the metric finds rows in the plain field")); }
    finish("G44", f);
}

/// The tone spread inside opaque cloud must be at least 1.5 x the unshaded look's: less and a thick cloud still reads as one flat tone.
const G46_SHADING: f32 = 1.5;

/// Pose `sky_up`. Opaque cloud = alpha probe at its brightest in the off look (the probe does not depend on the lighting). Metric: standard
/// deviation of luma over those pixels, relative to their mean (the occlusion also darkens, so levels alone would flatter it).
/// Red arm: ambient_occlusion 0 (the reference itself: ratio 1).
#[test]
#[ignore = "manual gate"]
fn g46_ambient_occlusion_shades_cloud_bodies() {
    let alpha = shot("g46_alpha", "sky_up", &["clouds.debug_view=4"]);
    // The probe is tonemapped, so alpha 1 is not level 255, and the sun's rays are added on top near the disc. Alpha 1 = the probe's
    // 90th percentile (coverage 0.5 puts far more than a tenth of the frame under opaque cloud); opaque = within 2% of it.
    let mut levels: Vec<u8> = alpha.pixels().map(|p| p[0]).collect();
    levels.sort_unstable();
    let full = levels[levels.len() * 9 / 10] as f32;
    let contrast = |im: &RgbImage| {
        let (m, s) = mean_std(&im.pixels().zip(alpha.pixels()).filter(|(_, a)| (a[0] as f32 - full).abs() <= 0.02 * full).map(|(p, _)| luma(p)).collect::<Vec<_>>());
        s / m.max(1e-6)
    };
    let on = contrast(&shot("g46_on", "sky_up", &["clouds.look.ambient_occlusion=0.15"]));
    let red = contrast(&shot("g46_red", "sky_up", &[]));
    println!("G46 tone spread / mean inside opaque cloud: occlusion 0.15 {on:.4}, red arm (0) {red:.4}, budget >= {G46_SHADING} x red = {:.4}", G46_SHADING * red);
    let mut f = vec![];
    if red <= 0.0 { f.push("no opaque cloud in the red arm".into()); }
    if on < G46_SHADING * red { f.push(format!("cloud bodies no more shaded: {on:.4} < {:.4}", G46_SHADING * red)); }
    finish("G46", f);
}
