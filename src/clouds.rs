//! Clouds: an HZD-style cloud layer over the weather map, raymarched in the render world (clouds/render.rs,
//! shaders/clouds_march.wesl) and composited over Bevy's sky-view LUT sky; stars and the discs are Bevy's Skybox and SunDisk (sky.rs).
//! Every knob is in the config (`clouds`, `resolve`); the plugin is built by `NephthenePlugin`.
use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    prelude::*,
    light::DirectionalLightTexture,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
};
use crate::sky::{Moon, NephtheneSun, Weather};
use nephthene_core as w;

pub mod render;
pub use render::{CloudParams, StormGpu};
use render::{CloudGpu, FlashGpu, ShadowGpu, ShadowPresentGpu, ShaftGpu};

#[derive(Resource)]
struct SharedShaders(#[allow(dead_code)] Handle<bevy::shader::Shader>);

pub struct CloudPlugin;
impl Plugin for CloudPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/clouds_common.wesl");
        embedded_asset!(app, "shaders/clouds_march.wesl");
        embedded_asset!(app, "shaders/clouds_composite.wesl");
        embedded_asset!(app, "shaders/clouds_resolve.wesl");
        embedded_asset!(app, "shaders/clouds_shadow.wesl");
        embedded_asset!(app, "shaders/clouds_shadow_present.wesl");
        embedded_asset!(app, "shaders/clouds_shaft.wesl");
        embedded_asset!(app, "shaders/clouds_shaft_resolve.wesl");
        embedded_asset!(app, "shaders/clouds_shaft_comp.wesl");
        // Load the shared module up front and keep it alive: the other passes import it (`super::clouds_common`).
        let common = app.world().resource::<AssetServer>().load::<bevy::shader::Shader>("embedded://nephthene/shaders/clouds_common.wesl");
        app.insert_resource(SharedShaders(common));
        let pass_timing = app.world().resource::<Weather>().cfg.clouds.pass_timing;
        render::plugin(app, pass_timing);
        app.add_systems(PostStartup, spawn_params.after(crate::sky::setup))
            .add_systems(Update, drive.in_set(crate::NephtheneSet::Drive).after(crate::NephtheneSet::Tick));
    }
}

fn check_div(v: u32) -> u32 {
    assert!([1, 2, 4].contains(&v), "clouds.update_div = {v}: want 1, 2 or 4");
    v
}

