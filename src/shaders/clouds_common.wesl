// Clouds: code shared by clouds_march / clouds_shadow / clouds_shaft (one copy). Bindings stay in each pass (the shadow pass
// uses group 0, the others group 1), so textures and the uniform come in as arguments. Every constant here is untuned.
import bevy_pbr::atmosphere::{
    bindings::atmosphere,
    functions::sample_transmittance_lut,
};

struct Clouds {
    sun_dir: vec3<f32>,
    coverage_override: f32,
    sun_color: vec3<f32>,
    ambient_scale: f32,
    moon_dir: vec3<f32>,
    extinction: f32,
    moon_color: vec3<f32>,
    max_distance: f32,
    map_origin: vec2<f32>,
    map_size: vec2<f32>,
    world_offset: vec2<f32>,
    wind: vec2<f32>,
    bottom: f32,
    top: f32,
    base_tile: f32,
    detail_tile: f32,
    detail_strength: f32,
    steps: u32,
    // Metres of the far light-march segment.
    light_far: f32,
    // Metres of the first light-march segment.
    light_first: f32,
    light_gain: f32,
    shoulder: f32,
    shoulder_limit: f32,
    frame: u32,
    // debug_view probe mode (0 = off, the shipped look). 1-5 write grey probes, see the fragment.
    debug: u32,
    // Second base-noise scale (m), relative coverage variation 0..1, curl-like detail warp (m). Untuned.
    base_tile2: f32,
    cov_var: f32,
    curl: f32,
    // Panel rain toggle: precip used in place of the map's G channel when >= 0 (-1 = the map).
    precip_override: f32,
    // HZD update (clouds_march.wesl): this frame's pixel offset inside each 4x4 block, and the full render size in pixels.
    march_ox: f32,
    march_oy: f32,
    full_w: f32,
    full_h: f32,
    // 4 = the 4x4 update, 1 = march every pixel every frame (update_div 1, the full-quality reference arm).
    march_div: f32,
    // Tier-2 towers : up to 4 (cloud type, column depth m) stops, padded with the last; tower_d.x <= 0 = off (the flat layer).
    tower_t: vec4<f32>,
    tower_d: vec4<f32>,
    // (coverage, precip, type) used outside the weather map's uv, .w = 1 when set (a far field); 0 = the map's edge clamp.
    background: vec4<f32>,
    // Look constants (config clouds.look). Detail erosion eats the thin edge, so the base threshold widens by cov_boost.
    cov_boost: f32,
    // Tower noise: altitude squash for the base shape, and the share of unsquashed noise in the surface.
    tower_vsq: f32,
    tower_billow: f32,
    // Where the map's edge blends into `background`, fraction of the map size.
    background_blend: f32,
    // The coverage-variation noise tile is base_tile * this.
    vary_tile_mul: f32,
    horizon_step_boost: f32,
    horizon_step_y: f32,
    cheap_empty_steps: f32,
    // Resolve-related march inputs (config resolve): 1 = the light march shares the eye march's jitter (test red arm), golden-step per visit, light jitter pixel offset.
    light_jitter_shared: f32,
    jitter_step: f32,
    light_jitter_offset: vec2<f32>,
    // Storm render constants (config storms.render and the profile shape).
    s_freq: f32, s_vsq: f32, s_vsq_low: f32, s_amb_floor: f32,
    s_amb_var: f32, s_erode: f32, s_gain: f32, s_od_per_m: f32,
    s_amb_k: f32, s_detail_m: f32, s_field_floor: f32, s_range: f32,
    s_steps: f32, s_ds_min: f32, s_near: f32, s_sun_len: f32,
    s_col_len: f32, s_core_flare: f32, s_anvil_wide: f32, s_anvil_p: f32,
    s_shelf_ceil: f32,
    s_curtain_full: f32,
    s_couple: f32,
}

