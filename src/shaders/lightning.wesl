// lightning channel, GPU only: segment i of the bolt is quad i (mesh vertex = corner.x, corner.y, quad id), a camera-facing ribbon at
// least `min_px` pixels wide. A ribbon widened past the channel width is dimmed by the same factor, so the light it sends does not
// change with distance. The light is added (premultiplied colour with alpha 0), dimmed by the camera's distance fog (Bevy
// FogFalloff::Atmospheric extinction) and faded out above the cloud base, where the channel is inside the cloud.
// Positions are world metres; the values come from nephthene_core::lightning (src/lightning.rs).
import bevy_render::view::View;

struct Bolt {
    color: vec3<f32>,
    radiance: f32,       // cd/m^2 before exposure, times this frame's flash brightness
    extinction: vec3<f32>,
    width: f32,          // channel width, m
    min_px: f32,
    base_y: f32,         // cloud base over the strike, m
    base_fade: f32,
    n: u32,              // segments in use
}

// Segment i: (a.xyz, brightness), (b.xyz, 0).
struct Segs {
    s: array<vec4<f32>, 512>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(3) @binding(0) var<uniform> u: Bolt;
@group(3) @binding(1) var<uniform> segs: Segs;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) light: vec3<f32>,
}

@vertex
fn vertex(@location(0) v: vec3<f32>) -> VOut {
    let q = u32(v.z + 0.5);
    var o: VOut;
    if (q >= u.n) {
        o.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return o;
    }
    let a = segs.s[2u * q];
    let b = segs.s[2u * q + 1u];
    let cam = view.world_position;
    let mid = 0.5 * (a.xyz + b.xyz);
    let to_cam = cam - mid;
    // ponytail: 0.1 m only guards the divisions below (as rain.wesl).
    let dist = max(length(to_cam), 0.1);
    let side = normalize(cross(b.xyz - a.xyz, to_cam));
    // One pixel in metres at this distance.
    let px = dist * 2.0 / (view.clip_from_view[1][1] * view.viewport.w);
    let w = max(u.width, u.min_px * px);
    let p = mix(a.xyz, b.xyz, v.y) + side * (v.x - 0.5) * w;
    o.clip = view.clip_from_world * vec4<f32>(p, 1.0);
    o.uv = v.xy;
    let inside = 1.0 - smoothstep(u.base_y, u.base_y + u.base_fade, p.y);
    o.light = u.color * u.radiance * a.w * (u.width / w) * inside * exp(-u.extinction * dist);
    return o;
}

@fragment
fn fragment(i: VOut) -> @location(0) vec4<f32> {
    // Soft across the ribbon; added light, as lit surfaces are scaled by exposure.
    let across = 1.0 - abs(2.0 * i.uv.x - 1.0);
    return vec4<f32>(i.light * across * view.exposure, 0.0);
}
