// Screen-space god rays (GPU Gems 3, ch. 13, Mitchell). `source` box-averages the scene's light above `threshold` (capped at
// `source_max`; sky and sunlit cloud edges) into a low-resolution target (rays.source_scale), so the sun disc spreads over more than one
// march step. `march` walks from each pixel toward the sun's screen position through that source, with a weight that decays per step
// and a Gaussian mask around the sun, at rays.render_scale. `composite` adds the result to the HDR scene (additive blend). Clouds and
// terrain in front of the sun send no light, so the gaps between them stream. Every constant comes from config `rays` (src/rays.rs).
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
}

@group(0) @binding(0) var<uniform> r: Rays;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;

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
            acc += c * (clamp(l - r.threshold, 0.0, r.source_max) / max(l, 1e-4));
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
    for (var k = 0u; k < n; k++) {
        let q = mix(in.uv, r.sun_uv, (f32(k) + j) / f32(n));
        if all(q >= vec2(0.0)) && all(q <= vec2(1.0)) {
            var c = vec3(1.0);
            if r.flat_source == 0u { c = textureSampleLevel(src, smp, q, 0.0).rgb; }
            let dq = (q - r.sun_uv) * vec2(r.aspect, 1.0);
            acc += c * exp(-dot(dq, dq) / max(r.radius * r.radius, 1e-6)) * w;
        }
        w *= r.decay;
    }
    return vec4(min(acc * (r.strength * r.fade / f32(n)), vec3(F16_MAX)), 1.0);
}

// src = the `march` target.
@fragment
fn composite(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return vec4(textureSampleLevel(src, smp, in.uv, 0.0).rgb, 0.0);
}