// Storm cells . One StormCell is nephthene_core::storm::pack's 4 vec4 per cell, field for field (a text-pin test, tests/storm_layout.rs, fails when the order here drifts). xz are world metres (the host subtracts the world offset).
struct StormCell {
    a: vec2<f32>, b: vec2<f32>,
    core_r: f32, base: f32, top: f32, overshoot: f32,
    anvil_r: f32, shift: vec2<f32>, anvil_base: f32,
    precip: f32, gust_r: f32, shelf_low: f32, dir_angle: f32,
}
// `n` live cells in `cells[0..4n]`. Declared at binding 6 on the march, shadow and shaft passes. Drawn by the march (W3a), culled per ray there.
struct Storms {
    n: u32, pad0: u32, pad1: u32, pad2: u32,
    // 4 vec4 per cell (the uniform needs 16-byte strides, StormCell is 64 bytes with a vec2 at offset 36); storm_unpack() builds one.
    cells: array<vec4<f32>, 64>,
}

const PI: f32 = 3.14159265;
const F16_MAX: f32 = 60000.0;

fn remap(v: f32, lo: f32, hi: f32, nlo: f32, nhi: f32) -> f32 {
    return nlo + (v - lo) * (nhi - nlo) / (hi - lo);
}

fn hg(c: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (4.0 * PI * pow(max(1.0 + g2 - 2.0 * g * c, 1e-4), 1.5));
}

// Forward lobe for the silver lining, a weaker back lobe; `k` shrinks both per scattering octave.
fn phase(c: f32, k: f32) -> f32 {
    return mix(hg(c, 0.8 * k), hg(c, -0.3 * k), 0.3);
}

// Cloud type sets the density-height profile (untuned): 0 stratus is thin with a flat base and a hard top, 0.5 cumulus has a
// flat base and a long soft round top, 1 cumulonimbus fills the layer with a firmer base.
fn gradient(h: f32, kind: f32) -> f32 {
    let top = mix(0.22, 1.0, kind);
    let soft = sin(kind * PI);
    let rise = mix(0.03, 0.07, soft);
    return smoothstep(0.0, rise, h) * (1.0 - smoothstep(top * mix(0.75, 0.25, soft), top, h));
}

// Per-pixel white hash (PCG): a base value per pixel, stepped by the golden ratio per VISIT of that pixel (HZD 4x4: one visit per 16 frames,
// so a per-frame step would land on the coarse 16-frame lattice, 0.8885 per visit, and leave row-constant stripes). The per-visit
// golden step (config resolve.jitter_step) makes the jitter of the running mean stratify. No per-frame random term: it would only add noise to the mean.
fn pcg(v: u32) -> f32 {
    var h = v * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return f32((h >> 22u) ^ h) / 4294967295.0;
}
fn ign(p: vec2<f32>, visit: u32, jstep: f32) -> f32 {
    let k = u32(p.x) + 4096u * u32(p.y);
    return fract(pcg(k) + jstep * f32(visit & 255u));
}

fn atmo_transmittance(r: f32, mu: f32) -> vec3<f32> {
    let R = atmosphere.inner_radius;
    // Below the planet's horizon from here: the body is shadowed by the ground.
    if mu < -sqrt(max(1.0 - (R / r) * (R / r), 0.0)) { return vec3(0.0); }
    return sample_transmittance_lut(r, mu);
}

// Base shape 0..1 (uniform): the 128^3 Perlin-Worley tile sampled at two incommensurate scales, the second rotated 37 deg about
// the vertical and offset, so no period shows. The mean of two uniforms is triangular; its CDF restores a uniform value so
// coverage c still covers a fraction c of the sky (untuned rotation and offset).
fn base_shape_at(u: Clouds, base_tex: texture_3d<f32>, ns: sampler, p: vec3<f32>) -> f32 {
    let a = textureSampleLevel(base_tex, ns, p / u.base_tile, 0.0).r;
    let q = vec3(0.7986 * p.x - 0.6018 * p.z, p.y, 0.6018 * p.x + 0.7986 * p.z) + vec3(3170.0, 910.0, 5230.0);
    let b = textureSampleLevel(base_tex, ns, q / u.base_tile2, 0.0).r;
    let m = 0.5 * (a + b);
    return select(2.0 * m * m, 1.0 - 2.0 * (1.0 - m) * (1.0 - m), m > 0.5);
}

