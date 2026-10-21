// Clouds, march pass: renders the cloud layer into a per-view buffer (render world, weather_clouds/render.rs) before the
// opaque pass. Output 0 is premultiplied exposed cloud light (aerial perspective included) with the cloud opacity as alpha;
// output 1 is the cloud depth in metres. clouds_composite.wesl blends it over the sky render_sky drew. Composite math :
//   T_air = transmittance eye -> cloud, S_air = sky-view LUT sky scaled to the eye -> cloud segment,
//   src = shoulder(T_air * L_cloud * exposure) + alpha * S_air * exposure, alpha = 1 - T_cloud (never faded by distance),
// so thick cloud hides the discs at any distance and far cloud converges to the true sky colour.
// The shoulder (config clouds.shoulder, shoulder_limit) is a hue-preserving highlight roll-off on the cloud's own light.
// Lighting: 6-segment light march (25..400 m doubling + one far segment), Nubis in-scatter probability on direct
// light, ambient from the real sky (mean of 5 up-taps of the sky-view LUT, ground bounce below) in place of the old tinted
// sun-coloured term. HZD-style layer (Schneider 2015): weather map
// coverage and cloud type, Perlin-Worley base shape eroded by Worley detail, light marched toward the sun and moon
// through the atmosphere's transmittance LUT, multi-octave scattering (Wrenninge) for silver linings.
// runs at render_scale with a per-frame jittered start (animated IGN); clouds_resolve.wesl accumulates the frames.
// The march is adaptive (HZD 2015): double-length density-only steps until cloud, then full steps, cheap again after 6 empty ones.
import bevy_pbr::atmosphere::{
    bindings::{atmosphere, view},
    bruneton_functions::sample_transmittance_lut_segment,
    functions::{sample_transmittance_lut, sample_sky_view_lut, direction_world_to_atmosphere, uv_to_ray_direction, get_view_position},
};
import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import super::clouds_common::{Clouds, Storms, PI, F16_MAX, remap, hg, phase, gradient, pcg, ign, atmo_transmittance, base_shape_at, map_sample, density_at, StormCell, storm_unpack, storm_reach, storm_profile, storm_col_cell};

@group(1) @binding(0) var<uniform> u: Clouds;
@group(1) @binding(1) var base_tex: texture_3d<f32>;
@group(1) @binding(2) var detail_tex: texture_3d<f32>;
@group(1) @binding(3) var noise_sampler: sampler;
@group(1) @binding(4) var map_tex: texture_2d<f32>;
@group(1) @binding(5) var map_sampler: sampler;
@group(1) @binding(6) var<uniform> storms: Storms;

// ponytail: the literals left in this file (phase g, gradients, Nubis in-scatter constants, octave a/b/c, n*2+8 iteration cap, 5 up-taps at 30 deg) are look-shaping math, untuned; lift into clouds.look as they become knobs.

// (near, far) hits of a ray from `o` along unit `d` with a sphere of radius `r` at the origin; far < 0 = miss.
fn sphere(o: vec3<f32>, d: vec3<f32>, r: f32) -> vec2<f32> {
    let b = dot(o, d);
    let ol = length(o);
    let c = (ol - r) * (ol + r);
    let h = b * b - c;
    if h < 0.0 { return vec2(-1.0); }
    let s = sqrt(h);
    return vec2(-b - s, -b + s);
}

fn map_at(xz: vec2<f32>) -> vec4<f32> { return map_sample(u, map_tex, map_sampler, xz); }
fn density_shell(xz: vec2<f32>, alt: f32, h: f32, detail: bool) -> f32 {
    return density_at(u, base_tex, detail_tex, noise_sampler, map_tex, map_sampler, xz, alt, h, detail, false);
}
// ---- Storm cells read from the `storms` uniform (the same few functions live in clouds_shadow.wesl; WGSL cannot pass a uniform
// pointer, and passing the 1 KB struct by value hung the GPU). ----
fn cell(i: u32) -> StormCell { return storm_unpack(storms.cells[4u * i], storms.cells[4u * i + 1u], storms.cells[4u * i + 2u], storms.cells[4u * i + 3u]); }

