// clouds composite: blends the march buffer (premultiplied, see clouds_march.wesl) over the sky as a Transparent3d item.
// The pipeline depth-tests per MSAA sample against the cleared far plane, so only sky samples take cloud.
// The lightning light on the clouds is added from its own low-res march target, never through the history.
import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

@group(0) @binding(0) var cloud: texture_2d<f32>;
@group(0) @binding(1) var cloud_sampler: sampler;
@group(0) @binding(2) var flash: texture_2d<f32>;

@fragment
fn main(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(cloud, cloud_sampler, in.uv, 0.0);
    // Lightning light (clouds_march.wesl MarchOut.flash), added this frame: additive, alpha unchanged.
    let f = textureSampleLevel(flash, cloud_sampler, in.uv, 0.0);
    return vec4(c.rgb + f.rgb, c.a);
}