fn map_sample(u: Clouds, map_tex: texture_2d<f32>, ms: sampler, xz: vec2<f32>) -> vec4<f32> {
    let c = u.world_offset + xz;
    let uv = vec2((c.x - u.map_origin.x) / u.map_size.x, (u.map_origin.y - c.y) / u.map_size.y);
    let m = textureSampleLevel(map_tex, ms, uv, 0.0);
    if u.background.w <= 0.0 { return m; }
    let out = max(max(-uv.x, uv.x - 1.0), max(-uv.y, uv.y - 1.0));
    return vec4(mix(m.xyz, u.background.xyz, saturate(out / u.background_blend)), m.w);
}

// Tower depth (m) for cloud type `kind`: the piecewise-linear curve of the 4 stops (clamped outside).
fn tower_depth(u: Clouds, kind: f32) -> f32 {
    var d = u.tower_d.x;
    d += (u.tower_d.y - u.tower_d.x) * saturate((kind - u.tower_t.x) / max(u.tower_t.y - u.tower_t.x, 1e-4));
    d += (u.tower_d.z - u.tower_d.y) * saturate((kind - u.tower_t.y) / max(u.tower_t.z - u.tower_t.y, 1e-4));
    d += (u.tower_d.w - u.tower_d.z) * saturate((kind - u.tower_t.z) / max(u.tower_t.w - u.tower_t.z, 1e-4));
    return d;
}

// Density 0..~2 at world xz, altitude `alt` above the ground and height fraction `h` in the layer. `detail` runs the detail erosion;
// `mean_erode` (shadow pass, no detail lookup) erodes by the detail's mean (wisp ~0.5) so the shadow footprint matches what the march draws.
fn density_at(u: Clouds, base_tex: texture_3d<f32>, detail_tex: texture_3d<f32>, ns: sampler, map_tex: texture_2d<f32>, ms: sampler,
              xz: vec2<f32>, alt: f32, h_shell: f32, detail: bool, mean_erode: bool) -> f32 {
    let m = map_sample(u, map_tex, ms, xz);
    var cov = select(m.r, u.coverage_override, u.coverage_override >= 0.0);
    let p = vec3(xz.x + u.wind.x, alt, xz.y + u.wind.y);
    // Towers: the base shape sees a vertically squashed altitude (one coherent column per blob).
    let tower = u.tower_d.x > 0.0;
    let pb = select(p, vec3(p.x, u.bottom + (alt - u.bottom) * u.tower_vsq, p.z), tower);
    // Coverage varies from noise at a third, larger scale (the map alone is smooth), relative so zero stays zero.
    let vary = textureSampleLevel(base_tex, ns, vec3(p.x, 0.0, p.z) / (u.base_tile * u.vary_tile_mul) + vec3(0.31, 0.5, 0.77), 0.0).r;
    cov = saturate(cov * (1.0 + u.cov_var * (2.0 * vary - 1.0)));
    let bs_col = base_shape_at(u, base_tex, ns, pb);
    var bs = bs_col;
    if tower { bs = mix(bs_col, base_shape_at(u, base_tex, ns, p), u.tower_billow); }
    let lo = 1.0 - min(cov * u.cov_boost, 1.0);
    // Tier-2 towers (plan 5.1): the column's depth is the type's curve times sqrt(blob strength); the height fraction is relative to it.
    var h = h_shell;
    if tower { h = (alt - u.bottom) / max(tower_depth(u, m.b) * sqrt(saturate((bs_col - lo) / max(1.0 - lo, 1e-4))), 50.0); }
    var c = saturate(remap(bs * gradient(h, m.b), lo, 1.0, 0.0, 1.0));
    h = saturate(h);
    if detail && c > 0.0 {
        // Curl-like warp (untuned): a coarse Worley lookup displaces the detail lookup, so erosion swirls rather than tiles.
        let dp = p / u.detail_tile;
        let cu = textureSampleLevel(detail_tex, ns, dp / 4.7, 0.0).gb - 0.5;
        let q = dp + vec3(cu.x, 0.35 * (cu.x - cu.y), cu.y) * (2.0 * u.curl / u.detail_tile);
        let d = textureSampleLevel(detail_tex, ns, q, 0.0);
        let dfbm = d.r * 0.625 + d.g * 0.25 + d.b * 0.125;
        // Wispy at the base, billowy at the top, with erosion growing with height (untuned).
        let wisp = mix(dfbm, 1.0 - dfbm, smoothstep(0.1, 0.6, h));
        c = saturate(remap(c, wisp * u.detail_strength * mix(0.8, 1.6, h), 1.0, 0.0, 1.0));
    }
    if mean_erode && !detail {
        c = saturate(remap(c, 0.5 * u.detail_strength * mix(0.8, 1.6, h), 1.0, 0.0, 1.0));
    }
    // Rain-bearing cloud is denser and darker underneath.
    return c * (1.0 + select(m.g, u.precip_override, u.precip_override >= 0.0));
}