// Sum of cell profiles at a point (cells that overlap add: a front of several cells is darker than one).
fn storm_profile_sum(q: vec3<f32>) -> f32 {
    var s = 0.0;
    for (var i = 0u; i < storms.n; i++) {
        let c = cell(i);
        if q.y < c.shelf_low || q.y > c.top + c.overshoot { continue; }
        let dm = q.xz - 0.5 * (c.a + c.b);
        let r = storm_reach(c);
        if dot(dm, dm) > r * r { continue; }
        s += storm_profile(u, c, q.xz, q.y);
    }
    return s;
}

// Optical depth along `dir` (unit) from p, from `start` m for `len` m in `steps` midpoint samples. Coarser than storm_fx::od_along
// (250 m grid) on purpose: this runs per march sample.
fn storm_od(p: vec3<f32>, dir: vec3<f32>, start: f32, len: f32, steps: u32) -> f32 {
    let ds = len / f32(steps);
    var sum = 0.0;
    for (var i = 0u; i < steps; i++) { sum += storm_profile_sum(p + dir * (start + (f32(i) + 0.5) * ds)); }
    return sum * ds * u.s_od_per_m;
}

// Column optical depth up from (xz, y0) through every cell in reach.
fn storm_col(xz: vec2<f32>, y0: f32) -> f32 {
    var od = 0.0;
    for (var i = 0u; i < storms.n; i++) {
        let c = cell(i);
        let dm = xz - 0.5 * (c.a + c.b);
        let r = storm_reach(c);
        if dot(dm, dm) > r * r { continue; }
        od += storm_col_cell(u, c, xz, y0);
    }
    return od;
}

// (density 0..1, height fraction in the densest cell) of the storm cells at world xz, altitude alt. Noise is cell-relative so the
// structure travels with the cell. `detail` runs the detail erosion (wispy underside, billowy top, as the shell does).
fn storm_at(xz: vec2<f32>, alt: f32, detail: bool) -> vec3<f32> {
    var best = vec3(0.0);
    for (var i = 0u; i < storms.n; i++) {
        let c = cell(i);
        if alt < c.shelf_low || alt > c.top + c.overshoot { continue; }
        let dm = xz - 0.5 * (c.a + c.b);
        let r = storm_reach(c);
        if dot(dm, dm) > r * r { continue; }
        let pr = storm_profile(u, c, xz, alt);
        if pr <= 0.0 { continue; }
        let rel = xz - c.a;
        let hs = saturate((alt - c.base) / max(c.top - c.base, 1.0));
        // Flat strata near the base and shelf (vertically squashed lookups), isotropic billows above (no vertical streaks).
        let vsq = mix(u.s_vsq_low, u.s_vsq, smoothstep(0.0, 0.3, hs));
        let n = base_shape_at(u, base_tex, noise_sampler, vec3(rel.x, alt * vsq, rel.y) * u.s_freq);
        var d = saturate((pr - u.s_erode * (1.0 - n)) * u.s_gain);
        if detail && d > 0.0 {
            let dp = vec3(xz.x + u.wind.x, alt, xz.y + u.wind.y) / u.detail_tile;
            let dd = textureSampleLevel(detail_tex, noise_sampler, dp, 0.0);
            let dfbm = dd.r * 0.625 + dd.g * 0.25 + dd.b * 0.125;
            let wisp = mix(dfbm, 1.0 - dfbm, smoothstep(0.1, 0.6, hs));
            d = saturate(remap(d, wisp * u.detail_strength * mix(0.8, 1.6, hs), 1.0, 0.0, 1.0));
        }
        if d > best.x { best = vec3(d, hs, n); }
    }
    return best;
}

// Shell plus storm cells (the light march sees both). With no storms this is exactly the shell, as before.
fn density(xz: vec2<f32>, alt: f32, h: f32, detail: bool) -> f32 {
    if storms.n == 0u { return density_shell(xz, alt, h, detail); }
    var d = 0.0;
    if h >= 0.0 && h <= 1.0 { d = density_shell(xz, alt, h, detail); }
    return max(d, storm_at(xz, alt, detail).x);
}