fn spawn_params(sun: Query<Entity, With<NephtheneSun>>, mut commands: Commands, wx: Res<Weather>, mut images: ResMut<Assets<Image>>, ground: Option<Res<crate::rain::Ground>>) {
    let grid = crate::rain::ground_grid(ground.as_deref());
    let map = &wx.map;
    let (cfg, c) = (&wx.cfg, &wx.cfg.clouds);
    let sr = &cfg.storms.render;
    assert!(!(cfg.map.wrap && c.background.is_some()), "map.wrap repeats the map, so clouds.background (the weather outside it) never applies: unset one");
    let gpu = CloudGpu {
        coverage_override: c.coverage_override.unwrap_or(-1.0), precip_override: -1.0,
        debug: c.debug_view,
        map_origin: Vec2::new(map.min_x, map.max_z), map_size: Vec2::new(map.size_x, map.size_z), world_offset: Vec2::from(cfg.map.world_offset_m),
        bottom: c.bottom_m, top: c.top_m, base_tile: c.base_tile_m, detail_tile: c.detail_tile_m, detail_strength: c.detail_strength,
        extinction: c.extinction, ambient_scale: c.ambient_scale, max_distance: c.max_distance_m, steps: c.steps, light_far: c.light_far_m, light_first: c.light_first_m,
        base_tile2: c.base_tile2_m, cov_var: c.coverage_var, curl: c.curl_m,
        light_gain: c.light_gain, shoulder: c.shoulder, shoulder_limit: c.shoulder_limit,
        tower_t: tower_stops(&c.tower_depth_m, 0), tower_d: tower_stops(&c.tower_depth_m, 1),
        background: c.background.map_or(Vec4::ZERO, |b| Vec4::new(b[0], b[1], b[2], 1.0)),
        cov_boost: c.look.cov_boost, tower_vsq: c.look.tower_vsq, tower_billow: c.look.tower_billow, background_blend: c.look.background_blend,
        vary_tile_mul: c.look.vary_tile_mul, horizon_step_boost: c.look.horizon_step_boost, horizon_step_y: c.look.horizon_step_y, cheap_empty_steps: c.look.cheap_empty_steps as f32,
        light_jitter_shared: cfg.resolve.light_jitter_shared as u32 as f32, jitter_step: cfg.resolve.jitter_step, light_jitter_offset: Vec2::from(cfg.resolve.light_jitter_offset),
        s_freq: sr.freq, s_vsq: sr.vsq, s_vsq_low: sr.vsq_low, s_amb_floor: sr.amb_floor, s_amb_var: sr.amb_var, s_erode: sr.erode, s_gain: sr.gain,
        s_od_per_m: sr.od_per_m, s_amb_k: sr.amb_k, s_detail_m: sr.detail_m, s_field_floor: sr.field_floor, s_range: sr.range_m, s_steps: sr.steps,
        s_ds_min: sr.ds_min_m, s_near: sr.near_m, s_sun_len: sr.sun_len_m, s_col_len: sr.col_len_m,
        s_core_flare: cfg.storms.core_flare, s_anvil_wide: cfg.storms.anvil_wide, s_anvil_p: cfg.storms.anvil_p, s_shelf_ceil: cfg.storms.shelf_ceil,
        s_curtain_full: cfg.storms.fx.curtain_full, s_couple: cfg.storms.couple as u32 as f32,
        air_guard: c.air_guard as u32,
        storm_step_opacity: c.storm_step_opacity as u32, depth_nearest: c.depth_nearest as u32,
        over_terrain: c.over_terrain as u32, g_min: Vec2::new(grid.min_x, grid.max_z), g_cell: grid.cell_m, g_w: grid.w, g_h: grid.h,
        ..default()
    };
    // Cloud shadows: an R8 image the cloud_shadow pass draws, projected by Bevy's directional light cookie.
    let mut shadow = Image::new_fill(Extent3d { width: c.shadow_res, height: c.shadow_res, depth_or_array_layers: 1 }, TextureDimension::D2, &[255], TextureFormat::R8Unorm, RenderAssetUsages::RENDER_WORLD);
    shadow.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
    // Explicit linear, clamped: Bevy's clustered-light sampler is the first light texture's.
    shadow.sampler = bevy::image::ImageSampler::linear();
    let shadow_back = images.add(shadow.clone());
    let shadow_done = images.add(shadow.clone()); let shadow_prev = images.add(shadow.clone());
    let shadow = images.add(shadow);
    for e in &sun { commands.entity(e).insert(DirectionalLightTexture { image: shadow.clone(), tiled: false }); }
    let (r, sh, n) = (&cfg.rain, &cfg.shafts, &c.noise);
    let shadow_gpu = ShadowGpu { half_m: c.shadow_half_m, strength: c.shadow_strength, samples: c.shadow_samples, ..default() };
    commands.insert_resource(CloudParams {
        shadow, shadow_back, shadow_done, shadow_prev, shadow_gpu, shadow_prev_gpu: shadow_gpu, shadow_draw_gpu: shadow_gpu, shadow_present: default(), shadow_draw: true, shadow_band: (0, 1),
        shaft: ShaftGpu { scale: c.shaft_scale, slant: Vec2::ZERO, g_cell: grid.cell_m, g_min: Vec2::new(grid.min_x, grid.max_z), g_w: grid.w, g_h: grid.h,
            precip_lo: r.precip_lo, precip_hi: r.precip_hi, sigma: sh.sigma, near_m: sh.near_m, fade_m: sh.fade_m, max_m: sh.max_m, xz_m: sh.xz_m, y_m: sh.y_m,
            ground_keep: sh.ground_keep, cloud_lo: sh.cloud_lo, cloud_hi: sh.cloud_hi, streak_lo: sh.streak_lo, streak_hi: sh.streak_hi, gap: sh.gap,
            albedo: sh.albedo, light_od: sh.light_od, steps: sh.steps },
        ground: images.add(crate::rain::ground_image(&grid)), storm: StormGpu::default(), flash: FlashGpu::default(),
        gpu, update_div: check_div(c.update_div), render_scale: c.render_scale, temporal_alpha: c.temporal_alpha,
        resolve: cfg.resolve.clone(), pass_timing: c.pass_timing, map_wrap: cfg.map.wrap,
        base: images.add(noise_3d(n.base_res, &n.base_freqs, Some((n.base_perlin_cells, n.base_perlin_octaves)), n.worley_mix)),
        detail: images.add(noise_3d(n.detail_res, &n.detail_freqs, None, n.worley_mix)),
        map: images.add(Image::new(Extent3d { width: map.width, height: map.height, depth_or_array_layers: 1 }, TextureDimension::D2,
            map.rgba.clone(), TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD)),
        oro: images.add(Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, TextureDimension::D2, vec![0, 0, 0, 255], TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD)),
    });
    println!("CLOUDS map {}x{} coverage {}", map.width, map.height, c.coverage_override.map_or("map".to_string(), |v| format!("override {v:.2}")));
}

