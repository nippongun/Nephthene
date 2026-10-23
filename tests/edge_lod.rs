//! Cloud edge LOD gate (headless; every shot is the `sky` example in `--shot` mode). Manual: `cargo test --release --test edge_lod --
//! --ignored --nocapture --test-threads=1`. Metrics print before asserting; every threshold is a budget with a stated reason.
//! G33: `clouds.edge_lod` changes the edges and leaves the centre alone. Scene: pose `sky_up` (90 deg, looking up, cloud over the
//! whole frame) at noon, coverage 0.6, no storms or ground fog, 960x540.
mod common;
use common::*;
use image::RgbImage;

/// Noon, cloud over the whole frame, no storms or ground fog, plus `sets`. Returns the frame.
fn shot(label: &str, sets: &[&str]) -> RgbImage {
    let mut args = vec!["--hour", "12.0", "--size", "960x540", "--timeout", "900", "--set", "clouds.coverage_override=0.6", "--set", "storms.enabled=false", "--set", "ground_fog.enabled=false"];
    for s in sets { args.extend(["--set", *s]); }
    read(&shoot(label, "sky_up", &args), "sky_up")
}

/// The screen radius of a pixel centre, the same measure as the shader (x and y each -1..1).
fn radius(x: u32, y: u32, w: u32, h: u32) -> f32 {
    ((2.0 * (x as f32 + 0.5) / w as f32 - 1.0).powi(2) + (2.0 * (y as f32 + 0.5) / h as f32 - 1.0).powi(2)).sqrt()
}

/// Mean |luma(a) - luma(b)| over the pixels with `lo <= radius < hi`.
fn diff(a: &RgbImage, b: &RgbImage, lo: f32, hi: f32) -> f32 {
    let (w, h) = (a.width(), a.height());
    let (mut sum, mut n) = (0f64, 0usize);
    for y in 0..h {
        for x in 0..w {
            let r = radius(x, y, w, h);
            if r >= lo && r < hi {
                sum += (luma(a.get_pixel(x, y)) - luma(b.get_pixel(x, y))).abs() as f64;
                n += 1;
            }
        }
    }
    (sum / n.max(1) as f64) as f32
}

/// Centre: rays inside 0.4 are 0.1 inside the default `r0` 0.5 and the default `detail_r0` 0.8 is further out, so they march exactly as
/// with the switch off; 0.1 is margin for the resolve's neighbourhood. Budget: the run-to-run noise of two off shots plus 0.5 levels (the
/// golden pack's run-to-run noise).
const CENTRE_R: f32 = 0.4;
const CENTRE_ALLOW: f32 = 0.5;
/// Edge: past 1.2 the default reduction is full (half the steps, no detail). A change under 1 level above the run-to-run noise is no
/// change: the switch does nothing there.
const EDGE_R: f32 = 1.2;
const EDGE_MIN: f32 = 1.0;

#[test]
#[ignore = "manual gate"]
fn g33_edge_lod_changes_the_edges_only() {
    let off = shot("g33_off", &["clouds.edge_lod.enabled=false"]);
    let off2 = shot("g33_off2", &["clouds.edge_lod.enabled=false"]);
    let on = shot("g33_on", &["clouds.edge_lod.enabled=true"]);
    // Red arm for the centre half: the full reduction everywhere (radii 0 .. 0.01) must change the centre.
    let red = shot("g33_red", &["clouds.edge_lod.enabled=true", "clouds.edge_lod.r0=0.0", "clouds.edge_lod.r1=0.01", "clouds.edge_lod.detail_r0=0.0", "clouds.edge_lod.detail_r1=0.01"]);
    let (nc, ne) = (diff(&off2, &off, 0.0, CENTRE_R), diff(&off2, &off, EDGE_R, 2.0));
    let (c, e, rc) = (diff(&on, &off, 0.0, CENTRE_R), diff(&on, &off, EDGE_R, 2.0), diff(&red, &off, 0.0, CENTRE_R));
    println!("G33 noise (two off shots): centre {nc:.3}, edge {ne:.3}");
    println!("G33 centre {c:.3} (budget <= noise {nc:.3} + {CENTRE_ALLOW} = {:.3})", nc + CENTRE_ALLOW);
    println!("G33 edge {e:.3} (budget >= noise {ne:.3} + {EDGE_MIN} = {:.3})", ne + EDGE_MIN);
    println!("G33 red arm (reduction everywhere) centre {rc:.3} (must be > {:.3}); the edge half's red arm is the off pair itself (noise {ne:.3})", nc + CENTRE_ALLOW);
    let mut f = vec![];
    if c > nc + CENTRE_ALLOW { f.push(format!("centre changed: {c:.3} > {:.3}", nc + CENTRE_ALLOW)); }
    if e < ne + EDGE_MIN { f.push(format!("edge unchanged: {e:.3} < {:.3}", ne + EDGE_MIN)); }
    if rc <= nc + CENTRE_ALLOW { f.push(format!("red arm centre {rc:.3} <= {:.3}: the centre metric cannot see a reduction", nc + CENTRE_ALLOW)); }
    finish("G33", f);
}
