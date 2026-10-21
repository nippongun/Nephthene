//! Sky gates G1-G2 (headless; every shot is the `sky` example in `--shot` mode). Manual, #[ignore]:
//! `cargo test --release --test sky_gates -- --ignored --nocapture --test-threads=1`. Metrics print before asserting.
//! G1 proves the sun disc is drawn where the sun direction says, at Bevy's Earth angular size. G2 proves a geometry edge against the
//! bright sky has no pale MSAA hairline (`sky.msaa_fix`). Every budget is reasoned from the geometry or the 8-bit levels, not fitted.
mod common;
use common::*;
use image::RgbImage;
use nephthene::core::{self as w};
use bevy::light::SunDisk;
use bevy::math::Vec3;

/// Pixel (x, y), continuous (pixel centre x + 0.5), of the unit direction `d` in a w x h image of a camera at `pos` looking at `look`
/// (Bevy looking_at, +Y up, vertical fov): the inverse of fog.rs `ray`.
fn project(pos: Vec3, look: Vec3, fov_deg: f32, d: Vec3, w: f32, h: f32) -> (f32, f32) {
    let fwd = (look - pos).normalize();
    let right = fwd.cross(Vec3::Y).normalize();
    let up = right.cross(fwd);
    let t = (fov_deg.to_radians() / 2.0).tan();
    let (a, b, c) = (d.dot(fwd), d.dot(right), d.dot(up));
    ((1.0 + b / a / (t * w / h)) * w / 2.0, (1.0 - c / a / t) * h / 2.0)
}

/// Saturated disc pixels (luma >= 250): their count and centroid (continuous pixel coordinates).
fn disc(im: &RgbImage) -> (usize, (f32, f32)) {
    let (mut n, mut sx, mut sy) = (0usize, 0f32, 0f32);
    for (x, y, p) in im.enumerate_pixels() {
        if luma(p) >= 250.0 { n += 1; sx += x as f32 + 0.5; sy += y as f32 + 0.5; }
    }
    let k = n.max(1) as f32;
    (n, (sx / k, sy / k))
}

// ---------------------------------------------------------------- G1 sun disc

/// G1 the sun disc is drawn where the light comes from, at its true size. Pose `sun_disc` (10 deg lens), 17.5 h, clear sky. The expected
/// centre is the sun direction projected into the pose camera; the expected diameter is Bevy's `SunDisk::EARTH` angular size over the field
/// of view, in pixels. Budget: the centroid within 0.25 x the expected diameter (a quarter of the disc's own width: a sun direction error
/// large enough to see in shadows is far larger); the equivalent diameter 2 sqrt(n / pi) within +-25% of expected (saturation bleeds at
/// most a pixel ring). Red arm: overcast (`clouds.coverage_override` 1.0) hides the disc, so it must fail the same budget.
#[test]
#[ignore = "manual gate"]
fn g1_sun_disc() {
    let (el, az) = w::sun(&w::Clock::default(), 17.5);
    let d = w::dir(el, az);
    let clear = still("g1_clear", "sun_disc", &["--hour", "17.5", "--set", "clouds.coverage_override=0.0"]);
    let overcast = still("g1_overcast", "sun_disc", &["--hour", "17.5", "--set", "clouds.coverage_override=1.0"]);
    let (iw, ih) = (clear.width() as f32, clear.height() as f32);
    let expect = project(Vec3::new(0.0, 1.7, 0.0), Vec3::new(-926.6, 305.7, 222.6), 10.0, d, iw, ih);
    let exp_diam = SunDisk::EARTH.angular_size / 10f32.to_radians() * ih;
    let within = |n: usize, c: (f32, f32)| {
        let diam = 2.0 * (n as f32 / std::f32::consts::PI).sqrt();
        let off = ((c.0 - expect.0).powi(2) + (c.1 - expect.1).powi(2)).sqrt();
        off <= 0.25 * exp_diam && (diam - exp_diam).abs() <= 0.25 * exp_diam
    };
    let (n, c) = disc(&clear);
    let (rn, _) = disc(&overcast);
    let diam = 2.0 * (n as f32 / std::f32::consts::PI).sqrt();
    println!("G1 sun disc: centroid ({:.1}, {:.1}), expected ({:.1}, {:.1}), diameter {diam:.1} px, expected {exp_diam:.1} px; red arm (overcast) {rn} disc px",
        c.0, c.1, expect.0, expect.1);
    let mut f = vec![];
    if !within(n, c) { f.push(format!("G1 disc ({:.1}, {:.1}) diameter {diam:.1} off the sun ({:.1}, {:.1}) diameter {exp_diam:.1}", c.0, c.1, expect.0, expect.1)); }
    if within(rn, disc(&overcast).1) { f.push(format!("G1 red arm (overcast) still shows the disc in budget ({rn} px); the gate is blind")); }
    finish("G1", f);
}

// ---------------------------------------------------------------- G2 MSAA hairline

/// G2 no pale hairline at a geometry edge against the sky. Bevy 0.20 render_sky shades every MSAA sample with depth sample 0, so an edge
/// pixel that blends sky and ground comes out brighter than both sides; `sky.msaa_fix` (default on) shades each sample. Pose `fog_valley`
/// with terrain, 17.5 h, clouds off: mountain silhouettes against a bright sky. Metric on luma l: for each pixel with rows y-2 and y+2 in the
/// image, up = l[y-2], dn = l[y+2]; an edge if |up - dn| > 4 levels, a hairline if also l[y] - max(up, dn) > 2 levels. Score = hairline /
/// edge. Budget 0.01: a pixel that blends sky and ground cannot be brighter than both beyond 8-bit rounding (2 levels covers the rounding
/// of three samples), and 1% leaves room for stray sky-gradient pixels. Red arm `sky.msaa_fix=false` must score above 0.01.
#[test]
#[ignore = "manual gate"]
fn g2_msaa_hairline() {
    let score = |label: &str, sets: &[&str]| {
        let mut a = vec!["--terrain", "--hour", "17.5", "--set", "clouds.enabled=false"];
        for s in sets { a.extend(["--set", s]); }
        let im = still(label, "fog_valley", &a);
        let (w, h) = im.dimensions();
        let (mut edge, mut hair) = (0u64, 0u64);
        for y in 2..h - 2 { for x in 0..w {
            let (up, dn, c) = (luma(im.get_pixel(x, y - 2)), luma(im.get_pixel(x, y + 2)), luma(im.get_pixel(x, y)));
            if (up - dn).abs() > 4.0 {
                edge += 1;
                if c - up.max(dn) > 2.0 { hair += 1; }
            }
        } }
        (hair as f32 / edge.max(1) as f32, edge)
    };
    let (fixed, e1) = score("g2_fix", &[]);
    let (red, e2) = score("g2_red", &["sky.msaa_fix=false"]);
    println!("G2 MSAA hairline: score with fix {fixed:.4} ({e1} edge px), red arm (msaa_fix=false) {red:.4} ({e2} edge px); budget 0.01");
    let mut f = vec![];
    if fixed > 0.01 { f.push(format!("G2 hairline score {fixed:.4} > 0.01")); }
    if red <= 0.01 { f.push(format!("G2 red arm {red:.4} not above 0.01; the check does not see the hairline")); }
    finish("G2", f);
}
