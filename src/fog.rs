//! Ground fog, main world: keeps the terrain fields and per-texel fog amounts around the camera (nephthene_core::fog), steps them with
//! the clock and the weather at the camera (each texel burns only while the sun reaches it over the terrain), bakes the fog map (first
//! bake inline, later ones on a background task) and fills the uniform the render world marches (fog/render.rs, shaders/fog.wesl).
//! Always built by `NephthenePlugin`; idle unless config `ground_fog.enabled` (the panel can switch it on) or `sky.sun_occlusion` with a
//! terrain `Ground` (the pass then only removes the aerial sun glow from shadowed air). Read [`FogState`].
use std::sync::Arc;
use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    diagnostic::FrameCount,
    pbr::AtmosphereSettings,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};
use nephthene_core::{self as w, config::GroundFog, fog::{self as f, Air, Field, FogMap}, rain::{HeightGrid, smoothstep}};
use crate::{clouds::CloudParams, rain::RainState, sky::{Moon, NephtheneSun, Weather}};

pub mod render;
pub use render::{FogGpu, FogParams};

/// Live ground fog at the camera: `amount` 0..1 (formation and local burn-off, or `ground_fog.force`), `lift` the burn-off lift of its
/// base (m).
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct FogState { pub amount: f32, pub lift: f32 }

pub struct FogPlugin;
impl Plugin for FogPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/fog.wesl");
        render::plugin(app);
        app.init_resource::<FogState>()
            .add_systems(PostStartup, spawn.after(crate::sky::setup))
            .add_systems(Update, (depth_readable, (bake, drive).chain().in_set(crate::NephtheneSet::Drive).after(crate::NephtheneSet::Tick)));
    }
}

/// One bake's result: the fields it used, the amounts after it, the map.
type Baked = (Arc<Field>, Vec<f32>, FogMap);

/// What a frozen clock's spin-up was computed for (hour, weather, field origin); a change re-spins.
type Key = [f32; 7];

/// Fog state around the camera and the bake in flight.
#[derive(Resource)]
struct Baker { grid: Arc<HeightGrid>, terrain: bool, field: Option<Arc<Field>>, m: Vec<f32>, alt: Vec<f32>, task: Option<Task<Baked>>, pending_h: f32, since: f32, key: Option<Key>, cfg: Option<GroundFog> }

fn image(w: u32, h: u32, format: TextureFormat, data: Vec<u8>) -> Image {
    let mut im = Image::new(Extent3d { width: w, height: h, depth_or_array_layers: 1 }, TextureDimension::D2, data, format, RenderAssetUsages::RENDER_WORLD);
    im.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST;
    im
}

fn spawn(mut commands: Commands, mut images: ResMut<Assets<Image>>, ground: Option<Res<crate::rain::Ground>>, wx: Res<Weather>) {
    let grid = crate::rain::ground_grid(ground.as_deref());
    commands.insert_resource(FogParams {
        gpu: FogGpu { g_min: Vec2::new(grid.min_x, grid.max_z), g_cell: grid.cell_m, g_w: grid.w, g_h: grid.h, map_n: 1, ..default() },
        map: images.add(image(1, 1, TextureFormat::Rgba32Float, vec![0; 16])),
        ground: images.add(crate::rain::ground_image(&grid)),
        // No clouds: a white cookie (sh_on stays 0 and the shader never reads it).
        cookie: images.add(image(1, 1, TextureFormat::R8Unorm, vec![255])),
        // ponytail: 64^3 cells of noise lattice: tiles every 64 noise cells (16 km at the 250 m breakup cell), too far to see.
        noise: images.add({ let mut im = image(64, 64, TextureFormat::R8Unorm, f::noise_lattice(64)); im.texture_descriptor.size.depth_or_array_layers = 64; im.texture_descriptor.dimension = TextureDimension::D3; im }),
        render_scale: wx.cfg.ground_fog.render_scale,
        timing: wx.cfg.clouds.pass_timing,
    });
    commands.insert_resource(Baker { grid: Arc::new(grid), terrain: ground.is_some(), field: None, m: vec![], alt: vec![], task: None, pending_h: 0.0, since: 0.0, key: None, cfg: None });
}

/// The pass reads the scene depth: the atmosphere camera's depth texture must be bindable.
fn depth_readable(mut cams: Query<&mut Camera3d, With<AtmosphereSettings>>) {
    let bit = TextureUsages::TEXTURE_BINDING.bits();
    for mut c in &mut cams { if c.depth_texture_usages.0 & bit == 0 { c.depth_texture_usages.0 |= bit; } }
}

