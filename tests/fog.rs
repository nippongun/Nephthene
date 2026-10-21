//! Ground fog gates G15-G16 (headless; every shot is the `sky` example in `--shot` mode with `--terrain`). Manual, #[ignore]:
//! `cargo test --release --test fog -- --ignored --nocapture --test-threads=1`. Metrics print before asserting.
//! Pixels are classed on the CPU: each pose ray is cast into the test valley and marched through nephthene_core::fog's own density
//! (no noise), so "pooled" and "clear" pixels come from the model, not from hand-picked image regions.
mod common;
use common::*;
use image::RgbImage;
use nephthene::core::{self as w, config::GroundFog, fog, rain::{Ground, HeightGrid}};
use bevy::math::Vec3;

/// Pose `fog_valley` (examples/poses.json): from a lookout 78 m above the west rim, across the valley toward the east mountain.
const POS: Vec3 = Vec3::new(-1500.0, 260.0, 600.0);
const LOOK: Vec3 = Vec3::new(1500.0, 20.0, -600.0);
const FOV_DEG: f32 = 70.0;

/// Camera ray through pixel (x, y) of a w x h image (Bevy: vertical fov, looking_at with +Y up).
fn ray(x: f32, y: f32, w: f32, h: f32) -> Vec3 {
    let fwd = (LOOK - POS).normalize();
    let right = fwd.cross(Vec3::Y).normalize();
    let up = right.cross(fwd);
    let t = (FOV_DEG.to_radians() / 2.0).tan();
    (fwd + right * ((2.0 * x / w - 1.0) * t * w / h) + up * ((1.0 - 2.0 * y / h) * t)).normalize()
}

/// Per sampled pixel (every 4th in x and y): terrain hit (None = sky or past `max_m`) and the model's fog opacity along the ray.
fn classes(c: &GroundFog, g: &HeightGrid, m: &[f32], hour: f32, w: u32, h: u32) -> Vec<(u32, u32, Option<Vec3>, f32)> {
    let clock = w::Clock::default();
    let (el, az) = w::sun(&clock, hour);
    let f = fog::field(c, g, [POS.x, POS.z]);
    let map = fog::map(c, &f, m, &fog::sun_alt(c, g, &f, w::dir(el, az)));
    let sun_k = w::rain::smoothstep(0.0, c.lift_full_deg, el);
    let mut out = vec![];
    for y in (0..h).step_by(4) { for x in (0..w).step_by(4) {
        let d = ray(x as f32 + 0.5, y as f32 + 0.5, w as f32, h as f32);
        let (mut od, mut hit, dt) = (0.0, None, 10.0);
        let mut t = dt * 0.5;
        while t < c.max_m {
            let p = POS + d * t;
            let gy = g.height(p.x, p.z);
            if p.y < gy { hit = Some(p); break; }
            od += fog::density(c, map.at(p.x, p.z), gy, p.y, sun_k, 0.0, 0.5, fog::sigma(c)) * dt;
            t += dt;
        }
        out.push((x, y, hit, 1.0 - (-od).exp()));
    } }
    out
}

/// Mean |luma| difference (0..1) between two shots over the sampled pixels `pick` selects.
fn diff(a: &RgbImage, b: &RgbImage, px: &[(u32, u32, Option<Vec3>, f32)], pick: impl Fn(&(u32, u32, Option<Vec3>, f32)) -> bool) -> (f32, usize) {
    let sel: Vec<_> = px.iter().filter(|p| pick(p)).collect();
    let s: f32 = sel.iter().map(|&&(x, y, _, _)| (luma(a.get_pixel(x, y)) - luma(b.get_pixel(x, y))).abs() / 255.0).sum();
    (s / sel.len().max(1) as f32, sel.len())
}

fn shot(label: &str, hour: &str, sets: &[&str]) -> RgbImage {
    let mut a = vec!["--terrain", "--hour", hour, "--set", "clouds.enabled=false", "--set", "wind.speed_mps=1.0", "--set", "map.default_coverage=0.1", "--set", "map.default_coverage_swing=0.0"];
    for s in sets { a.extend(["--set", s]); }
    still(label, "fog_valley", &a)
}

