// rain shafts (virga): a rain medium from the cloud base down to the terrain, marched per pixel and drawn after the cloud composite
// as a Transparent3d item. Its fragment depth is the first shaft sample along the ray, so nearer terrain/objects occlude it and farther
// ones are dimmed and greyed by it (the depth test replaces the cloud composite's sky-only rule). Maths: nephthene_core::rain (shaft_*).
// The cloud presence under a column is the march's own base density (kept in sync with clouds_march.wesl `density`, no detail pass).
// Storm cells add their rain curtain (storm_fx::precip_at) where it is wetter than the map, fed by the storm's own cloud.
// Column: rain at (xz, y) left the base upwind by the lean (`column_origin`); gated by the map's precip and that cloud, streaked by the
// base noise stretched vertically, thinner toward the ground, lit by the sun through the atmosphere and the cloud above. All untuned.
import bevy_pbr::atmosphere::{
    bindings::{atmosphere, view},
    bruneton_functions::sample_transmittance_lut_segment,
    functions::{sample_transmittance_lut, sample_sky_view_lut, direction_world_to_atmosphere, uv_to_ray_direction, get_view_position},
};
import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import super::clouds_common::{Clouds, Storms, F16_MAX, PI, hg, phase, pcg, atmo_transmittance, map_sample, density_at, storm_unpack, storm_precip};

struct Shaft {
    slant: vec2<f32>,   // horizontal m per vertical m the rain drifts
    scale: f32,         // 0 = off
    g_cell: f32,
    g_min: vec2<f32>,   // ground grid west x, north z
    g_w: u32,
    g_h: u32,
    // config rain.precip_lo/hi and shafts.* (nephthene_core::rain mirrors the shaft_density maths).
    precip_lo: f32, precip_hi: f32, sigma: f32, near_m: f32,
    fade_m: f32, max_m: f32, xz_m: f32, y_m: f32,
    ground_keep: f32, cloud_lo: f32, cloud_hi: f32, streak_lo: f32,
    streak_hi: f32, gap: f32, albedo: f32, light_od: f32,
    steps: f32,
}

@group(1) @binding(0) var<uniform> u: Clouds;
@group(1) @binding(1) var base_tex: texture_3d<f32>;
@group(1) @binding(2) var detail_tex: texture_3d<f32>;
@group(1) @binding(3) var noise_sampler: sampler;
@group(1) @binding(4) var map_tex: texture_2d<f32>;
@group(1) @binding(5) var map_sampler: sampler;
@group(1) @binding(6) var<uniform> storms: Storms;
@group(1) @binding(7) var oro_tex: texture_2d<f32>;
@group(1) @binding(8) var oro_sampler: sampler;
@group(2) @binding(0) var<uniform> sh: Shaft;
@group(2) @binding(1) var ground: texture_2d<f32>;

// The shaft constants are the Shaft uniform fields (config shafts.*), mirroring nephthene_core::rain.

fn map_at(xz: vec2<f32>) -> vec4<f32> { return map_sample(u, map_tex, map_sampler, oro_tex, oro_sampler, xz); }

fn gtex(i: i32, j: i32) -> f32 {
    return textureLoad(ground, vec2<i32>(clamp(i, 0, i32(sh.g_w) - 1), clamp(j, 0, i32(sh.g_h) - 1)), 0).r;
}
// Mirrors nephthene_core::rain::HeightGrid::height.
fn ground_h(xz: vec2<f32>) -> f32 {
    let f = clamp(vec2<f32>((xz.x - sh.g_min.x) / sh.g_cell - 0.5, (sh.g_min.y - xz.y) / sh.g_cell - 0.5), vec2<f32>(0.0), vec2<f32>(f32(sh.g_w) - 1.0, f32(sh.g_h) - 1.0));
    let i = vec2<i32>(floor(f));
    let t = f - floor(f);
    let a = mix(gtex(i.x, i.y), gtex(i.x + 1, i.y), t.x);
    let b = mix(gtex(i.x, i.y + 1), gtex(i.x + 1, i.y + 1), t.x);
    return mix(a, b, t.y);
}

// clouds_march.wesl `density(.., detail = false)`.
fn density(xz: vec2<f32>, alt: f32, h: f32) -> f32 {
    return density_at(u, base_tex, detail_tex, noise_sampler, map_tex, map_sampler, oro_tex, oro_sampler, xz, alt, h, false, false);
}

fn smooth01(lo: f32, hi: f32, x: f32) -> f32 { return smoothstep(lo, hi, x); }

// nephthene_core::rain::shaft_density.
fn shaft_density(precip: f32, cloud: f32, streak: f32, hf: f32) -> f32 {
    let gate = smoothstep(sh.precip_lo, sh.precip_hi, precip) * smoothstep(sh.cloud_lo, sh.cloud_hi, cloud);
    let streaks = sh.gap + (1.0 - sh.gap) * smoothstep(sh.streak_lo, sh.streak_hi, streak);
    return gate * streaks * (sh.ground_keep + (1.0 - sh.ground_keep) * clamp(hf, 0.0, 1.0));
}

// W4a: marched into half-res buffers before the opaque pass (render.rs), resolved over time (clouds_shaft_resolve.wesl) and drawn by
// clouds_shaft_comp.wesl, which supplies the per-pixel depth from `t_front` (metres along the ray, 0 = no shaft).
struct Out {
    @location(0) color: vec4<f32>,
    @location(1) t_front: f32,
}