/// The first 4 (type, depth) stops of clouds.tower_depth_m as one vec4 per column `k`, padded with the last; empty = zeros (towers off).
fn tower_stops(curve: &[[f32; 2]], k: usize) -> Vec4 {
    let Some(last) = curve.last() else { return Vec4::ZERO };
    Vec4::from_array(std::array::from_fn(|i| curve.get(i).unwrap_or(last)[k]))
}

/// Per frame: bodies from the lights sky.rs placed, wind drift (frozen in headless shots).
pub(crate) fn drive(
    time: Res<Time>, wx: Res<Weather>, params: Option<ResMut<CloudParams>>, mut drift: Local<Vec2>, mut slant: Local<Vec2>,
    frame: Res<bevy::diagnostic::FrameCount>, cam: Query<&GlobalTransform, With<bevy::pbr::AtmosphereSettings>>,
    mut sun: Query<(&DirectionalLight, &mut Transform), (With<NephtheneSun>, Without<Moon>)>, moon: Query<(&DirectionalLight, &Transform), With<Moon>>,
    oro: Option<Res<crate::orographic::Orographic>>, mut images: ResMut<Assets<Image>>, mut shown: Local<u32>,
) {
    let Some(mut m) = params else { return };
    if wx.rate > 0.0 { *drift += wx.wind.xz() * wx.cfg.clouds.wind_scale * time.delta_secs(); }
    let body = |l: &DirectionalLight, t: &Transform| (t.back().as_vec3(), l.color.to_linear().to_vec3() * l.illuminance);
    if let Some((l, mut t)) = sun.iter_mut().next() {
        (m.gpu.sun_dir, m.gpu.sun_color) = body(l, &t);
        // Cloud shadow cookie: a cycle redraws `shadow_every` bands into the back image, one per frame. Finished cookies are never shown
        // directly: the present pass mixes the last two in the newest one's frame, so the light's origin and the shown image always agree
        // and a cycle's change spreads over its frames instead of stepping once.
        let every = wx.cfg.clouds.shadow_every.max(1);
        m.shadow_draw = frame.0 % every == 0;
        m.shadow_band = (frame.0 % every, every);
        if m.shadow_draw && let Some(p) = cam.iter().next().map(|g| g.translation()) {
            let m = &mut *m;
            // The finished back image becomes the newest; the one before it is drawn over next.
            std::mem::swap(&mut m.shadow_prev, &mut m.shadow_done);
            std::mem::swap(&mut m.shadow_done, &mut m.shadow_back);
            m.shadow_prev_gpu = m.shadow_gpu;
            m.shadow_gpu = m.shadow_draw_gpu;
            let (r, u) = (t.right().as_vec3(), t.up().as_vec3());
            m.shadow_draw_gpu.origin = w::shadow_origin(p, r, u, wx.cfg.clouds.shadow_half_m, wx.cfg.clouds.shadow_res);
            (m.shadow_draw_gpu.right, m.shadow_draw_gpu.up) = (r, u);
        }
        m.shadow_draw_gpu.samples = w::shadow_samples(&wx.cfg.clouds, wx.cfg.clouds.top_m - wx.cfg.clouds.bottom_m, m.gpu.sun_dir.y);
        m.shadow_gpu.samples = m.shadow_draw_gpu.samples;
        let c = &wx.cfg.clouds;
        m.shadow_present = ShadowPresentGpu {
            shift: w::shadow_prev_shift(m.shadow_gpu.origin, m.shadow_prev_gpu.origin, m.shadow_gpu.right, m.shadow_gpu.up, c.shadow_half_m).into(),
            t: w::shadow_mix(frame.0, every, c.shadow_blend),
            soft: c.shadow_soft_px / c.shadow_res as f32,
        };
        (t.translation, t.scale) = (m.shadow_gpu.origin, Vec3::splat(wx.cfg.clouds.shadow_half_m));
    }
    if let Some((l, t)) = moon.iter().next() { (m.gpu.moon_dir, m.gpu.moon_color) = body(l, t); }
    // Upwind is where the noise comes from: sampling at p - drift moves the shapes downwind.
    m.gpu.wind = -*drift;
    // The clouds read the advected map (map.static_gpu, the test red arm, keeps the unshifted one).
    let shift = if wx.cfg.map.static_gpu { Vec2::ZERO } else { wx.map_shift };
    m.gpu.world_offset = Vec2::from(wx.cfg.map.world_offset_m) - shift;
    // Mean wind, slewed: gusts never lean the shafts (shafts.gust_lean is the test red arm).
    *slant = w::rain::shaft_slant_step(&wx.cfg.rain, &wx.cfg.shafts, &wx.cfg.wind, wx.wind, *slant, time.delta_secs());
    m.shaft.slant = *slant;
    // The orographic tile (world frame): upload a newly landed one; without a tile the shader skips it (size 0).
    match oro.as_ref().and_then(|o| o.tile.clone().map(|t| (t, o.bakes))) {
        Some((t, bakes)) => {
            if *shown != bakes {
                let _ = images.insert(&m.oro, Image::new(Extent3d { width: t.n, height: t.n, depth_or_array_layers: 1 }, TextureDimension::D2, t.rgba8(), TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD));
                *shown = bakes;
            }
            (m.gpu.oro_origin, m.gpu.oro_size, m.gpu.oro_weight) = (Vec2::new(t.min_x, t.max_z), t.n as f32 * t.cell_m, wx.cfg.orographic.cover_weight);
            m.gpu.oro_type = wx.cfg.orographic.type_weight;
        }
        None => m.gpu.oro_size = 0.0,
    }
    // Read every frame so the panel toggle and live config edits apply at once.
    let e = &wx.cfg.clouds.edge_lod;
    (m.gpu.edge_on, m.gpu.edge_r0, m.gpu.edge_r1, m.gpu.edge_step_scale, m.gpu.edge_detail_r0, m.gpu.edge_detail_r1) =
        (e.enabled as u32 as f32, e.r0, e.r1, e.step_scale, e.detail_r0, e.detail_r1);
    let f = &wx.cfg.clouds.far_lod;
    (m.gpu.far_on, m.gpu.far_m, m.gpu.far_samples, m.gpu.far_detail) = (f.enabled as u32 as f32, f.start_m, f.samples.max(1) as f32, f.detail as u32 as f32);
}

