// clouds composite: blends the march buffer (premultiplied, see clouds_march.wesl) over the sky as a Transparent3d item.
// The pipeline depth-tests per MSAA sample against the cleared far plane, so only sky samples take cloud.
import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

@group(0) @binding(0) var cloud: texture_2d<f32>;
@group(0) @binding(1) var cloud_sampler: sampler;

@fragment
fn main(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return textureSampleLevel(cloud, cloud_sampler, in.uv, 0.0);
}