/// G15 fog follows the terrain. Fog forced to 0.9 just after sunrise; on vs off. Pooled pixels (model opacity > 0.8) must change by at
/// least 0.08 luma: 80% of the background is replaced by fog light, and a budget of a tenth of that change still separates a pool from
/// none. Clear terrain pixels (opacity < 0.05: the hill under the camera, the mountain above the pool) may change by at most a third of
/// the pooled change (thin open-ground mist, noise breakup, aerial light). Red arm: ground_fog.ignore_terrain (a blanket at a fixed
/// height over each column) fogs the clear terrain and must fail the clear budget.
#[test]
#[ignore = "manual gate"]
fn g15_fog_follows_terrain() {
    let c = GroundFog { enabled: true, force: Some(0.9), ..GroundFog::default() };
    let g = fog::test_valley();
    let off = shot("g15_off", "5.6", &[]);
    let (w, h) = off.dimensions();
    let px = classes(&c, &g, &vec![0.9; (c.bake_px * c.bake_px) as usize], 5.6, w, h);
    let arm = |label: &str, extra: &[&str]| {
        let mut sets = vec!["ground_fog.enabled=true", "ground_fog.force=0.9"];
        sets.extend_from_slice(extra);
        let on = shot(label, "5.6", &sets);
        let (pool, np) = diff(&on, &off, &px, |p| p.3 > 0.8);
        let (clear, nc) = diff(&on, &off, &px, |p| p.2.is_some() && p.3 < 0.05);
        println!("G15 {label}: pooled {pool:.4} ({np} px), clear {clear:.4} ({nc} px)");
        assert!(np > 200 && nc > 200, "{label}: pose classes too small ({np} pooled, {nc} clear); the pose no longer frames the valley");
        (pool, clear)
    };
    let (pool, clear) = arm("g15_on", &[]);
    let (_, red_clear) = arm("g15_red", &["ground_fog.ignore_terrain=true"]);
    let mut red = vec![];
    if pool < 0.08 { red.push(format!("pooled change {pool:.4} < 0.08")); }
    if clear > pool / 3.0 { red.push(format!("clear change {clear:.4} > pooled/3 {:.4}", pool / 3.0)); }
    if red_clear <= pool / 3.0 { red.push(format!("red arm (ignore_terrain) clear change {red_clear:.4} within budget: the gate cannot fail")); }
    finish("G15", red);
}

/// G16 a valley in a mountain's shadow keeps its fog. Formation model (no force), calm clear night, 1.5 h after sunrise: the east
/// mountain still shades the floor, open ground has burned to about a third. Floor pixels (terrain hit on the valley floor, |x| < 700 m,
/// below 50 m) must change at least 1.5x as much as in the red arm, ground_fog.ignore_shadow (every texel burns as if sunlit, the
/// elevation-only model): 1.5x is the smallest gap that still means the shadow held fog the open model lost (pool opacity at amount
/// 0.88 vs 0.34 differs far more).
#[test]
#[ignore = "manual gate"]
fn g16_mountain_shadow_holds_fog() {
    let c = GroundFog { enabled: true, ..GroundFog::default() };
    let g = fog::test_valley();
    let hour = "6.5";
    let off = shot("g16_off", hour, &[]);
    let (w, h) = off.dimensions();
    let px = classes(&c, &g, &vec![1.0; (c.bake_px * c.bake_px) as usize], 6.5, w, h);
    let floor = |p: &(u32, u32, Option<Vec3>, f32)| p.2.is_some_and(|q| q.x.abs() < 700.0 && q.y < 50.0);
    let (held, n) = diff(&shot("g16_on", hour, &["ground_fog.enabled=true"]), &off, &px, floor);
    let (burnt, _) = diff(&shot("g16_red", hour, &["ground_fog.enabled=true", "ground_fog.ignore_shadow=true"]), &off, &px, floor);
    println!("G16: floor change shaded {held:.4}, red arm (ignore_shadow) {burnt:.4} ({n} px)");
    assert!(n > 200, "pose class too small ({n} floor px)");
    finish("G16", if held >= 1.5 * burnt && held > 0.02 { vec![] } else { vec![format!("shaded floor {held:.4} not 1.5x the sunlit-burn arm {burnt:.4}")] });
}