/// Tileable cloud noise, `n`^3 RGBA8 (Schneider 2015). With `perlin` = Some(f): R is the base shape, Perlin fbm at
/// f cells per tile billowed by the Worley octaves, then rank-equalised to uniform 0..1 so that coverage c covers a
/// fraction c of the sky; GBA are the inverted Worley octaves at `freqs`. Without: RGB are the Worley octaves, A = 1.
fn noise_3d(n: u32, freqs: &[u32; 3], perlin: Option<(u32, u32)>, worley_mix: f32) -> Image {
    let mut px: Vec<[f32; 4]> = vec![[0.0; 4]; (n * n * n) as usize];
    // One z slice per task, split over the cores (128^3 base is 2M voxels).
    let per = (n * n) as usize;
    let threads = std::thread::available_parallelism().map_or(4, |t| t.get());
    let chunk = (n as usize).div_ceil(threads);
    std::thread::scope(|sc| for (ci, part) in px.chunks_mut(chunk * per).enumerate() {
        sc.spawn(move || for (k, v) in part.iter_mut().enumerate() {
            let i = ci * chunk * per + k;
            let (x, y, z) = ((i % n as usize) as f32, ((i / n as usize) % n as usize) as f32, (i / per) as f32);
            let p = Vec3::new(x, y, z) / n as f32;
            let wy = freqs.map(|f| 1.0 - worley(p, f));
            *v = match perlin {
                Some((f, oct)) => {
                    let norm: f32 = (0..oct).map(|o| 0.5f32.powi(o as i32)).sum();
                    let pf = (0..oct).map(|o| perlin_noise(p, f << o) * 0.5f32.powi(o as i32)).sum::<f32>() / norm * 0.5 + 0.5;
                    // ponytail: fbm weights 0.625/0.25/0.125 are fixed (shader mirrors them for the detail tile).
                    let wf = wy[0] * 0.625 + wy[1] * 0.25 + wy[2] * 0.125;
                    [pf + worley_mix * wf, wy[0], wy[1], wy[2]]
                }
                None => [wy[0], wy[1], wy[2], 1.0],
            };
        });
    });
    if perlin.is_some() {
        let mut order: Vec<usize> = (0..px.len()).collect();
        order.sort_by(|&a, &b| px[a][0].total_cmp(&px[b][0]));
        let last = (px.len() - 1) as f32;
        for (rank, i) in order.into_iter().enumerate() { px[i][0] = rank as f32 / last; }
    }
    let data = px.iter().flat_map(|v| v.map(|c| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)).collect();
    Image::new(Extent3d { width: n, height: n, depth_or_array_layers: n }, TextureDimension::D3, data, TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD)
}

