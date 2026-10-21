// rain-shaft resolve: the pre-4x4 cloud resolve (full jittered half-res march every frame, EMA + 3x3 clamp), on the shaft buffers. Header of the original:
// clouds resolve : blends last frame's resolved cloud buffer into the jittered march output.
// The history is reprojected through the previous view at the cloud's own depth (max_distance where there is no cloud),
// clamped to the current 3x3 neighbourhood (Frostbite 2016 s5.5.3), and blended by `alpha` (weather.json clouds.temporal_alpha).
// A reset (first frame, resized view, skipped frame) takes the current frame as is.
import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct Resolve {
    prev_clip_from_ray: mat4x4<f32>,
    ray_from_clip: mat4x4<f32>,
    delta: vec3<f32>,
    alpha: f32,
    max_distance: f32,
    reset: u32,
}

@group(0) @binding(0) var<uniform> r: Resolve;
@group(0) @binding(1) var cur_tex: texture_2d<f32>;
@group(0) @binding(2) var t_tex: texture_2d<f32>;
@group(0) @binding(3) var hist_tex: texture_2d<f32>;
@group(0) @binding(4) var hist_sampler: sampler;

@fragment
fn main(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(cur_tex));
    let px = vec2<i32>(in.position.xy);
    let c = textureLoad(cur_tex, px, 0);
    if r.reset != 0u { return c; }
    // Untuned: 3x3 neighbourhood clamp (Frostbite 2016 s5.5.3); a wider window ghosts less, a narrower one flickers less.
    var lo = c;
    var hi = c;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let v = textureLoad(cur_tex, clamp(px + vec2(dx, dy), vec2(0), size - 1), 0);
            lo = min(lo, v);
            hi = max(hi, v);
        }
    }
    let t = textureLoad(t_tex, px, 0).r;
    let h = r.ray_from_clip * vec4(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0, 1.0, 1.0);
    let d = normalize(h.xyz / h.w);
    let pc = r.prev_clip_from_ray * vec4(d * select(r.max_distance, t, t > 0.0) + r.delta, 1.0);
    if pc.w <= 0.0 { return c; }
    let puv = vec2(pc.x / pc.w * 0.5 + 0.5, 0.5 - 0.5 * pc.y / pc.w);
    if any(puv < vec2(0.0)) || any(puv > vec2(1.0)) { return c; }
    return mix(clamp(textureSampleLevel(hist_tex, hist_sampler, puv, 0.0), lo, hi), c, r.alpha);
}