// ---- Storm cells. Their constants are uniform fields s_* (config storms.render). ----
// Detail erosion only inside s_detail_m ray distance: the detail tile is point-sampled, so further out it only aliases.

fn storm_unpack(v0: vec4<f32>, v1: vec4<f32>, v2: vec4<f32>, v3: vec4<f32>) -> StormCell {
    return StormCell(v0.xy, v0.zw, v1.x, v1.y, v1.z, v1.w, v2.x, v2.yz, v2.w, v3.x, v3.y, v3.z, v3.w);
}

fn storm_nearest(a: vec2<f32>, b: vec2<f32>, p: vec2<f32>) -> vec2<f32> {
    let ab = b - a;
    let l2 = dot(ab, ab);
    if l2 < 1e-6 { return a; }
    return a + ab * clamp(dot(p - a, ab) / l2, 0.0, 1.0);
}

// nephthene_core::storm_fx::precip_at: the rain curtain under a cell, full inside s_curtain_full x core_r, from the base down.
fn storm_precip(cl: Clouds, c: StormCell, g: vec2<f32>, y: f32) -> f32 {
    if (c.precip <= 0.0 || c.core_r < 1.0 || y < 0.0 || y > c.base) { return 0.0; }
    let d = distance(g, storm_nearest(c.a, c.b, g));
    return c.precip * (1.0 - smoothstep(cl.s_curtain_full * c.core_r, c.core_r, d));
}

// Metres from the cell's axis midpoint beyond which the profile is zero (bounding circle for culling).
fn storm_reach(c: StormCell) -> f32 {
    return 0.5 * length(c.b - c.a) + max(c.core_r * 1.5, max(c.anvil_r, c.gust_r));
}

