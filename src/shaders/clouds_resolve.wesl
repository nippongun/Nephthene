// Clouds resolve: HZD update (div x div, default 4x4). The march target is 1/4 x 1/4 of the render size and covers one pixel of each
// 4x4 block per frame (offset (ox, oy), a 16-frame cycle). Here, at render size: that pixel takes the fresh sample; every other pixel keeps
// last frame's resolved buffer reprojected through the previous view at the cloud's own depth (max_distance where there is no cloud),
// clamped to the low-res neighbourhood (Frostbite 2016 s5.5.3). A reset (first frame, resized view, skipped frame) and off-screen
// history take the bilinear upsample of the low-res march. alpha >= 1 (config clouds.temporal_alpha = 1, test red arm) = no history at all.
import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct Resolve {
    prev_clip_from_ray: mat4x4<f32>,
    ray_from_clip: mat4x4<f32>,
    delta: vec3<f32>,
    alpha: f32,
    max_distance: f32,
    reset: u32,
    ox: u32,
    oy: u32,
    noreproj: u32,
    div: u32,
    // Frames since the last reset: a pixel marched once per div*div frames has been visited n / (div*div) times before this one.
    n: u32,
    // Tuning (config resolve.*): see nephthene_core::config::Resolve.
    reject_gain: f32,
    still_px: f32,
    fresh: f32,
    conf_max: f32,
    upsample_mean_frames: f32,
    // Reprojection motion (render px) that halves a marched pixel's counted visits (0 = off; test red arm), and 1 = wide upsample for no-history pixels.
    motion_half_px: f32,
    wide_upsample: f32,
}

@group(0) @binding(0) var<uniform> r: Resolve;
@group(0) @binding(1) var cur_tex: texture_2d<f32>;
@group(0) @binding(2) var t_tex: texture_2d<f32>;
@group(0) @binding(3) var hist_tex: texture_2d<f32>;
@group(0) @binding(4) var hist_sampler: sampler;
// Per-pixel confidence how many samples this pixel's history has absorbed (0 = fresh: reset, off-screen, rejected). Ping-pong with hist.
@group(0) @binding(5) var conf_tex: texture_2d<f32>;

struct Out { @location(0) color: vec4<f32>, @location(1) conf: f32 }
// Constants are the uniform fields reject_gain (rejection gain), still_px (5x5 box below this displacement), fresh (confidence of a pixel revealed mid-motion: its first marched visit weighs 1/(1+fresh); weight 1 made a dot lattice on a turning view), conf_max, upsample_mean_frames (an unmarched pixel with confidence below this averages in each frame's upsample; 0 = never).

