//! Ground fog gates G15-G19 (headless; every shot is the `sky` example in `--shot` mode with `--terrain`). Manual, #[ignore]:
//! `cargo test --release --test fog -- --ignored --nocapture --test-threads=1`. Metrics print before asserting.
//! Pixels are classed on the CPU: each pose ray is cast into the test valley and marched through nephthene_core::fog's own density
//! (no noise), so "pooled" and "clear" pixels come from the model, not from hand-picked image regions.
mod common;
use common::*;
use image::RgbImage;
use nephthene::core::{self as w, config::GroundFog, fog, rain::{Ground, HeightGrid}};
use bevy::math::Vec3;
use std::process::Command;

/// A camera pose of examples/poses.json; `name` is the `sky --shot` pose name.
struct Pose { name: &'static str, pos: Vec3, look: Vec3, fov_deg: f32 }

/// Pose `fog_valley` (examples/poses.json): from a lookout 78 m above the west rim, across the valley toward the east mountain.
const FOG_VALLEY: Pose = Pose { name: "fog_valley", pos: Vec3::new(-1500.0, 260.0, 600.0), look: Vec3::new(1500.0, 20.0, -600.0), fov_deg: 70.0 };

/// Camera ray through pixel (x, y) of a w x h image (Bevy: vertical fov, looking_at with +Y up).
fn ray(p: &Pose, x: f32, y: f32, w: f32, h: f32) -> Vec3 {
    let fwd = (p.look - p.pos).normalize();
    let right = fwd.cross(Vec3::Y).normalize();
    let up = right.cross(fwd);
    let t = (p.fov_deg.to_radians() / 2.0).tan();
    (fwd + right * ((2.0 * x / w - 1.0) * t * w / h) + up * ((1.0 - 2.0 * y / h) * t)).normalize()
}

/// The fog map around `pose` for per-texel amounts `m` at `hour`, and the sun's lift factor for it.
fn model(c: &GroundFog, g: &HeightGrid, pose: &Pose, hour: f32, m: &[f32]) -> (fog::FogMap, f32) {
    let (el, az) = w::sun(&w::Clock::default(), hour);
    let f = fog::field(c, g, [pose.pos.x, pose.pos.z]);
    let map = fog::map(c, &f, m, &fog::sun_alt(c, g, &f, w::dir(el, az)));
    (map, w::rain::smoothstep(0.0, c.lift_full_deg, el))
}

/// Terrain hit (None = sky or past `max_m`) and the model's fog opacity along the eye ray `d`, marched through `map` (no noise).
fn march(pose: &Pose, c: &GroundFog, g: &HeightGrid, map: &fog::FogMap, sun_k: f32, d: Vec3) -> (Option<Vec3>, f32) {
    let (mut od, mut hit, dt) = (0.0, None, 10.0);
    let mut t = dt * 0.5;
    while t < c.max_m {
        let p = pose.pos + d * t;
        let gy = g.height(p.x, p.z);
        if p.y < gy { hit = Some(p); break; }
        od += fog::density(c, map.at(p.x, p.z), gy, p.y, sun_k, 0.0, 0.5, fog::sigma(c)) * dt;
        t += dt;
    }
    (hit, 1.0 - (-od).exp())
}

/// Per sampled pixel (every 4th in x and y): terrain hit (None = sky or past `max_m`) and the model's fog opacity along the ray.
fn classes(pose: &Pose, c: &GroundFog, g: &HeightGrid, m: &[f32], hour: f32, w: u32, h: u32) -> Vec<(u32, u32, Option<Vec3>, f32)> {
    let (map, sun_k) = model(c, g, pose, hour, m);
    let mut out = vec![];
    for y in (0..h).step_by(4) { for x in (0..w).step_by(4) {
        let (hit, op) = march(pose, c, g, &map, sun_k, ray(pose, x as f32 + 0.5, y as f32 + 0.5, w as f32, h as f32));
        out.push((x, y, hit, op));
    } }
    out
}

/// Mean |luma| difference (0..1) between two shots over the sampled pixels `pick` selects.
fn diff(a: &RgbImage, b: &RgbImage, px: &[(u32, u32, Option<Vec3>, f32)], pick: impl Fn(&(u32, u32, Option<Vec3>, f32)) -> bool) -> (f32, usize) {
    let sel: Vec<_> = px.iter().filter(|p| pick(p)).collect();
    let s: f32 = sel.iter().map(|&&(x, y, _, _)| (luma(a.get_pixel(x, y)) - luma(b.get_pixel(x, y))).abs() / 255.0).sum();
    (s / sel.len().max(1) as f32, sel.len())
}

/// One `sky --shot` run of pose `p` at `hour` with `--set` pairs `sets` and extra CLI args `extra` (e.g. `--size 960x540`). Clouds are off
/// by default; `sets` apply after that default, so a `clouds.enabled=true` in `sets` keeps them.
fn shot(p: &Pose, label: &str, hour: &str, sets: &[&str], extra: &[&str]) -> RgbImage {
    let mut a = vec!["--terrain", "--hour", hour, "--set", "clouds.enabled=false", "--set", "wind.speed_mps=1.0", "--set", "map.default_coverage=0.1", "--set", "map.default_coverage_swing=0.0"];
    for s in sets { a.extend(["--set", s]); }
    a.extend_from_slice(extra);
    still(label, p.name, &a)
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
    let off = shot(&FOG_VALLEY, "g15_off", "5.6", &[], &[]);
    let (w, h) = off.dimensions();
    let px = classes(&FOG_VALLEY, &c, &g, &vec![0.9; (c.bake_px * c.bake_px) as usize], 5.6, w, h);
    let arm = |label: &str, extra: &[&str]| {
        let mut sets = vec!["ground_fog.enabled=true", "ground_fog.force=0.9"];
        sets.extend_from_slice(extra);
        let on = shot(&FOG_VALLEY, label, "5.6", &sets, &[]);
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
    let off = shot(&FOG_VALLEY, "g16_off", hour, &[], &[]);
    let (w, h) = off.dimensions();
    let px = classes(&FOG_VALLEY, &c, &g, &vec![1.0; (c.bake_px * c.bake_px) as usize], 6.5, w, h);
    let floor = |p: &(u32, u32, Option<Vec3>, f32)| p.2.is_some_and(|q| q.x.abs() < 700.0 && q.y < 50.0);
    let (held, n) = diff(&shot(&FOG_VALLEY, "g16_on", hour, &["ground_fog.enabled=true"], &[]), &off, &px, floor);
    let (burnt, _) = diff(&shot(&FOG_VALLEY, "g16_red", hour, &["ground_fog.enabled=true", "ground_fog.ignore_shadow=true"], &[]), &off, &px, floor);
    println!("G16: floor change shaded {held:.4}, red arm (ignore_shadow) {burnt:.4} ({n} px)");
    assert!(n > 200, "pose class too small ({n} floor px)");
    finish("G16", if held >= 1.5 * burnt && held > 0.02 { vec![] } else { vec![format!("shaded floor {held:.4} not 1.5x the sunlit-burn arm {burnt:.4}")] });
}

/// G17 terrain hides the sun glow. Fog off, hour 5.6, pose fog_valley, default size. Terrain pixels 2000..6000 m out whose eye ray
/// is at least 90% in the terrain's shadow (32 points on the eye->hit ray with `fog::lit` below one half, the same sun altitude the
/// fog map uses) are split by the angle to the sun: near (under 20 deg) and far (over 40 deg). Sun occlusion takes the aerial
/// in-scatter of the sun glow out of shadowed terrain, so the near faces must not outshine the far ones: R = mean luma(near) /
/// mean luma(far) <= 1.25. Budget reason: in the ridge's shadow only the kept multiple-scattered share (sky.occlusion_keep 0.2) and
/// the brighter low sky toward the sun may brighten faces near the sun; 25% covers both. Red arm: sky.sun_occlusion=false keeps the
/// glow on the shadowed terrain and must give R > 1.25, else the gate cannot fail.
#[test]
#[ignore = "manual gate"]
fn g17_terrain_hides_the_sun_glow() {
    let c = GroundFog { enabled: false, ..GroundFog::default() };
    let g = fog::test_valley();
    let hour = "5.6";
    let on = shot(&FOG_VALLEY, "g17_on", hour, &["ground_fog.enabled=false"], &[]);
    let red = shot(&FOG_VALLEY, "g17_red", hour, &["ground_fog.enabled=false", "sky.sun_occlusion=false"], &[]);
    let (iw, ih) = on.dimensions();
    let (el, az) = w::sun(&w::Clock::default(), 5.6);
    let sun = w::dir(el, az);
    let f = fog::field(&c, &g, [FOG_VALLEY.pos.x, FOG_VALLEY.pos.z]);
    let map = fog::map(&c, &f, &vec![0.0; (c.bake_px * c.bake_px) as usize], &fog::sun_alt(&c, &g, &f, sun));
    let (mut near, mut far) = (vec![], vec![]);
    for y in (0..ih).step_by(4) { for x in (0..iw).step_by(4) {
        let d = ray(&FOG_VALLEY, x as f32 + 0.5, y as f32 + 0.5, iw as f32, ih as f32);
        // Terrain hit: 10 m steps from the eye out to 6000 m, as in `classes`.
        let (mut t, mut hit) = (5.0f32, None);
        while t < 6000.0 {
            let p = FOG_VALLEY.pos + d * t;
            if p.y < g.height(p.x, p.z) { hit = Some(p); break; }
            t += 10.0;
        }
        let Some(q) = hit else { continue };
        let dist = (q - FOG_VALLEY.pos).length();
        if !(2000.0..6000.0).contains(&dist) { continue; }
        let shaded = (0..32).filter(|i| {
            let p = FOG_VALLEY.pos + (q - FOG_VALLEY.pos) * ((*i as f32 + 0.5) / 32.0);
            fog::lit(&c, p.y, map.at(p.x, p.z)[2]) < 0.5
        }).count();
        if (shaded as f32 / 32.0) <= 0.9 { continue; }
        let angle = d.dot(sun).clamp(-1.0, 1.0).acos().to_degrees();
        if angle < 20.0 { near.push((x, y)); } else if angle > 40.0 { far.push((x, y)); }
    } }
    let mean = |img: &RgbImage, px: &[(u32, u32)]| px.iter().map(|&(x, y)| luma(img.get_pixel(x, y))).sum::<f32>() / px.len().max(1) as f32;
    let (mn, mf) = (mean(&on, &near), mean(&on, &far));
    let (rn, rf) = (mean(&red, &near), mean(&red, &far));
    let (r, rr) = (mn / mf, rn / rf);
    println!("G17: near {mn:.2} ({} px), far {mf:.2} ({} px), R {r:.4}; red arm (sun_occlusion=false) R {rr:.4}", near.len(), far.len());
    assert!(near.len() > 200 && far.len() > 200, "pose classes too small ({} near, {} far px)", near.len(), far.len());
    let mut red_list = vec![];
    if r > 1.25 { red_list.push(format!("R {r:.4} > 1.25")); }
    if rr <= 1.25 { red_list.push(format!("red arm (sun_occlusion=false) R {rr:.4} <= 1.25: the gate cannot fail")); }
    finish("G17", red_list);
}

/// Mean 10-90% edge width (rows) of the fog top over every 4th column of `img` (pose fog_valley), and the columns kept. Per column the
/// noise-free model opacity is marched on every row with `c` (this arm's config); y_b is the first row where opacity reaches 0.5 from a
/// row above 0.5. a = mean luma of rows y_b-12..=y_b-8, b = mean of y_b+8..=y_b+12; columns with |b - a| < 10 levels are skipped; the
/// width is the count of rows y_b-12..=y_b+12 whose luma lies strictly between a + 0.1(b - a) and a + 0.9(b - a).
fn top_width(img: &RgbImage, c: &GroundFog, g: &HeightGrid, hour: f32) -> (f32, usize) {
    let (w, h) = img.dimensions();
    let (map, sun_k) = model(c, g, &FOG_VALLEY, hour, &vec![0.9; (c.bake_px * c.bake_px) as usize]);
    let (mut sum, mut kept) = (0.0, 0usize);
    for x in (0..w).step_by(4) {
        let op: Vec<f32> = (0..h).map(|y| march(&FOG_VALLEY, c, g, &map, sun_k, ray(&FOG_VALLEY, x as f32 + 0.5, y as f32 + 0.5, w as f32, h as f32)).1).collect();
        let Some(yb) = (1..h as usize).find(|&y| op[y] >= 0.5 && op[y - 1] < 0.5) else { continue };
        if yb < 12 || yb + 12 >= h as usize { continue; }
        let l = |y: usize| luma(img.get_pixel(x, y as u32));
        let a = (yb - 12..=yb - 8).map(l).sum::<f32>() / 5.0;
        let b = (yb + 8..=yb + 12).map(l).sum::<f32>() / 5.0;
        if (b - a).abs() < 10.0 { continue; }
        let (p, q) = (a + 0.1 * (b - a), a + 0.9 * (b - a));
        let (lo, hi) = (p.min(q), p.max(q));
        sum += (yb - 12..=yb + 12).filter(|&y| l(y) > lo && l(y) < hi).count() as f32;
        kept += 1;
    }
    (sum / kept.max(1) as f32, kept)
}

/// G18 the pool's top is soft. Fog forced to 0.9, pose fog_valley, hour 8.0 (the pool top is in the sun); measured by `top_width`. Green
/// (default wisp_m and top_fade_m) must be at least 1.5x the red arm, which is the hard level top: ground_fog.wisp_m=0 and
/// ground_fog.top_fade_m=0 together (no wisps, no haze above the pool top). A hard top puts the 10-90% transition in one
/// or two rows at this distance; 1.5x is the smallest spread that reads as soft. Needs 30 kept columns.
#[test]
#[ignore = "manual gate"]
fn g18_pool_top_is_soft() {
    let c = GroundFog { enabled: true, force: Some(0.9), ..GroundFog::default() };
    let red_cfg = GroundFog { wisp_m: 0.0, top_fade_m: 0.0, ..c.clone() };
    let g = fog::test_valley();
    let arm = |label: &str, cfg: &GroundFog, extra: &[&str]| {
        let mut sets = vec!["ground_fog.enabled=true", "ground_fog.force=0.9"];
        sets.extend_from_slice(extra);
        let img = shot(&FOG_VALLEY, label, "8.0", &sets, &[]);
        let (width, kept) = top_width(&img, cfg, &g, 8.0);
        println!("G18 {label}: edge width {width:.3} rows over {kept} columns");
        assert!(kept >= 30, "{label}: {kept} columns with a fog top (need 30)");
        width
    };
    let green = arm("g18_green", &c, &[]);
    let red = arm("g18_red", &red_cfg, &["ground_fog.wisp_m=0", "ground_fog.top_fade_m=0"]);
    finish("G18", if green >= 1.5 * red { vec![] } else { vec![format!("green width {green:.3} < 1.5x red {red:.3}")] });
}

/// `FOGMS march a | composite b | total c` lines of a run's output as (march, composite, total) in ms.
fn fogms(log: &str) -> Vec<(f32, f32, f32)> {
    log.lines().filter_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() != 9 || f[0] != "FOGMS" || f[1] != "march" || f[4] != "composite" || f[7] != "total" { return None; }
        Some((f[2].parse().ok()?, f[5].parse().ok()?, f[8].parse().ok()?))
    }).collect()
}