fn hash3(c: IVec3, salt: u32) -> Vec3 {
    let mut h = (c.x as u32).wrapping_mul(73856093) ^ (c.y as u32).wrapping_mul(19349663) ^ (c.z as u32).wrapping_mul(83492791) ^ salt.wrapping_mul(2654435761);
    let mut next = || { h ^= h << 13; h ^= h >> 17; h ^= h << 5; h as f32 / u32::MAX as f32 };
    Vec3::new(next(), next(), next())
}

/// Distance to the nearest feature point, in cells, on a lattice of `f` cells that wraps at the tile edge.
fn worley(p: Vec3, f: u32) -> f32 {
    let q = p * f as f32;
    let c = q.floor().as_ivec3();
    let mut best = f32::MAX;
    for dz in -1..=1 { for dy in -1..=1 { for dx in -1..=1 {
        let cell = c + IVec3::new(dx, dy, dz);
        let wrapped = cell.rem_euclid(IVec3::splat(f as i32));
        best = best.min((cell.as_vec3() + hash3(wrapped, f) - q).length());
    }}}
    best.min(1.0)
}

/// Gradient noise in -1..1 with period `f` cells.
fn perlin_noise(p: Vec3, f: u32) -> f32 {
    let q = p * f as f32;
    let c = q.floor().as_ivec3();
    let t = q - c.as_vec3();
    let s = t * t * (Vec3::splat(3.0) - 2.0 * t);
    let grad = |o: IVec3| {
        let g = hash3((c + o).rem_euclid(IVec3::splat(f as i32)), 977 + f) * 2.0 - 1.0;
        g.normalize_or_zero().dot(t - o.as_vec3())
    };
    let l = |a: f32, b: f32, k: f32| a + (b - a) * k;
    let x00 = l(grad(IVec3::new(0, 0, 0)), grad(IVec3::new(1, 0, 0)), s.x);
    let x10 = l(grad(IVec3::new(0, 1, 0)), grad(IVec3::new(1, 1, 0)), s.x);
    let x01 = l(grad(IVec3::new(0, 0, 1)), grad(IVec3::new(1, 0, 1)), s.x);
    let x11 = l(grad(IVec3::new(0, 1, 1)), grad(IVec3::new(1, 1, 1)), s.x);
    l(l(x00, x10, s.y), l(x01, x11, s.y), s.z) * 1.4
}

#[cfg(test)]
mod noise_tests {
    use super::*;
    /// Mean |difference| of channel `ch` between neighbours along each axis (x, y, z) and the mean of channel; an axis far below the others is a layering artefact.
    fn axis_diff(img: &Image, n: usize, ch: usize) -> [f32; 3] {
        let d = img.data.as_ref().unwrap();
        let v = |x: usize, y: usize, z: usize| d[((z * n + y) * n + x) * 4 + ch] as f32;
        let mut s = [0.0f32; 3];
        for z in 0..n { for y in 0..n { for x in 0..n {
            s[0] += (v(x, y, z) - v((x + 1) % n, y, z)).abs();
            s[1] += (v(x, y, z) - v(x, (y + 1) % n, z)).abs();
            s[2] += (v(x, y, z) - v(x, y, (z + 1) % n)).abs();
        }}}
        s.map(|a| a / (n * n * n) as f32)
    }
    #[test]
    fn noise_isotropic() {
        let b = noise_3d(128, &[4, 8, 16], Some((4, 4)), 0.6);
        let dt = noise_3d(32, &[2, 4, 8], None, 0.6);
        for ch in 0..4 { println!("base ch{ch} dx,dy,dz = {:?}", axis_diff(&b, 128, ch)); }
        for ch in 0..3 { println!("detail ch{ch} dx,dy,dz = {:?}", axis_diff(&dt, 32, ch)); }
    }
}