/// A bake: new fields if the camera left the old ones (or the config changed), then the amounts (spun up for a frozen clock, the first
/// bake or a config change, else stepped by the game hours since the last bake; none with the fog off), then the map.
fn job(c: &GroundFog, grid: &HeightGrid, clock: &w::Clock, hour: f32, air: Air, at: Vec2, old: Option<(Arc<Field>, Vec<f32>)>, spin: bool, dt_h: f32) -> Baked {
    let (el, az) = w::sun(clock, hour);
    let sun = w::dir(el, az);
    let fresh = spin || old.as_ref().is_none_or(|(o, _)| at.distance(Vec2::new(o.min_x, o.max_z) + Vec2::new(0.5, -0.5) * c.extent_m) > c.rebake_m);
    let field = if fresh { Arc::new(f::field(c, grid, [at.x, at.y])) } else { old.as_ref().unwrap().0.clone() };
    let mut m = if !c.enabled { vec![0.0; field.ground.len()] } else if spin { f::spin_up_local(c, clock, hour, &air, grid, &field) } else {
        let (o, om) = old.unwrap();
        if fresh { f::resample(&o, &om, &field, f::spin_up(c, clock, hour, &air)) } else { om }
    };
    let alt = if spin || !c.enabled { f::sun_alt(c, grid, &field, sun) } else { f::step(c, grid, &field, &mut m, dt_h, el, &air, sun) };
    let map = f::map(c, &field, &m, &alt);
    (field, m, map)
}

/// Keeps the fog state current: a running clock steps it every `rebake_s` (and when the camera moves `rebake_m`), a frozen one (shots,
/// `force`) re-spins when the hour, the weather or the field changes. The first bake runs inline so the first frame has fog.
fn bake(
    time: Res<Time>, wx: Res<Weather>, rain: Option<Res<RainState>>, mut b: ResMut<Baker>, mut params: ResMut<FogParams>, mut images: ResMut<Assets<Image>>,
    mut fog: ResMut<FogState>, cam: Query<&GlobalTransform, With<AtmosphereSettings>>,
) {
    let c = &wx.cfg.ground_fog;
    let mut done = |(field, m, map): Baked, b: &mut Baker, params: &mut FogParams| {
        let thin = m.iter().all(|&a| a < 1e-3);
        (params.gpu.map_min, params.gpu.map_cell, params.gpu.map_n) = (Vec2::new(map.min_x, map.max_z), map.cell_m, map.n);
        // No fog anywhere: an empty band skips the pass.
        (params.gpu.y_lo, params.gpu.y_hi) = if thin { (0.0, 0.0) } else { (map.y_lo, map.y_hi) };
        b.alt = map.texels.iter().map(|t| t[2]).collect();
        let _ = images.insert(&params.map, image(map.n, map.n, TextureFormat::Rgba32Float, map.texels.iter().flatten().flat_map(|v| v.to_le_bytes()).collect()));
        (b.field, b.m) = (Some(field), m);
    };
    if let Some(r) = b.task.as_mut().and_then(check_ready) { b.task = None; done(r, &mut b, &mut params); }
    if !c.enabled && !(wx.cfg.sky.sun_occlusion && b.terrain) {
        (params.gpu.y_lo, params.gpu.y_hi, b.cfg, *fog) = (0.0, 0.0, None, FogState::default());
        return;
    }
    let Some(at) = cam.iter().next().map(|g| g.translation().xz()) else { return };
    let air = Air { coverage: wx.here.coverage, wind_mps: wx.wind_mean_mps, wetness: rain.map_or(0.0, |r| r.wetness), dust: wx.here.dust };
    let frozen = wx.rate <= 0.0 || c.force.is_some();
    b.pending_h += time.delta_secs() * wx.rate;
    b.since += time.delta_secs();
    let moved = b.field.as_ref().is_none_or(|o| at.distance(Vec2::new(o.min_x, o.max_z) + Vec2::new(0.5, -0.5) * c.extent_m) > c.rebake_m);
    let origin = b.field.as_ref().map_or([f32::NAN; 2], |o| [o.min_x, o.max_z]);
    let key = [wx.hour, air.coverage, air.wind_mps, air.wetness, air.dust, origin[0], origin[1]];
    let changed = b.cfg.as_ref() != Some(c);
    let due = changed || if frozen { moved || b.key.is_none_or(|k| k.iter().zip(&key).any(|(a, b)| (a - b).abs() > 1e-3) || k[5].is_nan()) } else { moved || b.since > c.rebake_s };
    if b.task.is_none() && due {
        let spin = frozen || b.field.is_none() || changed;
        let old = b.field.clone().map(|o| (o, b.m.clone()));
        let (cfg, grid, clock, hour, dt_h) = (c.clone(), b.grid.clone(), wx.cfg.clock.clone(), wx.hour, b.pending_h);
        if b.field.is_none() {
            let r = job(&cfg, &grid, &clock, hour, air, at, old, spin, dt_h);
            done(r, &mut b, &mut params);
        } else {
            b.task = Some(AsyncComputeTaskPool::get().spawn(async move { job(&cfg, &grid, &clock, hour, air, at, old, spin, dt_h) }));
        }
        (b.pending_h, b.since, b.key, b.cfg) = (0.0, 0.0, Some(key), Some(c.clone()));
    }
    // The state at the camera, for readers.
    let el = w::sun(&wx.cfg.clock, wx.hour).0;
    if let Some(k) = b.field.as_ref().and_then(|o| o.texel(at.x, at.y)) {
        let (fld, a) = (b.field.as_ref().unwrap(), b.m[k]);
        let lit = b.alt.get(k).map_or(0.0, |&s| f::lit(c, fld.top(k, a).max(fld.ground[k]), s));
        *fog = FogState { amount: a, lift: f::lift_m(c, smoothstep(0.0, c.lift_full_deg, el), lit, a) };
    }
}