@fragment
fn main(in: FullscreenVertexOutput) -> Out {
    let low = vec2<i32>(textureDimensions(cur_tex));
    let px = vec2<i32>(in.position.xy);
    let o = vec2<i32>(i32(r.ox), i32(r.oy));
    let div = i32(r.div);
    let lpx = clamp(px / div, vec2(0), low - 1);
    let selected = all(px % div == o);
    // Bilinear low-res value at this pixel: texel i holds the pixel 4i + o.
    let luv = ((vec2<f32>(px - o)) / f32(div) + 0.5) / vec2<f32>(low);
    let up = textureSampleLevel(cur_tex, hist_sampler, luv, 0.0);
    let c = select(up, textureLoad(cur_tex, lpx, 0), selected);
    // Wide upsample for a pixel with no history (revealed mid-motion): 4 bilinear taps half a low-res texel apart. Each texel is ONE jittered sample, so the
    // plain bilinear value is 8 px blotches of sample noise; this averages up to 16 texels.
    let h2 = 0.5 / vec2<f32>(low);
    let wide = 0.25 * (textureSampleLevel(cur_tex, hist_sampler, luv + vec2(h2.x, h2.y), 0.0) + textureSampleLevel(cur_tex, hist_sampler, luv + vec2(-h2.x, h2.y), 0.0)
        + textureSampleLevel(cur_tex, hist_sampler, luv + vec2(h2.x, -h2.y), 0.0) + textureSampleLevel(cur_tex, hist_sampler, luv + vec2(-h2.x, -h2.y), 0.0));
    let cw = select(select(up, wide, r.wide_upsample != 0.0), c, selected);
    if r.reset != 0u { return Out(c, select(0.0, 1.0, selected)); }
    // No history (G3 red arm): only the 1/16 pixels marched this frame exist, the rest are empty, so the 4x4 update shows as a lattice.
    if r.alpha >= 1.0 { return Out(select(vec4(0.0), c, selected), 0.0); }
    let t = textureLoad(t_tex, lpx, 0).r;
    let h = r.ray_from_clip * vec4(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0, 1.0, 1.0);
    let d = normalize(h.xyz / h.w);
    let pc = r.prev_clip_from_ray * vec4(d * select(r.max_distance, t, t > 0.0) + r.delta, 1.0);
    if pc.w <= 0.0 { return Out(cw, select(r.fresh, r.fresh + 1.0, selected)); }
    var puv = vec2(pc.x / pc.w * 0.5 + 0.5, 0.5 - 0.5 * pc.y / pc.w);
    if r.noreproj != 0u { puv = in.uv; }
    if any(puv < vec2(0.0)) || any(puv > vec2(1.0)) { return Out(cw, select(r.fresh, r.fresh + 1.0, selected)); }
    let conf = textureLoad(conf_tex, clamp(vec2<i32>(puv * vec2<f32>(textureDimensions(conf_tex))), vec2(0), vec2<i32>(textureDimensions(conf_tex)) - 1), 0).r;
    let raw = textureSampleLevel(hist_tex, hist_sampler, puv, 0.0);
    // Box over the low-res samples around this pixel: 3x3 while the view moves (tight, kills ghosts), 5x5 when it is still (each
    // texel is ONE jittered sample of a grainy cloud, so a 3x3 box over thin cloud often holds only misses, lo = hi = empty sky, and clamping
    // every frame pinned the whole 4x4 block to that bound: flat 8 px squares that never converged with a still camera). A still view
    // reprojects exactly, so history needs no tight bound; blend by how far (low-res px) the reprojection moved this pixel.
    var lo = up;
    var hi = up;
    var lo5 = up;
    var hi5 = up;
    for (var dy = -2; dy <= 2; dy++) {
        for (var dx = -2; dx <= 2; dx++) {
            let v = textureLoad(cur_tex, clamp(lpx + vec2(dx, dy), vec2(0), low - 1), 0);
            lo5 = min(lo5, v);
            hi5 = max(hi5, v);
            if abs(dx) <= 1 && abs(dy) <= 1 { lo = min(lo, v); hi = max(hi, v); }
        }
    }
    let still = 1.0 - saturate(length((puv - in.uv) * vec2<f32>(low)) / r.still_px);
    lo = mix(lo, lo5, still);
    hi = mix(hi, hi5, still);
    let hist = clamp(raw, lo, hi);
    // The pixels NOT marched this frame keep their reprojected history (HZD): no blend toward the upsample.
    // Unmarched pixel: history. In the div*div frames after a reset (a cut: every pixel is fresh) it takes the running mean of the per-frame upsamples
    // (different offset each frame) instead, so a cut or teleport fills in within one cycle, not over its marched visits at weight alpha. Global count n,
    // not per pixel: a per-pixel mean blurred a moving view for 16 frames after every screen-edge reveal (golden sway).
    if !selected {
        let ramp = r.n < u32(div * div);
        if ramp { return Out(mix(hist, up, 1.0 / (f32(r.n) + 1.0)), max(conf, 1.0)); }
        // A revealed pixel (conf < upsample_mean_frames) absorbs this frame's upsample as one more sample, running mean 1/(1+conf), instead of holding its first one.
        let fill = conf < r.upsample_mean_frames;
        return Out(select(hist, mix(hist, up, 1.0 / (conf + 1.0)), fill), select(conf, conf + 1.0, fill));
    }
    // The marched pixel: running mean 1/(1+conf) of this pixel's own samples, floor alpha (cap); a rejected history restarts it.
    let rej = saturate(length(raw - hist) * r.reject_gain);
    let moved = length((puv - in.uv) * vec2<f32>(textureDimensions(hist_tex)));
    let visits = select(conf, conf / (1.0 + moved / max(r.motion_half_px, 1e-6)), r.motion_half_px > 0.0);
    let w = max(1.0 / (1.0 + visits), r.alpha);
    return Out(mix(hist, c, mix(w, 1.0, rej)), min(conf + 1.0, r.conf_max));
}
