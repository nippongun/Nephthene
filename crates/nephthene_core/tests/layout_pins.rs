//! Text pins: the Rust uniform structs (src/clouds/render.rs, src/fog/render.rs) and the WGSL structs (src/shaders) must list the same
//! fields in the same order with the same types, the WGSL `StormCell` must follow `storm::pack`'s layout, and the shaders' rain/shaft maths must keep the
//! expressions the Rust reference (rain.rs) implements. Reading the files as text is the only check that needs no GPU.
use glam::Vec2;
use nephthene_core::{rain as r, storm::{self, Kind, Now, Stage, Storm}};

const RENDER: &str = include_str!("../../../src/clouds/render.rs");
const FOG_RENDER: &str = include_str!("../../../src/fog/render.rs");
const FOG: &str = include_str!("../../../src/shaders/fog.wesl");
const COMMON: &str = include_str!("../../../src/shaders/clouds_common.wesl");
const SHAFT: &str = include_str!("../../../src/shaders/clouds_shaft.wesl");
const RESOLVE: &str = include_str!("../../../src/shaders/clouds_resolve.wesl");
const RAIN: &str = include_str!("../../../src/shaders/rain.wesl");
const RAIN_RS: &str = include_str!("../../../src/rain.rs");
const LIGHTNING_RS: &str = include_str!("../../../src/lightning.rs");
const LIGHTNING: &str = include_str!("../../../src/shaders/lightning.wesl");

/// Text between the `{` after `head` and the first line that is just `}`, comments removed.
fn body(src: &str, head: &str) -> String {
    let s = &src[src.find(head).unwrap_or_else(|| panic!("`{head}` not found")) + head.len()..];
    let s = &s[..s.find("\n}").expect("struct end")];
    s.lines().map(|l| l.split("//").next().unwrap()).collect::<Vec<_>>().join(" ")
}

/// (name, type) pairs of a struct body, in order.
fn fields(body: &str) -> Vec<(String, String)> {
    body.split(',').filter_map(|f| {
        let (n, t) = f.split_once(':')?;
        Some((n.trim().trim_start_matches("pub ").trim().to_string(), t.trim().to_string()))
    }).collect()
}

fn rust_to_wgsl(t: &str) -> String {
    match t { "Vec2" => "vec2<f32>", "Vec3" => "vec3<f32>", "Vec4" => "vec4<f32>", "Mat4" => "mat4x4<f32>", o => o }.to_string()
}

fn same_order(rust_src: &str, rust_head: &str, wgsl_src: &str, wgsl_head: &str) {
    let rs: Vec<_> = fields(&body(rust_src, rust_head)).into_iter().map(|(n, t)| (n, rust_to_wgsl(&t))).collect();
    let ws = fields(&body(wgsl_src, wgsl_head));
    assert!(rs.len() >= 5, "{rust_head}: parsed only {} fields", rs.len());
    assert_eq!(rs, ws, "{rust_head} and WGSL `{wgsl_head}` drifted apart");
}

#[test]
fn cloud_uniform_matches_wgsl() { same_order(RENDER, "pub struct CloudGpu {", COMMON, "struct Clouds {"); }
#[test]
fn shaft_uniform_matches_wgsl() { same_order(RENDER, "pub struct ShaftGpu {", SHAFT, "struct Shaft {"); }
#[test]
fn resolve_uniform_matches_wgsl() { same_order(RENDER, "struct ResolveGpu {", RESOLVE, "struct Resolve {"); }
#[test]
fn fog_uniform_matches_wgsl() { same_order(FOG_RENDER, "pub struct FogGpu {", FOG, "struct Fog {"); }
#[test]
fn fog_resolve_uniform_matches_wgsl() { same_order(FOG_RENDER, "pub struct FogResolveGpu {", FOG, "struct FogResolve {"); }
#[test]
fn rain_uniform_matches_wgsl() { same_order(RAIN_RS, "struct RainGpu {", RAIN, "struct Rain {"); }
#[test]
fn bolt_uniform_matches_wgsl() {
    same_order(LIGHTNING_RS, "struct BoltGpu {", LIGHTNING, "struct Bolt {");
    // The segment array: 2 vec4 per segment on both sides.
    let n = 2 * nephthene_core::lightning::MAX_SEGMENTS;
    assert!(LIGHTNING_RS.contains("s: [Vec4; 2 * MAX_SEGMENTS]"), "src/lightning.rs BoltSegs drifted");
    assert!(LIGHTNING.contains(&format!("s: array<vec4<f32>, {n}>")), "lightning.wesl Segs is not {n} vec4");
}

