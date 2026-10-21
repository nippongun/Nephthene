// Ground fog (nephthene_core::fog): marched per pixel after the main pass, blended over the scene (premultiplied: rgb + dst * (1 - a)).
// The ray runs from the eye to the scene depth (or max_m for sky) clipped to the altitude band [y_lo, y_hi] that holds fog. Density is
// `fog_density` (nephthene_core::fog::density) from the baked fog map (pool top, shallow mist, sunlit altitude) and the terrain height,
// broken up by value noise. Light: sun through the atmosphere, the cloud shadow cookie, the terrain (baked sunlit altitude) and the fog
// above (optical depth to the top, multiple-scattering octaves); the moon the same without terrain shadow; sky light from the mean sky-view
// radiance, half from above (dimmed by the fog above) and half reflected from below (fog layer and ground, two-stream). Aerial perspective as clouds_shaft.wesl. Every constant comes from config ground_fog.
import bevy_pbr::atmosphere::{
    bindings::{atmosphere, view},
    bruneton_functions::sample_transmittance_lut_segment,
    functions::{sample_transmittance_lut, sample_sky_view_lut, sample_aerial_view_lut, direction_world_to_atmosphere, uv_to_ray_direction, uv_to_ndc, get_view_position},
};
import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct Fog {
    sun_dir: vec3<f32>,
    sigma: f32,
    sun_color: vec3<f32>,
    sigma_shallow: f32,
    moon_dir: vec3<f32>,
    albedo: f32,
    moon_color: vec3<f32>,
    g: f32,
    map_min: vec2<f32>,
    map_cell: f32,
    map_n: u32,
    g_min: vec2<f32>,
    g_cell: f32,
    g_w: u32,
    g_h: u32,
    steps: u32,
    frame: u32,
    max_m: f32,
    y_lo: f32,
    y_hi: f32,
    top_soft: f32,
    edge_m: f32,
    top_wave: f32,
    wave_tile: f32,
    shallow_e: f32,
    lift_max: f32,
    lift_soft: f32,
    breakup: f32,
    breakup_tile: f32,
    breakup_y: f32,
    wind: vec2<f32>,
    sun_soft: f32,
    ambient_scale: f32,
    ambient_od: f32,
    ms_a: f32,
    ms_b: f32,
    ms_octaves: u32,
    sh_origin: vec3<f32>,
    sh_half: f32,
    sh_right: vec3<f32>,
    sh_on: f32,
    sh_up: vec3<f32>,
    edge_fade: f32,
    sun_k: f32,
    ground_albedo: f32,
    wisp_m: f32,
    wisp_erode: f32,
    diffuse_sun: f32,
    step_power: f32,
    // Terrain sun occlusion of the aerial in-scatter (config sky.sun_occlusion): on, kept share, samples per ray.
    occ_on: f32,
    occ_keep: f32,
    occ_steps: u32,
}

@group(1) @binding(0) var<uniform> f: Fog;
@group(1) @binding(1) var fog_map: texture_2d<f32>;
@group(1) @binding(2) var ground: texture_2d<f32>;
@group(1) @binding(3) var cookie: texture_2d<f32>;
@group(1) @binding(4) var smp: sampler;
@if(MULTISAMPLED)
@group(1) @binding(5) var depth: texture_depth_multisampled_2d;
@if(!MULTISAMPLED)
@group(1) @binding(5) var depth: texture_depth_2d;

const PI: f32 = 3.14159265;
const F16_MAX: f32 = 60000.0;

fn hg(c: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (4.0 * PI * pow(max(1.0 + g2 - 2.0 * g * c, 1e-4), 1.5));
}

fn pcg(v: u32) -> f32 {
    var h = v * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return f32((h >> 22u) ^ h) / 4294967295.0;
}