/// G19 the fog is cheap on the GPU. Manual, and measured on the reference GPU only: software renderers (lavapipe, llvmpipe) and runs
/// without timestamp queries print a skip line and pass. Pose fog_valley, hour 5.6, fog forced to 0.9, 1920x1080, clouds off, clouds.pass_timing
/// on. The last `FOGMS` total (mean of 60 frames, ms) must be at most 1.5 ms. Budget reason: a 60 fps frame is 16.7 ms and the fog may take
/// about a tenth of it next to the clouds; Retina's 2x pixels then stay near 3 ms. Red arm: render_scale=1 with hash_noise=true (the
/// full-resolution hashed-noise pass the owner measured at about 30 fps on Retina) must exceed 1.5 ms, else the gate cannot fail.
#[test]
#[ignore = "manual gate"]
fn g19_fog_cost() {
    let run = |arm: &str, extra: &[&str]| -> Option<(f32, f32, f32)> {
        let dir = root().join("shots").join(format!("g19_{arm}"));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cmd = Command::new(example());
        cmd.args(["--shot", "fog_valley", "--terrain", "--hour", "5.6", "--size", "1920x1080", "--settle", "200", "--out"]).arg(&dir);
        cmd.args(["--set", "ground_fog.enabled=true", "--set", "ground_fog.force=0.9", "--set", "clouds.enabled=false", "--set", "clouds.pass_timing=true"]);
        for s in extra { cmd.args(["--set", s]); }
        let out = cmd.env("CLAUDECODE", "1").output().expect("run the sky example");
        let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        if log.contains("llvmpipe") || log.contains("lavapipe") { return None; }
        assert_eq!(out.status.code(), Some(0), "g19_{arm}: `--shot fog_valley` exit {:?}\n{log}", out.status.code());
        let last = fogms(&log).last().copied();
        if let Some((m, c, t)) = last { println!("G19 {arm}: march {m:.3} ms | composite {c:.3} ms | total {t:.3} ms"); }
        last
    };
    let Some(green) = run("on", &[]) else {
        println!("G19 skipped: software renderer or no timestamp queries (cost is measured on the reference GPU)");
        return;
    };
    let red = run("red", &["ground_fog.render_scale=1", "ground_fog.hash_noise=true"]);
    let mut list = vec![];
    if green.2 > 1.5 { list.push(format!("fog total {:.3} ms > 1.5 ms", green.2)); }
    match red {
        Some((_, _, rt)) if rt > 1.5 => {}
        Some((_, _, rt)) => list.push(format!("red arm (render_scale=1, hash_noise) total {rt:.3} ms <= 1.5 ms: the gate cannot fail")),
        None => list.push("red arm produced no FOGMS line: the gate cannot be checked".into()),
    }
    finish("G19", list);
}