@fragment
fn main(in: FullscreenVertexOutput) -> Out {
    let d = uv_to_ray_direction(in.uv);
    let eye = view.world_position;
    let base_y = u.bottom;
    let thick = u.top - u.bottom;
    // Ray ends at the cloud base, the max distance or the terrain (below).
    var t_end = sh.max_m;
    if d.y > 1e-4 { t_end = clamp((base_y - eye.y) / d.y, 0.0, sh.max_m); }
    if sh.scale <= 0.0 || eye.y >= base_y || t_end <= sh.near_m { return Out(vec4(0.0), 0.0); }

    let R = atmosphere.inner_radius;
    let ce = get_view_position();
    let re = length(ce);
    let l_top = sample_sky_view_lut(re, direction_world_to_atmosphere(vec3(0.0, 1.0, 0.0)));
    let sa = 0.5 * l_top * u.ambient_scale;
    let cs = dot(d, u.sun_dir);
    let sun_up = u.sun_dir.y > 0.02;
    // White hash with a per-frame term: the temporal resolve averages it away (IGN's diagonal lattice read as hatching on the ridges).
    let jit = pcg(u32(in.position.x) + 4096u * u32(in.position.y) + u.frame * 16777619u);

    var tr = 1.0;
    var lum = vec3(0.0);
    var wsum = 0.0;
    var wt = 0.0;
    var t_front = -1.0;
    var t_prev = sh.near_m;
    for (var i = 0.0; i < sh.steps; i += 1.0) {
        let f = (i + 1.0 + jit - 0.5) / sh.steps;
        let t = sh.near_m + (t_end - sh.near_m) * f * f;
        let dt = max(t - t_prev, 0.0);
        t_prev = t;
        let p = eye + d * t;
        let g = ground_h(p.xz);
        if p.y < g { break; }
        let hf = (p.y - g) / max(base_y - g, 1.0);
        let xz0 = p.xz - sh.slant * (base_y - p.y);
        // Rain at the point is what the map says it is here (the ground precip); the cloud that feeds it is upwind at the base.
        var storm = 0.0;
        if (u.s_couple > 0.5) {
            for (var k = 0u; k < storms.n; k++) {
                let c = storm_unpack(storms.cells[4u * k], storms.cells[4u * k + 1u], storms.cells[4u * k + 2u], storms.cells[4u * k + 3u]);
                storm = max(storm, storm_precip(u, c, p.xz, p.y));
            }
        }
        let precip = max(select(map_at(p.xz).g, u.precip_override, u.precip_override >= 0.0), storm);
        if precip <= sh.precip_lo { continue; }
        let cloud = select(saturate(density(xz0, base_y + 0.08 * thick, 0.08)), 1.0, storm > sh.precip_lo);
        let streak = textureSampleLevel(base_tex, noise_sampler, vec3(xz0.x + u.wind.x, p.y, xz0.y + u.wind.y) / vec3(sh.xz_m, sh.y_m, sh.xz_m), 0.0).r;
        let dens = shaft_density(precip, cloud, streak, hf) * smoothstep(sh.near_m, sh.fade_m, t);
        if dens <= 0.0 { continue; }
        if t_front < 0.0 { t_front = t; }
        var sun = vec3(0.0);
        if sun_up {
            // The sun ray crosses the cloud layer above this point: two density taps, mid-low and mid-high.
            let sl = u.sun_dir.xz / u.sun_dir.y;
            let a1 = u.bottom + 0.3 * thick;
            let a2 = u.bottom + 0.7 * thick;
            let d1 = density(p.xz + sl * (a1 - p.y), a1, 0.3);
            let d2 = density(p.xz + sl * (a2 - p.y), a2, 0.7);
            let od = (d1 + d2) * 0.5 * u.extinction * thick * 0.5 / max(u.sun_dir.y, 0.3) * sh.light_od;
            let tcl = mix(exp(-od), exp(-0.2 * od), 0.5);
            sun = u.sun_color * atmo_transmittance(R + max(p.y, 0.0), u.sun_dir.y) * tcl * phase(cs, 1.0) * u.light_gain;
        }
        let s = sh.albedo * (sun + sa);
        let ts = exp(-dens * sh.sigma * dt);
        lum += tr * (s - s * ts);
        wsum += tr;
        wt += tr * t;
        tr *= ts;
        if tr < 0.01 { break; }
    }
    if wsum <= 0.0 { return Out(vec4(0.0), 0.0); }
    let tc = wt / wsum;
    let alpha = 1.0 - tr;

    let rc = length(ce);
    let mu = dot(d, ce / rc);
    let t_air = sample_transmittance_lut_segment(rc, mu, tc);
    let t_inf = sample_transmittance_lut(rc, mu);
    let s_sky = sample_sky_view_lut(rc, direction_world_to_atmosphere(d));
    let s_air = min(s_sky * (vec3(1.0) - t_air) / max(vec3(1.0) - t_inf, vec3(1e-4)), s_sky);
    var rgb = lum * t_air * view.exposure;
    let mx = max(rgb.r, max(rgb.g, rgb.b));
    if mx > u.shoulder {
        let w = u.shoulder_limit - u.shoulder;
        let x = mx - u.shoulder;
        rgb *= (u.shoulder + w * x / (x + w)) / mx;
    }
    rgb += alpha * s_air * view.exposure;
    return Out(vec4(min(rgb, vec3(F16_MAX)), alpha), t_front);
}