// Storm cells: rays are culled against each cell's bounding cylinder and altitude slab; the march covers the hull of the hits.
// Range, steps, min step, light-march end, sun chord and column lengths are the s_* uniforms.

// Hull (t start, t end) in ray metres of the ray's passes through the storm cells; end <= start = no storm on this ray.
fn storm_interval(o: vec3<f32>, d: vec3<f32>, R: f32, alt0: f32) -> vec2<f32> {
    var s0 = 1e30;
    var s1 = 0.0;
    for (var i = 0u; i < storms.n; i++) {
        let c = cell(i);
        let reach = storm_reach(c);
        if reach < 1.0 { continue; }
        let lo = c.shelf_low;
        let hi = c.top + c.overshoot;
        var ai = vec2(0.0);
        if alt0 < lo { ai = vec2(sphere(o, d, R + lo).y, sphere(o, d, R + hi).y); }
        else if alt0 < hi {
            let hb = sphere(o, d, R + lo);
            ai = vec2(0.0, select(sphere(o, d, R + hi).y, hb.x, hb.x > 0.0));
        }
        if ai.y <= ai.x { continue; }
        let f = view.world_position.xz - 0.5 * (c.a + c.b);
        let a2 = dot(d.xz, d.xz);
        let b2 = dot(f, d.xz);
        let c2 = dot(f, f) - reach * reach;
        var hr = vec2(0.0, 1e30);
        if a2 < 1e-8 { if c2 > 0.0 { continue; } }
        else {
            let disc = b2 * b2 - a2 * c2;
            if disc < 0.0 { continue; }
            let sq = sqrt(disc);
            hr = vec2((-b2 - sq) / a2, (-b2 + sq) / a2);
        }
        let a0 = max(max(ai.x, hr.x), 0.0);
        let a1 = min(min(ai.y, hr.y), u.s_range);
        if a1 > a0 { s0 = min(s0, a0); s1 = max(s1, a1); }
    }
    return vec2(s0, s1);
}

// Light reaching point `p` (planet-centred) from a body along `l`, scattered toward the eye (cosine `cv`).
// `h` height fraction, `base` cheap density at the sample (Nubis in-scatter probability, untuned constants).
fn body_light(p: vec3<f32>, r: f32, xz: vec2<f32>, alt: f32, l: vec3<f32>, color: vec3<f32>, cv: f32, rb: f32, rt: f32, jit: f32, h: f32, base: f32) -> vec3<f32> {
    if dot(color, color) <= 0.0 { return vec3(0.0); }
    let ta = atmo_transmittance(r, dot(p / r, l));
    if dot(ta, ta) <= 0.0 { return vec3(0.0); }
    // Segments first*2^j, j = 0..4, then one far segment (Frostbite 2016 s5.5.2), sampled near the midpoint (untuned).
    var od = 0.0;
    var start = 0.0;
    for (var j = 0u; j < 6u; j++) {
        let len = select(u.light_first * f32(1u << j), u.light_far, j == 5u);
        let q = l * (start + len * (0.25 + 0.5 * jit));
        let a = alt + q.y;
        od += density(xz + q.xz, a, (a - (rb - atmosphere.inner_radius)) / (rt - rb), false) * len;
        start += len;
    }
    od *= u.extinction;
    var s = 0.0;
    var a = 1.0;
    var b = 1.0;
    var k = 1.0;
    for (var o = 0; o < 3; o++) {
        s += a * exp(-od * b) * phase(cv, k);
        a *= 0.5;
        b *= 0.5;
        k *= 0.5;
    }
    // In-scatter probability (Nubis 2017), strongest looking away from the body; untuned.
    let hc = saturate(h);
    let depth_p = 0.05 + pow(max(base, 1e-4), remap(clamp(hc, 0.3, 0.85), 0.3, 0.85, 0.5, 2.0));
    let vert_p = pow(saturate(remap(clamp(hc, 0.07, 0.14), 0.07, 0.14, 0.1, 1.0)), 0.8);
    let pr = vert_p * mix(1.0, depth_p, saturate(0.5 - 0.5 * cv));
    return color * ta * s * pr;
}

fn ign_f(p: vec2<f32>) -> f32 { return ign(p, u.frame / u32(u.march_div * u.march_div), u.jitter_step); }

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}