/// (field, floats) in pack order; a vec2 is two floats.
const EXPECT: [(&str, usize); 13] = [("a", 2), ("b", 2), ("core_r", 1), ("base", 1), ("top", 1), ("overshoot", 1), ("anvil_r", 1), ("shift", 2), ("anvil_base", 1), ("precip", 1), ("gust_r", 1), ("shelf_low", 1), ("dir_angle", 1)];

#[test]
fn wgsl_storm_cell_field_order_is_the_pack_layout() {
    let fields: Vec<(String, usize)> = self::fields(&body(COMMON, "struct StormCell {")).into_iter().map(|(n, t)| (n, if t.contains("vec2") { 2 } else { 1 })).collect();
    let want: Vec<(String, usize)> = EXPECT.iter().map(|(n, k)| (n.to_string(), *k)).collect();
    assert_eq!(fields, want, "clouds_common.wesl StormCell drifted from storm::pack");
    let s = Storm { id: 1, kind: Kind::Cell, born_s: 0.0, life_s: 1.0, origin: Vec2::ZERO, vel: Vec2::ZERO, half_axis: Vec2::ZERO, core_r: 0.0, base: 0.0, top: 0.0, anvil_r: 0.0, intensity: 0.0 };
    let n = Now {
        a: Vec2::new(1.0, 2.0), b: Vec2::new(3.0, 4.0), dir: Vec2::new(1.0, 0.0), stage: Stage::Mature, age: 0.0, base: 6.0,
        core_r: 5.0, top: 7.0, overshoot: 8.0, anvil_r: 9.0, anvil_shift: Vec2::new(10.0, 11.0), anvil_base: 12.0,
        precip: 13.0, gust_r: 14.0, shelf_low: 15.0, outflow_mps: 0.0,
    };
    let flat: Vec<f32> = storm::pack(&[(s, n)]).into_iter().flatten().collect();
    // a.x a.y b.x b.y | core_r base top overshoot | anvil_r shift.x shift.y anvil_base | precip gust_r shelf_low dir_angle(0 = +x)
    assert_eq!(flat, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 0.0], "pack layout changed");
    assert_eq!(fields.iter().map(|f| f.1).sum::<usize>(), flat.len());
}

#[test]
fn rain_shader_mirrors_rust_hash() {
    for (name, v) in [("PCG_MUL", r::PCG_MUL), ("PCG_ADD", r::PCG_ADD), ("PCG_OUT", r::PCG_OUT), ("FLAKE_SALT", r::FLAKE_SALT)] {
        assert!(RAIN.contains(&format!("const {name}: u32 = {v}u;")), "{name} = {v} missing in rain.wesl");
    }
    // The same expressions, token for token.
    for line in [
        "let s = v * PCG_MUL + PCG_ADD;", "let w = ((s >> ((s >> 28u) + 4u)) ^ s) * PCG_OUT;", "return (w >> 22u) ^ w;",
        "f32(pcg(v) >> 8u) / 16777216.0", "hash01(q * 4u + 3u) >= u.intensity", "cam.x - u.half + rem(",
        "let c = q * 7919u + u32(k);", "hash01(c * 4u + 3u) >= u.intensity", "let tt = u.time / u.life + hash01(q * 4u);",
        "let fq = q ^ FLAKE_SALT;", "hash01(fq * 4u) < u.kind", "hash01(fq * 4u + 1u)",
    ] { assert!(RAIN.contains(line), "rain.wesl lost `{line}`"); }
    assert_eq!((r::pcg(0), r::pcg(1), r::pcg(12345)), (129708002, 2831084092, 4099845390));
}

/// clouds_shaft.wesl reads the shaft constants from the `Shaft` uniform and keeps the density expression of `rain::shaft_density`.
#[test]
fn shaft_shader_mirrors_rust() {
    for line in [
        "smoothstep(sh.precip_lo, sh.precip_hi, precip) * smoothstep(sh.cloud_lo, sh.cloud_hi, cloud)",
        "sh.gap + (1.0 - sh.gap) * smoothstep(sh.streak_lo, sh.streak_hi, streak)",
        "gate * streaks * (sh.ground_keep + (1.0 - sh.ground_keep) * clamp(hf, 0.0, 1.0))",
        "let xz0 = p.xz - sh.slant * (base_y - p.y);",
        "t_end = clamp((base_y - eye.y) / d.y, 0.0, sh.max_m)",
    ] { assert!(SHAFT.contains(line), "clouds_shaft.wesl lost `{line}`"); }
}