/// Per frame: lights, extinction, wind drift (frozen in shots) and the cloud shadow cookie's frame.
fn drive(
    time: Res<Time>, wx: Res<Weather>, mut params: ResMut<FogParams>, clouds: Option<Res<CloudParams>>, frame: Res<FrameCount>, mut drift: Local<Vec2>, b: Res<Baker>,
    sun: Query<(&DirectionalLight, &Transform), (With<NephtheneSun>, Without<Moon>)>, moon: Query<(&DirectionalLight, &Transform), With<Moon>>,
) {
    let c = &wx.cfg.ground_fog;
    if wx.rate > 0.0 { *drift += wx.wind.xz() * time.delta_secs(); }
    let body = |l: &DirectionalLight, t: &Transform| (t.back().as_vec3(), l.color.to_linear().to_vec3() * l.illuminance);
    params.render_scale = c.render_scale;
    let g = &mut params.gpu;
    if let Some((l, t)) = sun.iter().next() { (g.sun_dir, g.sun_color) = body(l, t); }
    if let Some((l, t)) = moon.iter().next() { (g.moon_dir, g.moon_color) = body(l, t); }
    (g.sigma, g.sigma_shallow) = f::sigma(c);
    g.sun_k = smoothstep(0.0, c.lift_full_deg, w::sun(&wx.cfg.clock, wx.hour).0);
    (g.ground_albedo, g.wisp_m, g.wisp_erode, g.diffuse_sun, g.step_power) = (c.ground_albedo, c.wisp_m, c.wisp_erode, c.diffuse_sun, c.step_power);
    let sky = &wx.cfg.sky;
    (g.occ_on, g.occ_keep, g.occ_steps) = ((sky.sun_occlusion && b.terrain) as u32 as f32, sky.occlusion_keep, sky.occlusion_steps.max(1));
    (g.albedo, g.g, g.steps, g.frame, g.max_m, g.lift_max) = (c.albedo, c.g, c.steps.max(1), frame.0, c.max_m, c.lift_max_m);
    (g.top_soft, g.edge_m, g.top_wave, g.wave_tile, g.shallow_e, g.lift_soft) = (c.top_soft_m, c.edge_m, c.top_wave_m, c.wave_tile_m, c.shallow_e_m, c.lift_soft_m);
    (g.breakup, g.breakup_tile, g.breakup_y, g.wind) = (c.breakup, c.breakup_tile_m, c.breakup_y_m, *drift);
    (g.sun_soft, g.ambient_scale, g.ambient_od, g.ms_a, g.ms_b, g.ms_octaves, g.edge_fade) = (c.sun_soft_m, c.ambient_scale, c.ambient_od, c.ms_a, c.ms_b, c.ms_octaves, c.edge_fade);
    // Cover the clouds draw: the panel / config override, else the map at the camera; none without clouds.
    let cover = clouds.as_ref().map_or(0.0, |cp| if cp.gpu.coverage_override >= 0.0 { cp.gpu.coverage_override } else { wx.here.coverage });
    // ponytail: Rayleigh scattering at sea level, Bevy's Atmosphere::earth values (sky.rs keeps its Rayleigh); read the medium to upgrade.
    (g.top_billow, g.billow_tile, g.mie_g, g.mie_beta, g.ray_beta) = (c.top_billow_m, c.billow_tile_m, sky.mie_asymmetry, sky.mie_scattering, Vec3::new(5.802e-6, 13.558e-6, 33.1e-6));
    g.hash_noise = c.hash_noise as u32 as f32;
    (g.top_fade, g.cloud_diffuse, g.cloud_cover) = (c.top_fade_m, c.cloud_diffuse, if c.ignore_cloud_sky { 0.0 } else { cover });
    if let Some(cp) = clouds {
        let s = &cp.shadow_gpu;
        (g.sh_origin, g.sh_right, g.sh_up, g.sh_half, g.sh_on) = (s.origin, s.right, s.up, wx.cfg.clouds.shadow_half_m, 1.0);
        params.cookie = cp.shadow.clone();
    }
}