// Value noise 0..1 on the integer lattice, smooth-step interpolated.
fn lattice(p: vec3<i32>) -> f32 {
    return pcg((u32(p.x) * 73856093u) ^ (u32(p.y) * 19349663u) ^ (u32(p.z) * 83492791u));
}
fn vnoise(p: vec3<f32>) -> f32 {
    let i = vec3<i32>(floor(p));
    let t = fract(p);
    let s = t * t * (3.0 - 2.0 * t);
    let x00 = mix(lattice(i), lattice(i + vec3(1, 0, 0)), s.x);
    let x10 = mix(lattice(i + vec3(0, 1, 0)), lattice(i + vec3(1, 1, 0)), s.x);
    let x01 = mix(lattice(i + vec3(0, 0, 1)), lattice(i + vec3(1, 0, 1)), s.x);
    let x11 = mix(lattice(i + vec3(0, 1, 1)), lattice(i + vec3(1, 1, 1)), s.x);
    return mix(mix(x00, x10, s.y), mix(x01, x11, s.y), s.z);
}

fn atmo_transmittance(r: f32, mu: f32) -> vec3<f32> {
    let R = atmosphere.inner_radius;
    if mu < -sqrt(max(1.0 - (R / r) * (R / r), 0.0)) { return vec3(0.0); }
    return sample_transmittance_lut(r, mu);
}

// Bilinear texel of the fog map (nephthene_core::fog::FogMap::at): (pool top, shallow, sunlit altitude, 0).
fn mtex(i: i32, j: i32) -> vec4<f32> {
    let n = i32(f.map_n) - 1;
    return textureLoad(fog_map, vec2<i32>(clamp(i, 0, n), clamp(j, 0, n)), 0);
}
fn map_at(xz: vec2<f32>) -> vec4<f32> {
    let m = f32(f.map_n) - 1.0;
    let q = clamp(vec2((xz.x - f.map_min.x) / f.map_cell - 0.5, (f.map_min.y - xz.y) / f.map_cell - 0.5), vec2(0.0), vec2(m));
    let i = vec2<i32>(floor(q));
    let t = q - floor(q);
    return mix(mix(mtex(i.x, i.y), mtex(i.x + 1, i.y), t.x), mix(mtex(i.x, i.y + 1), mtex(i.x + 1, i.y + 1), t.x), t.y);
}

// Mirrors nephthene_core::rain::HeightGrid::height (clouds_shaft.wesl ground_h).
fn gtex(i: i32, j: i32) -> f32 {
    return textureLoad(ground, vec2<i32>(clamp(i, 0, i32(f.g_w) - 1), clamp(j, 0, i32(f.g_h) - 1)), 0).r;
}
fn ground_h(xz: vec2<f32>) -> f32 {
    let q = clamp(vec2((xz.x - f.g_min.x) / f.g_cell - 0.5, (f.g_min.y - xz.y) / f.g_cell - 0.5), vec2(0.0), vec2(f32(f.g_w) - 1.0, f32(f.g_h) - 1.0));
    let i = vec2<i32>(floor(q));
    let t = q - floor(q);
    return mix(mix(gtex(i.x, i.y), gtex(i.x + 1, i.y), t.x), mix(gtex(i.x, i.y + 1), gtex(i.x + 1, i.y + 1), t.x), t.y);
}

// nephthene_core::fog::lit.
fn lit(y: f32, sun_alt: f32) -> f32 { return smoothstep(sun_alt - f.sun_soft, sun_alt + f.sun_soft, y); }

