// Weather surface: a StandardMaterial (ExtendedMaterial base) darkened and glossed by wetness and whitened by snow cover where the
// ground is flat enough to hold it. The snow mask mirrors nephthene_core::rain::snow_mask; wetness mirrors rain::wet_look's darkening
// and roughness (no snow). ponytail: wet_look's reflectance term is not mirrored yet (docs/bugs.md).
import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
};

struct Surface {
    snow_color: vec3<f32>,
    wet: f32,            // ground wetness 0..1
    snow: f32,           // snow cover 0..1
    wet_darken: f32,
    wet_roughness: f32,
    snow_roughness: f32,
    slope_lo: f32,       // normal y below which no snow holds
    slope_hi: f32,       // normal y above which full snow holds
}

@group(3) @binding(100) var<uniform> s: Surface;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material.flags, pbr_input.material.alpha_cutoff, pbr_input.material.base_color);
    let m = s.snow * smoothstep(s.slope_lo, s.slope_hi, pbr_input.world_normal.y);
    // Snow hides the wet ground under it.
    let wet = s.wet * (1.0 - m);
    let base = pbr_input.material.base_color.rgb * (1.0 - s.wet_darken * wet);
    pbr_input.material.base_color = vec4<f32>(mix(base, s.snow_color, m), pbr_input.material.base_color.a);
    let rough = mix(pbr_input.material.perceptual_roughness, s.wet_roughness, wet);
    pbr_input.material.perceptual_roughness = mix(rough, s.snow_roughness, m);
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
