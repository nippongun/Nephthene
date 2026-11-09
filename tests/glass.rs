//! The air's shadow correction behind glass (headless; every shot is the `sky` example in `--shot` mode). Manual: `cargo build --release
//! --example sky && cargo test --release --test glass -- --ignored --nocapture --test-threads=1`. Metrics print before asserting.
//! G42: alpha-blended glass (writes no depth) shows the scene behind it without black or red cut-outs. Scene: `--room-window --room-tint`
//! (dark tinted glass, 90% opaque, in the west wall), pose `room_sun`, 17 h, coverage 0.5, ev100 8 (a host's indoor exposure), no god
//! rays, frozen clock and wind, 960x540. The window's lower strip (rows 340..380, columns 385..580) holds the horizon and distant
//! ground, where the aerial in-scatter and so the correction are largest.
mod common;
use common::*;
use image::RgbImage;

const STRIP_X: std::ops::Range<u32> = 385..580;
const STRIP_Y: std::ops::Range<u32> = 340..380;

fn shot(label: &str, sets: &[&str]) -> RgbImage {
    let mut args = vec!["--room-window", "--room-tint", "--hour", "17.0", "--settle", "120", "--set", "clouds.coverage_override=0.5",
        "--set", "storms.enabled=false", "--set", "clock.minutes_per_second=0", "--set", "wind.speed_mps=0", "--set", "wind.gust=0",
        "--set", "exposure=[[-90.0,8.0],[90.0,8.0]]", "--set", "rays.enabled=false"];
    for s in sets { args.extend(["--set", *s]); }
    read(&shoot(label, "room_sun", &args), "room_sun")
}

/// Pixels of the strip that read as a cut-out: luma under 30 (black) or red over blue by 60 (the red and yellow fringe of a negative sum).
fn cutouts(img: &RgbImage) -> usize {
    STRIP_Y.flat_map(|y| STRIP_X.map(move |x| (x, y))).filter(|&(x, y)| {
        let p = img.get_pixel(x, y);
        luma(p) < 30.0 || p[0] as f32 - p[2] as f32 > 60.0
    }).count()
}

/// Budget 0: with the correction off the strip has none (the glass passes the dim ground and the haze), so any is the correction.
const CUTOUT_ALLOW: usize = 0;

#[test]
#[ignore = "manual gate"]
fn g42_glass_has_no_cutouts() {
    let on = shot("g42_on", &[]);
    // Red arm: the correction unbounded by what the glass lets through (the old rule).
    let red = shot("g42_red", &["sky.cloud_shafts_unbounded=true"]);
    // Reference: the correction off.
    let off = shot("g42_off", &["sky.cloud_shafts=false"]);
    let (c, r, o) = (cutouts(&on), cutouts(&red), cutouts(&off));
    println!("G42 cut-out pixels in the window strip: on {c} (budget {CUTOUT_ALLOW}), red arm {r}, correction off {o}");
    let mut f = vec![];
    if c > CUTOUT_ALLOW { f.push(format!("{c} cut-out pixels > {CUTOUT_ALLOW}")); }
    if r <= CUTOUT_ALLOW { f.push(format!("red arm {r} <= {CUTOUT_ALLOW}: the scene cannot show the bug")); }
    if o > CUTOUT_ALLOW { f.push(format!("correction off {o} > {CUTOUT_ALLOW}: the scene itself reads as cut-outs, the metric is wrong")); }
    finish("G42", f);
}