// nephthene_core::fog::density: texel = (pool top, shallow, sunlit altitude, amount). The noise `n` erodes the pool into wisps toward
// its top; the base lifts only where the sun reaches the top.
fn fog_density(texel: vec4<f32>, g: f32, y: f32, wave: f32, n: f32) -> f32 {
    let h = y - g;
    if h < 0.0 { return 0.0; }
    let top = texel.x + f.top_wave * wave;
    // ponytail: 0.2 is the noise band a wisp edge fades over (nephthene_core::fog::density).
    let thr = f.wisp_erode * (1.0 - clamp((top - y) / max(f.wisp_m, 1e-3), 0.0, 1.0));
    let wisp = select(1.0, smoothstep(thr, thr + 0.2, n), f.wisp_m > 0.0);
    let pool = smoothstep(-0.5, 0.5, (top - y) / max(f.top_soft, 1e-3)) * smoothstep(0.0, f.edge_m, top - g) * smoothstep(0.0, 0.5, texel.w) * wisp;
    let mist = texel.y * exp(-h / max(f.shallow_e, 1e-3));
    let lift = f.lift_max * f.sun_k * lit(max(texel.x, g), texel.z) * (1.0 - clamp(texel.w, 0.0, 1.0));
    let base = select(1.0, smoothstep(lift, lift + f.lift_soft, h), lift > 0.0);
    return (f.sigma * pool + f.sigma_shallow * mist) * base;
}

// Cloud shadow cookie (nephthene_core shadow_*): sun transmittance through the cloud layer above p, 1 when there is no cookie.
fn cloud_shadow(p: vec3<f32>) -> f32 {
    if f.sh_on <= 0.0 { return 1.0; }
    let l = vec2(dot(p - f.sh_origin, f.sh_right), dot(p - f.sh_origin, f.sh_up)) / f.sh_half;
    if any(abs(l) > vec2(1.0)) { return 1.0; }
    return textureSampleLevel(cookie, smp, vec2(0.5 - 0.5 * l.x, 0.5 + 0.5 * l.y), 0.0).r;
}

// Mean sky radiance over the upper hemisphere: 3 rings x 8 azimuths of the sky-view LUT, weighted by ring solid angle. A low sun leaves
// the zenith the darkest part of the sky, so the zenith alone underlights fog.
fn sky_mean(r: f32) -> vec3<f32> {
    var s = vec3(0.0);
    var wsum = 0.0;
    for (var i = 0u; i < 3u; i++) {
        let el = (f32(i) + 0.5) / 3.0 * 0.5 * PI;
        for (var k = 0u; k < 8u; k++) {
            let az = f32(k) / 8.0 * 2.0 * PI;
            s += cos(el) * sample_sky_view_lut(r, direction_world_to_atmosphere(vec3(cos(el) * sin(az), sin(el), cos(el) * cos(az))));
            wsum += cos(el);
        }
    }
    return s / wsum;
}

// Diffuse reflectance of what lies below a sample: a conservative fog layer of optical depth `tau` over ground of albedo `a`
// (two-stream: layer R = tau (1 - g) / (2 + tau (1 - g)), ground seen through it adds (1 - R)^2 a / (1 - R a)). Thick fog reflects
// sky light back up, which is why it reads white, not grey.
fn below(tau: f32, a: f32) -> f32 {
    let x = tau * (1.0 - f.g);
    let r = x / (2.0 + x);
    return r + (1.0 - r) * (1.0 - r) * a / max(1.0 - r * a, 1e-3);
}

// In-scattered light for optical depth `od` toward the light: octaves of weaker extinction, energy and anisotropy (multiple scattering).
fn ms(od: f32, c: f32) -> f32 {
    var s = 0.0;
    var a = 1.0;
    var b = 1.0;
    for (var k = 0u; k < max(f.ms_octaves, 1u); k++) {
        s += a * exp(-b * od) * hg(c, f.g * b);
        a *= f.ms_a;
        b *= f.ms_b;
    }
    return s;
}