// Mirror of nephthene_core::storm::profile: analytic envelope 0..1 at world (xz, altitude y). Shape values are the s_* uniforms.
fn storm_profile(cl: Clouds, c: StormCell, g: vec2<f32>, y: f32) -> f32 {
    let q = storm_nearest(c.a, c.b, g);
    let d = distance(g, q);
    var v = 0.0;
    if c.core_r > 1.0 && c.top > c.base {
        let hf = clamp((y - c.base) / max(c.top - c.base, 1.0), 0.0, 1.0);
        let rc = c.core_r * (1.0 + cl.s_core_flare * smoothstep(0.3, 1.0, hf));
        let dr = d / (0.8 * c.core_r * (1.0 + cl.s_core_flare));
        let dome = c.overshoot * max(1.0 - dr * dr, 0.0);
        let rise = max(min(c.top - c.base, 1000.0), 1.0);
        var vert = smoothstep(c.base, c.base + 0.5 * rise, y) * (1.0 - smoothstep(c.top + dome - 0.5 * rise, c.top + dome, y));
        if y > c.top + dome { vert = 0.0; }
        v = max(v, (1.0 - smoothstep(0.6, 1.0, d / rc)) * vert);
    }
    if c.anvil_r > 1.0 && c.core_r > 1.0 && y >= c.anvil_base && y <= c.top && dot(c.shift, c.shift) > 1.0 {
        let sd = normalize(c.shift);
        let r = g - q;
        let s = dot(r, sd);
        let t = dot(r, vec2(-sd.y, sd.x));
        let hw0 = c.core_r * (1.0 + cl.s_core_flare);
        let u = s / c.anvil_r;
        var w = hw0 * sqrt(max(1.0 - (s / hw0) * (s / hw0), 0.0));
        if s >= 0.0 { w = (hw0 + max(cl.s_anvil_wide * c.anvil_r - hw0, 0.0) * smoothstep(0.0, 0.5, u)) * sqrt(1.0 - smoothstep(0.55, 1.0, u)); }
        if w > 1.0 {
            let thick = max(c.top - c.anvil_base, 1.0);
            let uc = clamp(u, 0.0, 1.0);
            let yb = c.anvil_base + 0.6 * thick * smoothstep(0.2, 1.0, uc);
            let a = cl.s_anvil_p * (1.0 - smoothstep(0.6, 1.0, abs(t) / w)) * (1.0 - 0.7 * smoothstep(0.4, 1.0, uc)) * smoothstep(yb, yb + 0.3 * thick, y);
            v = max(v, a);
        }
    }
    let dir = vec2(cos(c.dir_angle), sin(c.dir_angle));
    if c.shelf_low < c.base && c.gust_r > c.core_r && c.core_r > 1.0 && dot(g - q, dir) > 0.0 && y >= c.shelf_low && y <= c.base + (c.base - c.shelf_low) * 2.0 {
        let dr = c.base - c.shelf_low;
        let ef = clamp((d - c.core_r) / max(c.gust_r - c.core_r, 1.0), 0.0, 1.0);
        let radial = smoothstep(c.core_r * 0.8, c.core_r * 1.2, d) * (1.0 - smoothstep(0.7, 1.0, d / c.gust_r));
        let fl = c.base - dr * smoothstep(0.0, 0.7, ef);
        let ce = c.base + dr * (cl.s_shelf_ceil * (1.0 - ef) + 0.5);
        let vert = smoothstep(fl, fl + 0.35 * dr, y) * (1.0 - smoothstep(ce - 0.5 * dr, ce, y));
        let ahead = smoothstep(0.4, 0.95, dot(g - q, dir) / max(d, 1.0));
        v = max(v, 0.8 * radial * vert * ahead);
    }
    return clamp(v, 0.0, 1.0);
}

// Column optical depth straight up from (xz, y0) through cell `c`: two profile taps (the core band and the anvil band) instead of a
// marched chord. storm_fx::column_od marches the same quantity; this is its cheap per-march-sample form.
fn storm_col_cell(u: Clouds, c: StormCell, xz: vec2<f32>, y0: f32) -> f32 {
    let y1 = max(y0, c.base);
    let ya = max(y1, c.anvil_base);
    var od = 0.0;
    if c.anvil_base > y1 { od += storm_profile(u, c, xz, 0.5 * (y1 + c.anvil_base)) * (c.anvil_base - y1); }
    if c.top + c.overshoot > ya { od += storm_profile(u, c, xz, 0.5 * (ya + c.top)) * (c.top - ya); }
    return od * u.s_od_per_m;
}

