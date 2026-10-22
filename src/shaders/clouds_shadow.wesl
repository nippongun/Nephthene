// Cloud shadow map: for each texel of the light-plane cookie (nephthene_core shadow_*), the sun transmittance through the cloud layer.
// Base noise only, no detail. Density is clouds_common.wesl's (mean detail erosion); flat layer, no planet curvature.
import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import super::clouds_common::{Clouds, Storms, density_at, map_sample, StormCell, storm_unpack, storm_reach, storm_profile};

@group(0) @binding(0) var<uniform> u: Clouds;
@group(0) @binding(1) var base_tex: texture_3d<f32>;
@group(0) @binding(2) var detail_tex: texture_3d<f32>;
@group(0) @binding(3) var noise_sampler: sampler;
@group(0) @binding(4) var map_tex: texture_2d<f32>;
@group(0) @binding(5) var map_sampler: sampler;
@group(0) @binding(6) var<uniform> storms: Storms;
@group(0) @binding(7) var oro_tex: texture_2d<f32>;
@group(0) @binding(8) var oro_sampler: sampler;

fn density(xz: vec2<f32>, alt: f32, h: f32, detail: bool) -> f32 {
    return density_at(u, base_tex, detail_tex, noise_sampler, map_tex, map_sampler, oro_tex, oro_sampler, xz, alt, h, detail, true);
}
// Analytic sun optical depth through the storm cells (the march has the same functions, clouds_march.wesl).
fn storm_od(p: vec3<f32>, dir: vec3<f32>, len: f32, steps: u32) -> f32 {
    let ds = len / f32(steps);
    var sum = 0.0;
    for (var k = 0u; k < steps; k++) {
        let q = p + dir * ((f32(k) + 0.5) * ds);
        for (var i = 0u; i < storms.n; i++) {
            let c = storm_unpack(storms.cells[4u * i], storms.cells[4u * i + 1u], storms.cells[4u * i + 2u], storms.cells[4u * i + 3u]);
            if q.y < c.shelf_low || q.y > c.top + c.overshoot { continue; }
            let dm = q.xz - 0.5 * (c.a + c.b);
            let r = storm_reach(c);
            if dot(dm, dm) > r * r { continue; }
            sum += storm_profile(u, c, q.xz, q.y);
        }
    }
    return sum * ds * u.s_od_per_m;
}
struct Shadow { origin: vec3<f32>, half_m: f32, right: vec3<f32>, strength: f32, up: vec3<f32>, samples: u32 }
@group(1) @binding(0) var<uniform> sh: Shadow;

@fragment
fn main(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let n = vec2<f32>(textureDimensions(map_tex)); // unused size source replaced below
    let uv = in.uv;
    let l = vec2(1.0 - 2.0 * uv.x, 2.0 * uv.y - 1.0);
    let s = u.sun_dir;
    if s.y < 0.02 { return vec4(1.0); }
    let p = sh.origin + sh.right * (l.x * sh.half_m) + sh.up * (l.y * sh.half_m);
    let g = p - s * (p.y / s.y);
    let t0 = u.bottom / s.y;
    let dt = (u.top - u.bottom) / s.y / f32(sh.samples);
    var od = 0.0;
    for (var i = 0u; i < sh.samples; i++) {
        let t = t0 + (f32(i) + 0.5) * dt;
        let q = g + s * t;
        od += density(q.xz, q.y, (q.y - u.bottom) / (u.top - u.bottom), false) * dt;
    }
    // Storm cells: analytic sun optical depth from the ground point (the cookie only covers the near field; the march lights the rest).
    var sod = 0.0;
    if storms.n > 0u { sod = storm_od(vec3(g.x, 0.0, g.z), s, 20000.0, 16u); }
    return vec4(exp(-od * u.extinction * sh.strength - sod));
}
