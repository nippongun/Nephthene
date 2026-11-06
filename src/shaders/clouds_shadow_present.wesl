// Cloud shadow cookie, shown image: per frame, the two latest finished cookies mixed across the redraw cycle, through a tent filter.
import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
struct Present { shift: vec2<f32>, t: f32, soft: f32 }
@group(0) @binding(0) var<uniform> p: Present;
@group(0) @binding(1) var done_tex: texture_2d<f32>;
@group(0) @binding(2) var prev_tex: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
fn tap(tex: texture_2d<f32>, uv: vec2<f32>) -> f32 { return textureSampleLevel(tex, samp, uv, 0.0).r; }
// Tent (1 2 1 in each axis at soft = one texel): four bilinear taps at +-soft/2 give the same weights as nine point reads. Soft 0 reads one.
fn tent(tex: texture_2d<f32>, uv: vec2<f32>) -> f32 {
    if p.soft <= 0.0 { return tap(tex, uv); }
    let h = 0.5 * p.soft;
    return 0.25 * (tap(tex, uv + vec2(-h, -h)) + tap(tex, uv + vec2(h, -h)) + tap(tex, uv + vec2(-h, h)) + tap(tex, uv + vec2(h, h)));
}
@fragment
fn main(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return vec4(mix(tent(prev_tex, in.uv + p.shift), tent(done_tex, in.uv), p.t));
}
