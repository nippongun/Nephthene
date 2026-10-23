// clouds composite: blends the march buffer (premultiplied, see clouds_march.wesl) as a Transparent3d item. Its fragment depth is the
// cloud's front (the march's transmittance-weighted depth `t`), depth-tested per MSAA sample: a cloud in front of terrain covers it,
// terrain in front of a cloud hides it, and the sky (the cleared far plane) always takes it. With no cloud depth (t 0) or
// clouds.over_terrain off, the depth is the far plane, so only sky samples take cloud (the old rule). Colour is bilinear, depth nearest
// (R32Float, unfilterable), as clouds_shaft_comp.wesl. The lightning light on the clouds is added from its own low-res march target,
// never through the history.
import bevy_pbr::atmosphere::{bindings::view, functions::uv_to_ray_direction};
import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import super::clouds_common::Clouds;

@group(1) @binding(0) var cloud: texture_2d<f32>;
@group(1) @binding(1) var cloud_sampler: sampler;
@group(1) @binding(2) var flash: texture_2d<f32>;
@group(1) @binding(3) var cloud_t: texture_2d<f32>;
@group(1) @binding(4) var<uniform> u: Clouds;

// The cloud plus the lightning light: additive, alpha unchanged (layout_pins checks this form).
fn with_flash(c: vec4<f32>, f: vec4<f32>) -> vec4<f32> {
    return vec4(c.rgb + f.rgb, c.a);
}

struct Out {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

@fragment
fn main(in: FullscreenVertexOutput) -> Out {
    let c = textureSampleLevel(cloud, cloud_sampler, in.uv, 0.0);
    // Lightning light (clouds_march.wesl MarchOut.flash), added this frame: additive, alpha unchanged.
    let f = textureSampleLevel(flash, cloud_sampler, in.uv, 0.0);
    let dim = vec2<i32>(textureDimensions(cloud_t));
    let t = textureLoad(cloud_t, clamp(vec2<i32>(in.uv * vec2<f32>(dim)), vec2(0), dim - 1), 0).r;
    var depth = 0.0;
    if u.over_terrain != 0u && t > 0.0 {
        let clip = view.clip_from_world * vec4(view.world_position + uv_to_ray_direction(in.uv) * t, 1.0);
        depth = clip.z / clip.w;
    }
    return Out(with_flash(c, f), depth);
}