// Share of the view ray from the eye to `t_end` that lies in the terrain's shadow (below each column's sunlit altitude), at occ_steps
// points jittered per pixel by `jit` (even spacing would band the share in steps of 1 / occ_steps). Outside the fog map counts as sunlit.
fn shadowed(eye: vec3<f32>, d: vec3<f32>, t_end: f32, jit: f32) -> f32 {
    let half = 0.5 * f.map_cell * f32(f.map_n);
    let centre = f.map_min + vec2(half, -half);
    var s = 0.0;
    for (var k = 0u; k < f.occ_steps; k++) {
        let p = eye + d * (t_end * (f32(k) + jit) / f32(f.occ_steps));
        if max(abs(p.x - centre.x), abs(p.z - centre.y)) > half { continue; }
        s += 1.0 - lit(p.y, map_at(p.xz).z);
    }
    return s / f32(max(f.occ_steps, 1u));
}

// Fog over the scene, plus the terrain-occlusion correction of the aerial perspective drawn on geometry (render_sky): the shadowed share
// of the ray to a surface loses that share of its aerial in-scatter (all but occ_keep). Returned premultiplied, so with the correction X
// the target becomes (dst - X)(1 - a) + rgb; the view target is float, so the negative source term subtracts.
@fragment
fn main(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let occ = f.occ_on > 0.0 && f.sun_dir.y > -0.05;
    if f.y_hi <= f.y_lo && !occ { return vec4(0.0); }
    let d = uv_to_ray_direction(in.uv);
    let eye = view.world_position;
    let z = textureLoad(depth, vec2<i32>(in.position.xy), 0);
    var t_scene = -1.0;
    if z > 0.0 {
        let v = view.view_from_clip * vec4(uv_to_ndc(in.uv), z, 1.0);
        t_scene = length(v.xyz / v.w);
    }
    var x_ap = vec3(0.0);
    if occ && t_scene > 0.0 {
        x_ap = shadowed(eye, d, t_scene, pcg(u32(in.position.x) + 4096u * u32(in.position.y) + f.frame * 16777619u + 7u)) * (1.0 - f.occ_keep) * sample_aerial_view_lut(in.uv, t_scene) * view.exposure;
    }
    let fog = march(in, d, eye, t_scene, occ);
    return vec4(fog.rgb - x_ap * (1.0 - fog.a), fog.a);
}

