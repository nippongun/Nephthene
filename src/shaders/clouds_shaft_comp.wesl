// rain-shaft composite: draws the resolved half-res shaft buffer as a Transparent3d item right after the cloud composite.
// Its fragment depth is the shaft's first sample (`t_front`, clouds_shaft.wesl), so nearer terrain/objects occlude it per MSAA sample
// and farther ones are dimmed by it, as when the march ran in this pass. Colour is bilinear, depth nearest (R32Float, unfilterable).
import bevy_pbr::atmosphere::{bindings::view, functions::uv_to_ray_direction};
import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

@group(1) @binding(0) var col: texture_2d<f32>;
@group(1) @binding(1) var tf: texture_2d<f32>;
@group(1) @binding(2) var smp: sampler;

struct Out {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

@fragment
fn main(in: FullscreenVertexOutput) -> Out {
    let dim = vec2<i32>(textureDimensions(tf));
    let t = textureLoad(tf, clamp(vec2<i32>(in.uv * vec2<f32>(dim)), vec2(0), dim - 1), 0).r;
    if t <= 0.0 { discard; }
    let d = uv_to_ray_direction(in.uv);
    let clip = view.clip_from_world * vec4(view.world_position + d * t, 1.0);
    return Out(textureSampleLevel(col, smp, in.uv, 0.0), clip.z / clip.w);
}
