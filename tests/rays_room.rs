//! God rays indoors (headless; every shot is the `sky` example in `--shot` mode). Manual: `cargo build --release --example sky &&
//! cargo test --release --test rays_room -- --ignored --nocapture --test-threads=1`. Metrics print before asserting.
//! G41: opaque geometry sends no rays (`rays.sky_source`). Scene: pose `room_sun` (inside the `--room` box, looking west toward the 16 h
//! sun, which sits behind the bright ceiling's seam with the dim west wall), coverage 0.3, frozen clock and wind, no storms, 960x540.
//! Metric: the light the rays add, mean over the frame of max(0, luma(rays on) - luma(rays off)).
mod common;
use common::*;
use image::RgbImage;

fn shot(label: &str, room: &str, sets: &[&str]) -> RgbImage {
    let mut args = vec![room, "--hour", "16", "--set", "clouds.coverage_override=0.3", "--set", "storms.enabled=false",
        "--set", "clock.minutes_per_second=0", "--set", "wind.speed_mps=0", "--set", "wind.gust=0"];
    for s in sets { args.extend(["--set", *s]); }
    read(&shoot(label, "room_sun", &args), "room_sun")
}

/// Mean light the rays add: max(0, on - off) in luma levels.
fn added(on: &RgbImage, off: &RgbImage) -> f32 {
    on.pixels().zip(off.pixels()).map(|(a, b)| (luma(a) - luma(b)).max(0.0)).sum::<f32>() / (on.width() * on.height()) as f32
}

/// Closed room: at most a quarter of one 8-bit level over the frame (rounding noise; no visible fan). The bug read 6.2 (lavapipe and
/// Metal alike would show a fan of tens of levels below the seam).
const CLOSED_ALLOW: f32 = 0.25;
/// Window: the rays must still add at least that much through a 2 x 1.4 m window (it read 0.89 on Metal): sky pixels still stream.
const WINDOW_MIN: f32 = 0.25;

#[test]
#[ignore = "manual gate"]
fn g41_walls_send_no_rays() {
    let off = shot("g41_closed_off", "--room", &["rays.enabled=false"]);
    let on = shot("g41_closed_on", "--room", &[]);
    // Red arm: every bright pixel a source (the old rule) must add a visible fan in the same closed room.
    let red = shot("g41_closed_red", "--room", &["rays.sky_source=false"]);
    let woff = shot("g41_window_off", "--room-window", &["rays.enabled=false"]);
    let won = shot("g41_window_on", "--room-window", &[]);
    let (c, r, w) = (added(&on, &off), added(&red, &off), added(&won, &woff));
    println!("G41 closed room: rays add {c:.3} levels (budget <= {CLOSED_ALLOW}); red arm (sky_source false) {r:.3}");
    println!("G41 window: rays add {w:.3} levels (must be >= {WINDOW_MIN}); its red arm is the closed room above ({c:.3})");
    let mut f = vec![];
    if c > CLOSED_ALLOW { f.push(format!("closed room adds {c:.3} > {CLOSED_ALLOW}: walls send rays")); }
    if r <= CLOSED_ALLOW { f.push(format!("red arm {r:.3} <= {CLOSED_ALLOW}: the scene cannot show the bug")); }
    if w < WINDOW_MIN { f.push(format!("window adds {w:.3} < {WINDOW_MIN}: the sky no longer streams")); }
    finish("G41", f);
}
