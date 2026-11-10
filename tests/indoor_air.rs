//! Air indoors (headless; every shot is the `sky` example in `--shot` mode). Manual: `cargo build --release --example sky && cargo test
//! --release --test indoor_air -- --ignored --nocapture --test-threads=1`. Metrics print before asserting.
//! G43: the air inside a closed building carries no sun in-scatter, and the clouds behind its ceiling do not print on it. Scene:
//! `--room --room-lit --room-deep` (a closed dark concrete room, west wall 13 m away), pose `room_sun` (looking west), 18.3 h (sun 8 deg
//! up behind the west wall), ev100 8 (a host's indoor exposure), frozen clock and wind, no storms, 960x540.
mod common;
use common::*;
use image::RgbImage;

fn shot(label: &str, sets: &[&str]) -> RgbImage {
    let mut args = vec!["--room", "--room-lit", "--room-deep", "--hour", "18.3", "--settle", "120", "--set", "storms.enabled=false",
        "--set", "clock.minutes_per_second=0", "--set", "wind.speed_mps=0", "--set", "wind.gust=0", "--set", "exposure=[[-90.0,8.0],[90.0,8.0]]"];
    for s in sets { args.extend(["--set", *s]); }
    read(&shoot(label, "room_sun", &args), "room_sun")
}

/// Luma of every pixel in a box (x0, y0, x1, y1).
fn lumas(img: &RgbImage, b: (u32, u32, u32, u32)) -> Vec<f32> {
    (b.1..b.3).flat_map(|y| (b.0..b.2).map(move |x| (x, y))).map(|(x, y)| luma(img.get_pixel(x, y))).collect()
}
fn mean(v: &[f32]) -> f32 { v.iter().sum::<f32>() / v.len() as f32 }

/// The west wall, facing the camera with the sun behind it.
const WALL: (u32, u32, u32, u32) = (400, 320, 560, 410);
/// The far half of the ceiling (10 to 13 m away: near the eye the aerial in-scatter, and so its correction, is close to zero).
const CEIL: (u32, u32, u32, u32) = (330, 215, 630, 305);

/// Wall: at most one 8-bit level above the same wall with the air's Mie scattering off (no forward sun glow at all): below what a
/// viewer sees on a flat wall. The bug read 2.4 here (and 16 levels in a host at its exposure and distance).
const WALL_ALLOW: f32 = 1.0;
/// Ceiling: the correction (frame minus the frame with sky.sun_occlusion off) may differ by at most 0.25 levels mean between coverage
/// 0.2 and 0.35: indoors the air is in the building's shadow whatever the sky does, so the clouds behind the ceiling must not change
/// it (rounding noise only). The bug read 0.4 to 2 levels, in the shape of the cloud gaps.
const CEIL_ALLOW: f32 = 0.25;

#[test]
#[ignore = "manual gate"]
fn g43_indoor_air() {
    let mut f = vec![];
    // Wall half (clouds off, so only the building can shadow the air).
    let wall = |label: &str, sets: &[&str]| mean(&lumas(&shot(label, &[&["clouds.enabled=false"], sets].concat()), WALL));
    let reference = wall("g43_wall_nomie", &["sky.mie_scattering=0.0"]);
    let (on, red) = (wall("g43_wall_on", &[]) - reference, wall("g43_wall_red", &["sky.scene_shadow=false"]) - reference);
    println!("G43 wall glow over no-Mie air: {on:.3} levels (budget <= {WALL_ALLOW}); red arm (sky.scene_shadow false) {red:.3}");
    if on > WALL_ALLOW { f.push(format!("wall glow {on:.3} > {WALL_ALLOW}")); }
    if red <= WALL_ALLOW { f.push(format!("wall red arm {red:.3} <= {WALL_ALLOW}: the scene cannot show the glow")); }
    // Ceiling half: the correction under two coverages.
    let corr = |label: &str, cov: &str, sets: &[&str]| {
        let c = format!("clouds.coverage_override={cov}");
        let a = lumas(&shot(&format!("{label}_{cov}"), &[&[c.as_str()], sets].concat()), CEIL);
        let b = lumas(&shot(&format!("{label}_{cov}_off"), &[c.as_str(), "sky.sun_occlusion=false"]), CEIL);
        a.iter().zip(&b).map(|(x, y)| x - y).collect::<Vec<f32>>()
    };
    let delta = |sets: &[&str], label: &str| {
        let (lo, hi) = (corr(label, "0.2", sets), corr(label, "0.35", sets));
        mean(&lo.iter().zip(&hi).map(|(a, b)| (a - b).abs()).collect::<Vec<f32>>())
    };
    let (c, cr) = (delta(&[], "g43_ceil_on"), delta(&["sky.cloud_shafts_mask_behind=true"], "g43_ceil_red"));
    println!("G43 ceiling correction change between coverage 0.2 and 0.35: {c:.3} levels (budget <= {CEIL_ALLOW}); red arm (sky.cloud_shafts_mask_behind true) {cr:.3}");
    if c > CEIL_ALLOW { f.push(format!("ceiling follows the clouds behind it: {c:.3} > {CEIL_ALLOW}")); }
    if cr <= CEIL_ALLOW { f.push(format!("ceiling red arm {cr:.3} <= {CEIL_ALLOW}: the scene cannot show the outline")); }
    finish("G43", f);
}
