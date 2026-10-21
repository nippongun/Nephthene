// rain, GPU only: every streak and splash is a quad whose corners are computed here from (quad id, time, uniform); no CPU particle state.
// Mesh vertex = (corner.x, corner.y, quad id). The hash and the drop/splash maths mirror crates/nephthene_core/src/rain.rs line for line
// (that file is the reference; tests/rain.rs asserts the constants and the unit tests pin the values). All numbers untuned.
import bevy_render::view::View;

struct Rain {
    light: vec3<f32>,   // scene radiance before exposure (Rust: sun/sky level, darker under cloud)
    time: f32,
    vel: vec3<f32>,     // drop velocity m/s (fall + wind drift)
    len: f32,           // streak length m
    sun_dir: vec3<f32>, // toward the sun
    intensity: f32,
    half: f32,
    height: f32,
    ground_y: f32,
    mode: u32,          // 0 streaks, 1 splashes
    width: f32,
    alpha: f32,
    life: f32,
    count: u32,
    g_min: vec2<f32>,   // ground grid: west edge x, north edge z
    g_cell: f32,
    g_w: u32,
    g_h: u32,
}

@group(0) @binding(0) var<uniform> view: View;
@group(3) @binding(0) var<uniform> u: Rain;
@group(3) @binding(1) var ground: texture_2d<f32>;

fn gtex(i: i32, j: i32) -> f32 {
    return textureLoad(ground, vec2<i32>(clamp(i, 0, i32(u.g_w) - 1), clamp(j, 0, i32(u.g_h) - 1)), 0).r;
}
// Mirrors nephthene_core::rain::HeightGrid::height (bilinear, clamped).
fn ground_h(xz: vec2<f32>) -> f32 {
    let f = clamp(vec2<f32>((xz.x - u.g_min.x) / u.g_cell - 0.5, (u.g_min.y - xz.y) / u.g_cell - 0.5), vec2<f32>(0.0), vec2<f32>(f32(u.g_w) - 1.0, f32(u.g_h) - 1.0));
    let i = vec2<i32>(floor(f));
    let t = f - floor(f);
    let a = mix(gtex(i.x, i.y), gtex(i.x + 1, i.y), t.x);
    let b = mix(gtex(i.x, i.y + 1), gtex(i.x + 1, i.y + 1), t.x);
    return mix(a, b, t.y);
}

const PCG_MUL: u32 = 747796405u;
const PCG_ADD: u32 = 2891336453u;
const PCG_OUT: u32 = 277803737u;

fn pcg(v: u32) -> u32 {
    let s = v * PCG_MUL + PCG_ADD;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * PCG_OUT;
    return (w >> 22u) ^ w;
}
fn hash01(v: u32) -> f32 { return f32(pcg(v) >> 8u) / 16777216.0; }
fn rem(x: f32, s: f32) -> f32 { return x - s * floor(x / s); }

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) fade: f32,
    @location(2) shade: f32,
}

fn hidden() -> VOut {
    var o: VOut;
    o.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    return o;
}

@vertex
fn vertex(@location(0) v: vec3<f32>) -> VOut {
    let q = u32(v.z + 0.5);
    let corner = v.xy;
    let cam = view.world_position;
    let s = 2.0 * u.half;
    var o: VOut;
    o.uv = corner;
    if (u.mode == 0u) {
        // Streak: head at the drop, tail up along the velocity; a camera-facing ribbon of at least one pixel.
        if (hash01(q * 4u + 3u) >= u.intensity) { return hidden(); }
        let b0 = hash01(q * 4u);
        let b1 = hash01(q * 4u + 1u);
        let b2 = hash01(q * 4u + 2u);
        let hx = cam.x - u.half + rem(b0 * s + u.vel.x * u.time - cam.x, s);
        let hz = cam.z - u.half + rem(b2 * s + u.vel.z * u.time - cam.z, s);
        let head = vec3<f32>(hx, ground_h(vec2<f32>(hx, hz)) + u.ground_y + rem(b1 * u.height + u.vel.y * u.time, u.height), hz);
        let dir = normalize(u.vel);
        let tail = head - dir * u.len;
        let mid = 0.5 * (head + tail);
        let to_cam = cam - mid;
        let dist = max(length(to_cam), 0.1);
        let side = normalize(cross(dir, to_cam));
        // One pixel in metres at this distance.
        let px = dist * 2.0 / (view.clip_from_view[1][1] * view.viewport.w);
        let w = max(u.width, 1.0 * px);
        let p = mix(head, tail, corner.y) + side * (corner.x - 0.5) * w;
        o.clip = view.clip_from_world * vec4<f32>(p, 1.0);
        let hd = length(vec2<f32>(mid.x - cam.x, mid.z - cam.z));
        o.fade = (u.width / w) * (1.0 - smoothstep(0.55 * u.half, u.half, hd)) * smoothstep(0.4, 2.0, dist);
        // Forward scatter: brighter looking toward the sun.
        let d = normalize(mid - cam);
        o.shade = 1.0 + 2.5 * pow(max(dot(d, u.sun_dir), 0.0), 6.0);
        o.uv = vec2<f32>(corner.x, corner.y);
    } else {
        let tt = u.time / u.life + hash01(q * 4u);
        let k = floor(tt);
        let age = tt - k;
        let c = q * 7919u + u32(k);
        if (hash01(c * 4u + 3u) >= u.intensity) { return hidden(); }
        let px0 = cam.x - u.half + rem(hash01(c * 4u + 1u) * s - cam.x, s);
        let pz0 = cam.z - u.half + rem(hash01(c * 4u + 2u) * s - cam.z, s);
        let pos = vec3<f32>(px0, ground_h(vec2<f32>(px0, pz0)) + u.ground_y + 0.01, pz0);
        let to_cam = cam - pos;
        let dist = max(length(to_cam), 0.1);
        let px = dist * 2.0 / (view.clip_from_view[1][1] * view.viewport.w);
        let r = max(0.03 + 0.14 * age, 1.5 * px);
        let p = pos + vec3<f32>((corner.x - 0.5) * 2.0 * r, 0.0, (corner.y - 0.5) * 2.0 * r);
        o.clip = view.clip_from_world * vec4<f32>(p, 1.0);
        let hd = length(vec2<f32>(pos.x - cam.x, pos.z - cam.z));
        o.fade = (1.0 - age) * (1.0 - age) * min(1.0, 0.14 / r * 2.0) * (1.0 - smoothstep(0.4 * u.half, u.half, hd)) * smoothstep(0.4, 2.0, dist);
        o.shade = 1.0 + 1.5 * pow(max(dot(normalize(pos - cam), u.sun_dir), 0.0), 6.0);
    }
    return o;
}

@fragment
fn fragment(i: VOut) -> @location(0) vec4<f32> {
    var a: f32;
    if (u.mode == 0u) {
        // Soft across the ribbon, brighter at the head (corner.y = 0), fading to the tail.
        let across = 1.0 - abs(2.0 * i.uv.x - 1.0);
        a = across * mix(1.0, 0.25, i.uv.y);
    } else {
        let r = length(2.0 * i.uv - vec2<f32>(1.0));
        a = smoothstep(0.45, 0.7, r) * (1.0 - smoothstep(0.8, 1.0, r)) + 0.5 * (1.0 - smoothstep(0.0, 0.35, r));
    }
    let al = clamp(a * i.fade * u.alpha, 0.0, 1.0);
    // Premultiplied: radiance times exposure, as lit surfaces are.
    return vec4<f32>(u.light * view.exposure * i.shade * al, al);
}