fn march(in: FullscreenVertexOutput, d: vec3<f32>, eye: vec3<f32>, t_scene: f32, occ: bool) -> vec4<f32> {
    if f.y_hi <= f.y_lo { return vec4(0.0); }
    var t1 = f.max_m;
    if t_scene > 0.0 { t1 = min(t1, t_scene); }
    var t0 = 0.0;
    if abs(d.y) > 1e-5 {
        let ta = (f.y_lo - eye.y) / d.y;
        let tb = (f.y_hi - eye.y) / d.y;
        t0 = max(t0, min(ta, tb));
        t1 = min(t1, max(ta, tb));
    } else if eye.y < f.y_lo || eye.y > f.y_hi {
        return vec4(0.0);
    }
    if t1 <= t0 { return vec4(0.0); }

    let R = atmosphere.inner_radius;
    let ce = get_view_position();
    let re = length(ce);
    let amb = sky_mean(re) * f.ambient_scale;
    let cs = dot(d, f.sun_dir);
    let cm = dot(d, f.moon_dir);
    let half = 0.5 * f.map_cell * f32(f.map_n);
    let centre = f.map_min + vec2(half, -half);
    let jit = pcg(u32(in.position.x) + 4096u * u32(in.position.y) + f.frame * 16777619u);
    let span = t1 - t0;

    var tr = 1.0;
    var lum = vec3(0.0);
    var wt = 0.0;
    var wsum = 0.0;
    var t_prev = t0;
    for (var i = 0u; i < f.steps; i++) {
        // Steps spaced as ((i + jitter) / steps)^step_power: dense near the eye, where the fog hides the most.
        let t = t0 + span * pow((f32(i) + jit) / f32(f.steps), max(f.step_power, 1.0));
        let dt = t - t_prev;
        t_prev = t;
        let p = eye + d * t;
        let texel = map_at(p.xz);
        let g = ground_h(p.xz);
        if p.y < g || (p.y > texel.x + f.top_soft + f.top_wave && texel.y <= 0.0) { continue; }
        let q = p - vec3(f.wind.x, 0.0, f.wind.y);
        let wave = 2.0 * vnoise(vec3(q.x, 0.0, q.z) / f.wave_tile) - 1.0;
        let nq = q / vec3(f.breakup_tile, f.breakup_y, f.breakup_tile);
        let n = 0.65 * vnoise(nq) + 0.35 * vnoise(nq * 2.7 + vec3(17.0, 3.0, 29.0));
        var dens = fog_density(texel, g, p.y, wave, n);
        if dens <= 0.0 { continue; }
        let rim = max(abs(p.x - centre.x), abs(p.z - centre.y)) / half;
        dens *= (1.0 - f.breakup * n) * (1.0 - smoothstep(1.0 - f.edge_fade, 1.0, rim));
        // Optical depth up to the pool top (the fog the light crosses to get here), plus the mist above.
        let h = max(p.y - g, 0.0);
        let od_up = f.sigma * max(texel.x - p.y, 0.0) + f.sigma_shallow * texel.y * f.shallow_e * exp(-h / max(f.shallow_e, 1e-3));
        // Isotropic in-scattering of diffuse light: half from the sky above (dimmed by the fog over the sample), half from below
        // (the fog and ground under the sample reflecting it).
        let tau_down = f.sigma * smoothstep(0.0, 0.5, texel.w) * max(min(texel.x, p.y) - g, 0.0);
        var light = amb * exp(-f.ambient_od * od_up) * (0.5 + 0.5 * below(tau_down, f.ground_albedo));
        if f.sun_dir.y > -0.05 {
            let sun = f.sun_color * atmo_transmittance(R + max(p.y, 0.0), f.sun_dir.y) * cloud_shadow(p);
            light += sun * lit(p.y, texel.z) * ms(od_up / max(f.sun_dir.y, 0.05), cs);
            // Diffuse sunlight from the sunlit top: two-stream diffuse transmission of a conservative layer, 1 / (1 + 0.75 (1 - g) tau),
            // minus the direct beam, as isotropic radiance (flux / pi). Neutral in colour: it is what makes deep fog white.
            let mu0 = max(f.sun_dir.y, 0.05);
            let td = max(1.0 / (1.0 + 0.75 * (1.0 - f.g) * od_up) - exp(-od_up / mu0), 0.0);
            light += sun * lit(texel.x, texel.z) * f.diffuse_sun * td * mu0 / PI;
        }
        if f.moon_dir.y > 0.0 {
            light += f.moon_color * atmo_transmittance(R + max(p.y, 0.0), f.moon_dir.y) * ms(od_up / max(f.moon_dir.y, 0.05), cm);
        }
        let s = f.albedo * light;
        let ts = exp(-dens * dt);
        lum += tr * (s - s * ts);
        wsum += tr;
        wt += tr * t;
        tr *= ts;
        if tr < 0.005 { break; }
    }
    if wsum <= 0.0 { return vec4(0.0); }
    let alpha = 1.0 - tr;
    let tc = wt / wsum;
    let mu = dot(d, ce / re);
    let t_air = sample_transmittance_lut_segment(re, mu, tc);
    let t_inf = sample_transmittance_lut(re, mu);
    let s_sky = sample_sky_view_lut(re, direction_world_to_atmosphere(d));
    var s_air = min(s_sky * (vec3(1.0) - t_air) / max(vec3(1.0) - t_inf, vec3(1e-4)), s_sky);
    // The air in front of the fog loses its sun glow where the terrain shadows it, as the aerial perspective does (main).
    if occ { s_air *= 1.0 - shadowed(eye, d, tc, jit) * (1.0 - f.occ_keep); }
    let rgb = (lum * t_air + alpha * s_air) * view.exposure;
    return vec4(min(rgb, vec3(F16_MAX)), alpha);
}
