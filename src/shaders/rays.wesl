// Screen-space god rays (GPU Gems 3, ch. 13, Mitchell). `source` box-averages the scene's light above `threshold` (capped at
// `source_max`; sky and sunlit cloud edges) into a low-resolution target (rays.source_scale), so the sun disc spreads over more than one
// march step. `march` walks from each pixel toward the sun's screen position through that source, with a weight that decays per step
// and a Gaussian mask around the sun, at rays.render_scale. `composite` adds the result to the HDR scene (additive blend). Clouds and
// terrain in front of the sun send no light, so the gaps between them stream. `march` subtracts the source's mean across the ray
// (rays.local_mean, rays.mean_width), so an even bright sky adds nothing. With rays.sky_source only pixels whose depth is past
// rays.source_min_m are sources: opaque geometry (a lit wall indoors) sends none. Every constant comes from config `rays` (src/rays.rs).
import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct Rays {
    sun_uv: vec2<f32>,
    fade: f32,
    strength: f32,
    threshold: f32,
    decay: f32,
    radius: f32,
    samples: u32,
    aspect: f32,
    flat_source: u32,
    source_max: f32,
    local_mean: u32,
    mean_width: f32,
    // rays.sky_source (1 = a pixel sends light only if its depth is below depth_max), depth_max = near / rays.source_min_m (reverse Z).
    sky_source: u32,
    depth_max: f32,
}

@group(0) @binding(0) var<uniform> r: Rays;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;
// The scene depth, source pass only (sample 0 under MSAA).
@if(MULTISAMPLED)
@group(0) @binding(3) var depth: texture_depth_multisampled_2d;
@if(!MULTISAMPLED)
@group(0) @binding(3) var depth: texture_depth_2d;

// ponytail: largest finite f16; the targets are Rgba16Float and the sun disc is bright enough to overflow them.
const F16_MAX: f32 = 65504.0;

// ponytail: Rec. 709 luma weights (a standard, not a tunable).
fn luma(c: vec3<f32>) -> f32 { return dot(c, vec3(0.2126, 0.7152, 0.0722)); }

// ponytail: 4 x 4 taps fixed; enough for source_scale down to 1/8, more taps if it goes lower.
// src = the HDR scene. 4 x 4 bilinear taps spread over this target's texel (fwidth of uv) average about 8 x 8 scene pixels at 1/8 scale.
@fragment
fn source(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    if r.fade <= 0.0 { return vec4(0.0); }
    let fp = fwidth(in.uv);
    var acc = vec3(0.0);
    for (var i = 0u; i < 4u; i++) {
        for (var j = 0u; j < 4u; j++) {
            let q = in.uv + (vec2(f32(i), f32(j)) - vec2(1.5)) * 0.25 * fp;
            let c = min(textureSampleLevel(src, smp, q, 0.0).rgb, vec3(F16_MAX));
            let l = luma(c);
            // Opaque geometry nearer than source_min_m sends no light (sky_source).
            let dims = vec2<f32>(textureDimensions(depth));
            let z = textureLoad(depth, vec2<i32>(clamp(q * dims, vec2(0.0), dims - 1.0)), 0);
            let sky = r.sky_source == 0u || z <= r.depth_max;
            acc += select(vec3(0.0), c * (clamp(l - r.threshold, 0.0, r.source_max) / max(l, 1e-4)), sky);
        }
    }
    return vec4(acc / 16.0, 1.0);
}

// src = the `source` target.
@fragment
fn march(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    if r.fade <= 0.0 { return vec4(0.0); }
    let n = max(r.samples, 1u);
    // ponytail: interleaved gradient noise (Jimenez 2014, its published constants) jitters the start so the steps do not band.
    let j = fract(52.9829189 * fract(dot(in.position.xy, vec2(0.06711056, 0.00583715))));
    var acc = vec3(0.0);
    var w = 1.0;
    // Unit vector across the ray (perpendicular to the direction to the sun, in aspect-corrected screen space), back in uv.
    let dsc = (r.sun_uv - in.uv) * vec2(r.aspect, 1.0);
    // ponytail: 1e-6 keeps normalize finite at the sun's own pixel.
    let pn = normalize(vec2(-dsc.y, dsc.x) + vec2(1e-6, 0.0));
    let perp = vec2(pn.x / r.aspect, pn.y) * r.mean_width;
    for (var k = 0u; k < n; k++) {
        let q = mix(in.uv, r.sun_uv, (f32(k) + j) / f32(n));
        if all(q >= vec2(0.0)) && all(q <= vec2(1.0)) {
            var c = vec3(1.0);
            if r.flat_source == 0u {
                c = textureSampleLevel(src, smp, q, 0.0).rgb;
                // Local mean across the ray (taps at +-1 and +-2 mean_width): an even sky cancels, lit gaps next to shadow remain.
                if r.local_mean == 1u {
                    var m = vec3(0.0);
                    for (var t = 1; t <= 2; t++) {
                        let o = perp * f32(t);
                        m += textureSampleLevel(src, smp, q + o, 0.0).rgb + textureSampleLevel(src, smp, q - o, 0.0).rgb;
                    }
                    c -= 0.25 * m;
                }
            }
            let dq = (q - r.sun_uv) * vec2(r.aspect, 1.0);
            acc += c * exp(-dot(dq, dq) / max(r.radius * r.radius, 1e-6)) * w;
        }
        w *= r.decay;
    }
    // ponytail: added light only; shadow streaks would subtract below the atmosphere's light (signed output is the upgrade path).
    return vec4(clamp(acc * (r.strength * r.fade / f32(n)), vec3(0.0), vec3(F16_MAX)), 1.0);
}

// src = the `march` target.
@fragment
fn composite(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return vec4(textureSampleLevel(src, smp, in.uv, 0.0).rgb, 0.0);
}