// Pixels without cloud: clear in the shipped look, opaque black under a debug probe so the probe reads every pixel.
struct MarchOut {
    @location(0) color: vec4<f32>,
    // Cloud depth in metres, 0 = no cloud.
    @location(1) depth: f32,
}

fn none() -> MarchOut {
    return MarchOut(select(vec4(0.0), vec4(0.0, 0.0, 0.0, 1.0), u.debug != 0u), 0.0);
}

@fragment
fn march(in: FullscreenVertexOutput) -> MarchOut {
    // HZD 4x4: this target is 1/4 x 1/4 of the render size; each texel marches one pixel of its 4x4 block, offset by the frame.
    let fp = floor(in.position.xy) * u.march_div + vec2(u.march_ox, u.march_oy);
    let d = uv_to_ray_direction((fp + 0.5) / vec2(u.full_w, u.full_h));
    let R = atmosphere.inner_radius;
    let alt0 = max(view.world_position.y, 0.0);
    let o = vec3(0.0, R + alt0, 0.0);
    // Below the horizon: the ground (Bevy's planet) is there, nothing of ours shows.
    if sphere(o, d, R).x > 0.0 { return none(); }

    let rb = R + u.bottom;
    let rt = R + u.top;
    var t0 = 0.0;
    var t1 = 0.0;
    if alt0 < u.bottom {
        t0 = sphere(o, d, rb).y;
        t1 = sphere(o, d, rt).y;
    } else if alt0 < u.top {
        let hb = sphere(o, d, rb);
        t1 = select(sphere(o, d, rt).y, hb.x, hb.x > 0.0);
    }
    t1 = min(t1, u.max_distance);
    var si = vec2(1e30, 0.0);
    if storms.n > 0u { si = storm_interval(o, d, R, alt0); }
    let has_sh = t1 > t0;
    let has_st = si.y > si.x;
    let s0 = si.x;
    let s1 = si.y;

    var tr = 1.0;
    var lum = vec3(0.0);
    // Debug split of `lum` into direct sun/moon light and sky ambient (only read when u.debug is set).
    var lum_d = vec3(0.0);
    var lum_a = vec3(0.0);
    // Transmittance-weighted front depth of the cloud (Frostbite 2016 eq. 21): sum(T*t) / sum(T) over in-cloud samples.
    var wsum = 0.0;
    // Storm shade on the air in front of the cloud (opacity-weighted mean sky visibility of the samples; 1 = no storm).
    var av_sum = 0.0;
    var av_w = 0.0;
    var wt = 0.0;
    var thick = false;
    if has_sh || has_st {
        // Low rays cross the shell obliquely: up to horizon_step_boost x the steps near the horizon so far clouds do not smear into step hatching.
        let n = floor(f32(u.steps) * mix(u.horizon_step_boost, 1.0, smoothstep(0.0, u.horizon_step_y, d.y)));
        let dt = (t1 - t0) / n;
        let cs = dot(d, u.sun_dir);
        let cm = dot(d, u.moon_dir);
        let jit = ign_f(fp);
        // The sun/moon light march gets its OWN jitter. Sharing `jit` correlates the light-sample offset with the eye-sample altitude
        // offset, so the average over jitter keeps a bias that varies with altitude: horizontal streak lines in lit cloud bodies (more steps hide them).
        // resolve.light_jitter_shared = true restores the shared jitter (test red arm).
        let jl = select(ign_f(fp + u.light_jitter_offset), jit, u.light_jitter_shared > 0.5);
        // Sky ambient : L_top = mean of 5 up-taps of the sky-view LUT at the eye, L_gnd = ground bounce of the bodies + sky.
        let ce = get_view_position();
        let re = length(ce);
        var l_top = sample_sky_view_lut(re, direction_world_to_atmosphere(vec3(0.0, 1.0, 0.0)));
        for (var q = 0u; q < 4u; q++) {
            let az = f32(q) * 1.5707963;
            // 30 deg elevation (untuned tap layout).
            l_top += sample_sky_view_lut(re, direction_world_to_atmosphere(vec3(0.866 * cos(az), 0.5, 0.866 * sin(az))));
        }
        l_top *= 0.2;
        let e_gnd = u.sun_color * atmo_transmittance(R, u.sun_dir.y) * max(u.sun_dir.y, 0.0)
            + u.moon_color * atmo_transmittance(R, u.moon_dir.y) * max(u.moon_dir.y, 0.0) + PI * l_top;
        let l_gnd = atmosphere.ground_albedo / PI * e_gnd;
        let ds = max((s1 - s0) / u.s_steps, u.s_ds_min);
        var t = select(t0 + dt * jit, s0 + ds * jit, has_st && (!has_sh || s0 < t0));
        let t_end = t0 + dt * n;
        var cheap = true;
        var empty = 0u;
        let cap = select(0u, u32(n) * 2u + 8u, has_sh) + select(0u, u32(u.s_steps) * 2u + 8u, has_st);
        for (var i = 0u; i < cap; i++) {
            if tr < 0.01 { thick = true; break; }
            let in_sh = has_sh && t >= t0 && t < t_end;
            let in_st = has_st && t >= s0 && t < s1;
            if !in_sh && !in_st {
                // Between intervals: jump to the next one ahead, else done.
                var nx = 1e30;
                if has_sh && t < t0 { nx = t0 + dt * jit; }
                if has_st && t < s0 { nx = min(nx, s0 + ds * jit); }
                if nx > 1e29 { break; }
                t = nx;
                continue;
            }
            let p = o + d * t;
            let r = length(p);
            let h = (r - rb) / (rt - rb);
            let xz = view.world_position.xz + d.xz * t;
            let alt = r - R;
            let inside = h >= 0.0 && h <= 1.0;
            if cheap && !in_st {
                // Density without detail: if any, back up one full step and march properly from there.
                if inside && density_shell(xz, alt, h, false) > 0.0 { cheap = false; empty = 0u; t = max(t - dt, t0); } else { t += dt; } // a 2*dt coarse stride skipped thin layers by altitude, which showed as horizontal bands
                continue;
            }
            var dens = 0.0;
            if in_sh && inside { dens = density_shell(xz, alt, h, true); }
            var sv = vec3(0.0);
            if in_st { sv = storm_at(xz, alt, t < u.s_detail_m); }
            let storm_dom = sv.x > dens;
            dens = max(dens, sv.x);
            if dens > 0.0 {
                empty = 0u;
                let sigma = dens * u.extinction;
                var sd = vec3(0.0);
                var sa = vec3(0.0);
                var air_vis = 1.0;
                if storm_dom {
                    // Black bases: the local light march sees the cell's own density; beyond it the sun is cut by the analytic
                    // chord, the sky ambient by the column above, and the ground bounce by the storm's shadow on the ground below.
                    let hs = sv.y;
                    let base = saturate(storm_at(xz, alt, false).x);
                    let q = vec3(xz.x, alt, xz.y);
                    sd = body_light(p, r, xz, alt, u.sun_dir, u.sun_color, cs, rb, rt, jl, hs, base)
                        * exp(-storm_od(q, u.sun_dir, u.s_near, u.s_sun_len, 6u));
                    if dot(u.moon_color, u.moon_color) > 0.0 {
                        sd += body_light(p, r, xz, alt, u.moon_dir, u.moon_color, cm, rb, rt, jl, hs, base)
                            * exp(-storm_od(q, u.moon_dir, u.s_near, u.s_sun_len, 6u));
                    }
                    // Slate, not black: a floor on the sky visibility, varied by the cell's own noise.
                    let amb_vis = max(exp(-u.s_amb_k * storm_col(xz, alt)), u.s_amb_floor) * (1.0 - u.s_amb_var + 2.0 * u.s_amb_var * sv.z);
                    let gnd_vis = max(exp(-storm_od(vec3(xz.x, 0.0, xz.y), u.sun_dir, 0.0, u.s_sun_len, 4u) - u.s_amb_k * storm_col(xz, 0.0)), u.s_amb_floor);
                    sa = 0.5 * mix(l_gnd * gnd_vis, l_top * amb_vis, hs) * u.ambient_scale;
                    air_vis = amb_vis;
                } else {
                    let base = saturate(density_shell(xz, alt, h, false));
                    sd = body_light(p, r, xz, alt, u.sun_dir, u.sun_color, cs, rb, rt, jl, h, base)
                        + body_light(p, r, xz, alt, u.moon_dir, u.moon_color, cm, rb, rt, jl, h, base);
                    // Sky light: one hemisphere, isotropic; ground bounce below, the real sky above (rising with height).
                    sa = 0.5 * mix(l_gnd, l_top, saturate(h)) * u.ambient_scale;
                    if storms.n > 0u {
                        // Field cloud under or near a storm sits in its shadow: the same chord and column as the storm's own samples.
                        let q = vec3(xz.x, alt, xz.y);
                        // Light floor (bisect 2026-10-20: no floor made field clouds black slabs under an anvil). Untuned.
                        let av = max(exp(-u.s_amb_k * storm_col(xz, alt)), u.s_field_floor);
                        sd *= max(exp(-storm_od(q, u.sun_dir, u.s_near, u.s_sun_len, 6u)), u.s_field_floor);
                        sa *= av;
                        air_vis = av;
                    }
                }
                let s = sd * u.light_gain + sa;
                let ts = exp(-sigma * dt);
                av_sum += tr * (1.0 - ts) * air_vis;
                av_w += tr * (1.0 - ts);
                // Energy-conserving integration over the step (Hillaire 2016).
                let e = tr * (s - s * ts);
                lum += e;
                if u.debug != 0u {
                    lum_d += tr * (sd * u.light_gain - sd * u.light_gain * ts);
                    lum_a += tr * (sa - sa * ts);
                }
                wsum += tr;
                wt += tr * t;
                tr *= ts;
            } else {
                empty++;
                if empty >= u32(u.cheap_empty_steps) && !in_st { cheap = true; }
            }
            var adv = select(ds, dt, in_sh);
            if !in_sh && has_sh && t < t0 { adv = min(adv, t0 - t + 1.0); }
            t += adv;
        }
        if tr < 0.01 { thick = true; }
    }
    if wsum <= 0.0 { return none(); }
    let tc = wt / wsum;
    let alpha = select(1.0 - tr, 1.0, thick);

    // Air between the eye and the cloud: transmittance, and the sky-view LUT sky scaled to that segment.
    let c = get_view_position();
    let rc = length(c);
    let mu = dot(d, c / rc);
    let t_air = sample_transmittance_lut_segment(rc, mu, tc);
    let t_inf = sample_transmittance_lut(rc, mu);
    let s_sky = sample_sky_view_lut(rc, direction_world_to_atmosphere(d));
    let s_air = min(s_sky * (vec3(1.0) - t_air) / max(vec3(1.0) - t_inf, vec3(1e-4)), s_sky);

    var rgb = lum * t_air * view.exposure;
    if u.debug != 0u {
        // Probes (grey, opaque). 1: exposed max channel before the shoulder, /8. 2: direct sun/moon light, /8.
        // 3: sky ambient, /8 (both luminance). 4: cloud alpha. 5: cloud depth over max_distance.
        var v = 0.0;
        if u.debug == 1u { v = max(rgb.r, max(rgb.g, rgb.b)) / 8.0; }
        else if u.debug == 2u { v = luma(lum_d * t_air * view.exposure) / 8.0; }
        else if u.debug == 3u { v = luma(lum_a * t_air * view.exposure) / 8.0; }
        else if u.debug == 4u { v = alpha; }
        else { v = tc / u.max_distance; }
        return MarchOut(vec4(vec3(v), 1.0), tc);
    }
    // Highlight shoulder on the cloud's own exposed light only: compress the max channel, keep the hue.
    let m = max(rgb.r, max(rgb.g, rgb.b));
    if m > u.shoulder {
        let w = u.shoulder_limit - u.shoulder;
        let x = m - u.shoulder;
        rgb *= (u.shoulder + w * x / (x + w)) / m;
    }
    rgb += alpha * s_air * view.exposure * select(1.0, av_sum / max(av_w, 1e-6), storms.n > 0u);
    return MarchOut(vec4(min(rgb, vec3(F16_MAX)), alpha), tc);
}
