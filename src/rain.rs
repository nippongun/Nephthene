//! Rain FX, the render half. Streaks and ground splashes are quads of two static meshes whose corners the vertex shader
//! (shaders/rain.wesl) computes from the quad id, time and one uniform: no CPU particle state. The maths and hash are
//! `nephthene_core::rain` (pure, unit-tested; the shader mirrors it). Wet ground darkens and glosses entities marked [`WetGround`].
//! Reads only the `Weather` resource (map sample at the atmosphere camera, wind, clock) plus `RainState.panel` (panel toggle) and
//! `rain.force` from the config (forces rain everywhere). `RainState.intensity` / `.wetness` are for audio and gameplay.
//! Ground: streak bottoms and splashes sit on the optional [`Ground`] height grid, otherwise flat 0.
//! Snow and sleet use the same streak quads: the share `kind` of them (air `precip_kind`, or `rain.kind_force`) draw as slow, swirling round flakes.
//! Only the falling quads (a prefix of the ids) are drawn: the plugin swaps in the smallest prebuilt prefix mesh that holds them (`rain.draw_levels`).
//! Roofs: meshes marked [`Shelter`] stop rain, snow and splashes on the top of their bounding box (a world-fixed roof grid).
//! ponytail: the ground under a roof still gets wet (WetGround is one material), and shelter heights come from axis-aligned boxes.
//! Ground that opts into [`SurfaceMaterial`] darkens with wetness and turns white with snow cover (built while it snows, melted above `rain.melt_c`).
use crate::sky::Weather;
use bevy::{
    asset::embedded_asset,
    asset::RenderAssetUsages,
    camera::visibility::NoFrustumCulling,
    light::NotShadowCaster,
    camera::primitives::Aabb,
    mesh::{Indices, PrimitiveTopology},
    mesh::MeshVertexBufferLayoutRef,
    pbr::{AtmosphereSettings, ExtendedMaterial, Material, MaterialExtension, MaterialPipeline, MaterialPipelineKey, MaterialPlugin, MeshMaterial3d},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError},
    shader::ShaderRef,
};
use nephthene_core::{self as w, rain as r};

pub struct RainPlugin;
impl Plugin for RainPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/rain.wesl");
        embedded_asset!(app, "shaders/surface.wesl");
        let cfg = &app.world().resource::<Weather>().cfg.rain;
        let force = cfg.force.inspect(|x| assert!((0.0..=1.0).contains(x), "rain.force = {x}: want 0..=1"));
        let enabled = cfg.enabled;
        let started = enabled && force.is_some_and(|e| e > cfg.wet_threshold);
        let snowing = r::kind(cfg, app.world().resource::<Weather>().air.precip_kind) > 0.5;
        // A forced rain has been falling "for a while": start wet, or under snow when the air gives snow.
        let (wetness, snow_cover) = match (started, snowing) { (true, false) => (1.0, 0.0), (true, true) => (0.0, 1.0), _ => (0.0, 0.0) };
        app.add_plugins((MaterialPlugin::<RainMaterial>::default(), MaterialPlugin::<SurfaceMaterial>::default()))
            .insert_resource(RainState { force, wetness, snow_cover, ..default() });
        // Off: no quads and no rain systems, so nothing runs per frame. RainState stays at rest (intensity 0, wetness 0).
        if enabled {
            app.add_systems(PostStartup, spawn.after(crate::sky::setup))
                .add_systems(Update, (roofs, state.before(crate::NephtheneSet::Drive), drive, wet_ground, surface).chain().after(crate::NephtheneSet::Tick));
        }
    }
}

/// Marks a mesh whose `StandardMaterial` gets wet (darker, glossier) in rain. Its dry roughness and reflectance are read at the first sight.
#[derive(Component)]
pub struct WetGround;

/// Marks a mesh precipitation stops on (a roof, an overhang): rain, snow and splashes land on the top of its world bounding box and
/// none falls below it. The box is axis-aligned, so mark roof meshes, not whole buildings with open sides.
#[derive(Component)]
pub struct Shelter;

/// The weather surface: a [`StandardMaterial`] that darkens and glosses with wetness and turns white with snow cover where the ground
/// is flat enough to hold it. Hosts opt in by giving ground meshes a [`SurfaceMaterial`]; the plugin writes the weather into every one.
pub type SurfaceMaterial = ExtendedMaterial<StandardMaterial, WeatherSurface>;

/// The weather part of a [`SurfaceMaterial`]; start from `WeatherSurface::default()`.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct WeatherSurface { #[uniform(100)] gpu: SurfaceGpu }
impl Default for WeatherSurface { fn default() -> Self { Self { gpu: surface_gpu(&w::RainCfg::default(), 0.0, 0.0) } } }
impl MaterialExtension for WeatherSurface {
    fn fragment_shader() -> ShaderRef { "embedded://nephthene/shaders/surface.wesl".into() }
}

/// Uniform for surface.wesl (field order identical in the shader).
#[derive(Clone, Copy, Default, ShaderType)]
struct SurfaceGpu {
    snow_color: Vec3, wet: f32, snow: f32, wet_darken: f32, wet_roughness: f32, snow_roughness: f32, slope_lo: f32, slope_hi: f32,
}

fn surface_gpu(c: &w::RainCfg, wet: f32, snow: f32) -> SurfaceGpu {
    SurfaceGpu { snow_color: Vec3::from(c.snow_color), wet, snow, wet_darken: c.wet_darken, wet_roughness: c.wet_roughness, snow_roughness: c.snow_roughness, slope_lo: c.cover_slope_lo, slope_hi: c.cover_slope_hi }
}

/// Optional terrain heights the rain and rain shafts stop on; absent = flat 0. Insert before PostStartup.
#[derive(Resource)]
pub struct Ground(pub r::HeightGrid);

/// The roof grid of every [`Shelter`] and its texture (rain.wesl `shelter`).
#[derive(Resource)]
struct Roofs { grid: r::HeightGrid, tex: Handle<Image> }

/// Live rain. `intensity` 0..1 at the camera, `wetness` 0..1 of the ground, `vel` the drop velocity (m/s). `snow_vel` the flake velocity (m/s),
/// `kind` the precipitation kind 0 rain .. 1 snow (sleet between) at the camera, `sheltered` the camera stands under a [`Shelter`]. `panel` is the panel toggle's
/// forced precip, `force` is the config's `rain.force`; either replaces the map. `snow_cover` 0..1 the ground's snow (live model; a [`SurfaceMaterial`]
/// shows it, or the climate map's ground snow if more).
#[derive(Resource, Default, Clone, Debug)]
pub struct RainState { pub intensity: f32, pub wetness: f32, pub snow_cover: f32, pub vel: Vec3, pub snow_vel: Vec3, pub kind: f32, pub sheltered: bool, pub panel: Option<f32>, pub force: Option<f32> }
impl RainState {
    pub fn slant_deg(&self) -> f32 { r::slant_deg(self.vel) }
    pub fn source(&self) -> &'static str { if self.panel.is_some() { "panel" } else if self.force.is_some() { "config rain.force" } else { "weather map" } }
}

/// Uniform for rain.wesl (field order identical in the shader).
#[derive(Clone, Copy, Default, ShaderType)]
struct RainGpu {
    light: Vec3, time: f32,
    vel: Vec3, len: f32,
    sun_dir: Vec3, intensity: f32,
    half: f32, height: f32, ground_y: f32, mode: u32,
    width: f32, alpha: f32, life: f32, count: u32,
    g_min: Vec2, g_cell: f32, g_w: u32, g_h: u32,
    snow_vel: Vec3, kind: f32, flake: f32, flake_alpha: f32, flutter_m: f32, flutter_hz: f32, snow_half: f32,
    s_min: Vec2, s_cell: f32, s_w: u32, s_h: u32,
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct RainMaterial { #[uniform(0)] gpu: RainGpu, #[texture(1, sample_type = "float", filterable = false)] ground: Handle<Image>, #[texture(2, sample_type = "float", filterable = false)] shelter: Handle<Image> }
impl Material for RainMaterial {
    fn vertex_shader() -> ShaderRef { "embedded://nephthene/shaders/rain.wesl".into() }
    fn fragment_shader() -> ShaderRef { "embedded://nephthene/shaders/rain.wesl".into() }
    fn alpha_mode(&self) -> AlphaMode { AlphaMode::Premultiplied }
    fn enable_prepass() -> bool { false }
    fn enable_shadows() -> bool { false }
    // Camera-facing ribbons: winding depends on the view, so draw both faces.
    fn specialize(_: &MaterialPipeline, d: &mut RenderPipelineDescriptor, _: &MeshVertexBufferLayoutRef, _: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        d.primitive.cull_mode = None;
        Ok(())
    }
}

/// The terrain height source: the [`Ground`] resource when present, otherwise flat 0.
pub fn ground_grid(ground: Option<&Ground>) -> r::HeightGrid {
    ground.map_or_else(|| r::HeightGrid::flat(0.0), |g| g.0.clone())
}

/// The grid as an R32Float texture (rain.wesl and clouds_shaft.wesl read it with textureLoad).
pub fn ground_image(g: &r::HeightGrid) -> Image {
    Image::new(bevy::render::render_resource::Extent3d { width: g.w, height: g.h, depth_or_array_layers: 1 }, bevy::render::render_resource::TextureDimension::D2,
        g.y.iter().flat_map(|v| v.to_le_bytes()).collect(), bevy::render::render_resource::TextureFormat::R32Float, RenderAssetUsages::RENDER_WORLD)
}

/// The material and the prefix meshes (quad count, mesh), largest first.
#[derive(Component)]
struct RainQuads(Handle<RainMaterial>, Vec<(u32, Handle<Mesh>)>);

/// `n` quads, vertex = (corner x, corner y, quad id).
pub(crate) fn quads(n: u32) -> Mesh {
    let pos: Vec<[f32; 3]> = (0..n).flat_map(|q| [[0.0, 0.0, q as f32], [1.0, 0.0, q as f32], [1.0, 1.0, q as f32], [0.0, 1.0, q as f32]]).collect();
    let idx: Vec<u32> = (0..n).flat_map(|i| [4 * i, 4 * i + 1, 4 * i + 2, 4 * i, 4 * i + 2, 4 * i + 3]).collect();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_indices(Indices::U32(idx))
}

fn spawn(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<RainMaterial>>, mut images: ResMut<Assets<Image>>, ground: Option<Res<Ground>>, wx: Res<Weather>) {
    let grid = ground_grid(ground.as_deref());
    let c = &wx.cfg.rain;
    let tex = images.add(ground_image(&grid));
    let roofs = r::shelter_grid(&[], c.shelter_cell_m, c.shelter_max_cells);
    let roof_tex = images.add(ground_image(&roofs));
    for (mode, n, width, alpha) in [(0, c.max_streaks, c.streak_width_m, c.streak_alpha), (1, c.max_splashes, 0.0, c.splash_alpha)] {
        let gpu = RainGpu { half: c.half_m, height: c.height_m, mode, width, alpha, life: c.splash_life_s, count: n,
            g_min: Vec2::new(grid.min_x, grid.max_z), g_cell: grid.cell_m, g_w: grid.w, g_h: grid.h, ..default() };
        let m = mats.add(RainMaterial { gpu, ground: tex.clone(), shelter: roof_tex.clone() });
        let levels: Vec<(u32, Handle<Mesh>)> = (0..=c.draw_levels).map(|k| n >> k).filter(|&m| m > 0).collect::<std::collections::BTreeSet<_>>().into_iter().rev().map(|m| (m, meshes.add(quads(m)))).collect();
        let mesh = levels.first().map(|l| l.1.clone()).unwrap_or_else(|| meshes.add(quads(n)));
        commands.spawn((Name::new(if mode == 0 { "RainStreaks" } else { "RainSplashes" }), Mesh3d(mesh), MeshMaterial3d(m.clone()),
            RainQuads(m, levels), NoFrustumCulling, NotShadowCaster, Visibility::Hidden));
    }
    commands.insert_resource(Roofs { grid: roofs, tex: roof_tex });
}

/// Rebuilds the roof grid when a [`Shelter`] is added, moved, resized or removed (bounding boxes come from Bevy's `Aabb`).
fn roofs(all: Query<(&Aabb, &GlobalTransform), With<Shelter>>, changed: Query<(), (With<Shelter>, Or<(Changed<Aabb>, Changed<GlobalTransform>)>)>,
    mut removed: RemovedComponents<Shelter>, wx: Res<Weather>, roofs: Option<ResMut<Roofs>>, mut images: ResMut<Assets<Image>>) {
    let Some(mut roofs) = roofs else { return };
    if changed.is_empty() && removed.read().count() == 0 { return; }
    let c = &wx.cfg.rain;
    let boxes: Vec<(Vec3, Vec3)> = if c.ignore_shelter { vec![] } else { all.iter().map(|(a, t)| world_box(a, t)).collect() };
    roofs.grid = r::shelter_grid(&boxes, c.shelter_cell_m, c.shelter_max_cells);
    if let Some(mut img) = images.get_mut(&roofs.tex) { *img = ground_image(&roofs.grid); }
}

/// World-space min/max corners of a mesh's local bounding box.
fn world_box(a: &Aabb, t: &GlobalTransform) -> (Vec3, Vec3) {
    let (c, e) = (Vec3::from(a.center), Vec3::from(a.half_extents));
    (0..8).map(|i| t.transform_point(c + e * Vec3::new(if i & 1 == 0 { -1.0 } else { 1.0 }, if i & 2 == 0 { -1.0 } else { 1.0 }, if i & 4 == 0 { -1.0 } else { 1.0 })))
        .fold((Vec3::MAX, Vec3::MIN), |(l, h), p| (l.min(p), h.max(p)))
}

fn state(time: Res<Time>, wx: Res<Weather>, mut rain: ResMut<RainState>, cam: Query<&GlobalTransform, With<AtmosphereSettings>>, roofs: Option<Res<Roofs>>) {
    let force = rain.panel.or(rain.force);
    let c = &wx.cfg.rain;
    rain.intensity = r::intensity(c, &wx.here, force);
    // The live wind (panel override included) plus the storms' gust front.
    rain.vel = r::velocity(c, wx.wind + Vec3::new(wx.storm.outflow.x, 0.0, wx.storm.outflow.y), rain.intensity);
    rain.kind = r::kind(c, wx.air.precip_kind);
    rain.snow_vel = r::snow_velocity(c, wx.wind + Vec3::new(wx.storm.outflow.x, 0.0, wx.storm.outflow.y));
    // Snow does not wet the ground: only the liquid share counts.
    rain.wetness = r::wetness_step(c, rain.wetness, r::liquid(rain.intensity, rain.kind), time.delta_secs());
    rain.snow_cover = r::snow_cover_step(c, rain.snow_cover, rain.intensity, rain.kind, wx.air.temp_c, time.delta_secs());
    rain.sheltered = roofs.as_deref().zip(cam.iter().next()).is_some_and(|(s, t)| r::sheltered(&s.grid, t.translation()));
}

/// Rain light before exposure: the exposure curve's scene level (so streaks sit at the same display brightness the sun/sky give surfaces),
/// the sun part dimmed under cloud and by night. Constants: config rain.light_* (untuned).
fn drive(time: Res<Time>, wx: Res<Weather>, rain: Res<RainState>, roofs: Option<Res<Roofs>>, mut mats: ResMut<Assets<RainMaterial>>, mut q: Query<(&RainQuads, &mut Visibility, &mut Mesh3d)>) {
    let c = &wx.cfg.rain;
    let (el, az) = w::sun(&wx.cfg.clock, wx.hour);
    let day = r::smoothstep(c.day_from_deg, c.day_to_deg, el);
    let sun_vis = 1.0 - c.cloud_dim * r::smoothstep(c.dim_from, c.dim_to, wx.here.coverage);
    let level = c.level_scale * 2f32.powf(w::curve(&wx.cfg.exposure, el));
    let light = Vec3::from(c.light_color) * c.light_gain * level * (day * (c.sky_share + c.sun_share * sun_vis) + c.night_floor);
    let on = rain.intensity > 0.01;
    for (m, mut vis, mut mesh) in &mut q {
        let v = if on { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != v { *vis = v; }
        if !on { continue; }
        let Some(mut mat) = mats.get_mut(&m.0) else { continue };
        let g = &mut mat.gpu;
        (g.light, g.time, g.vel, g.len, g.sun_dir, g.intensity, g.ground_y) =
            (light, time.elapsed_secs() % 4096.0, rain.vel, r::streak_len(c, rain.vel), w::dir(el, az), rain.intensity, 0.0);
        (g.snow_vel, g.kind, g.flake, g.flake_alpha, g.flutter_m, g.flutter_hz, g.snow_half) = (rain.snow_vel, rain.kind, c.flake_m, c.flake_alpha, c.flutter_m, c.flutter_hz, c.snow_half_m);
        if let Some(s) = roofs.as_deref() { (g.s_min, g.s_cell, g.s_w, g.s_h) = (Vec2::new(s.grid.min_x, s.grid.max_z), s.grid.cell_m, s.grid.w, s.grid.h); }
        // Splashes come from the liquid share only.
        if g.mode == 1 { g.intensity = r::liquid(rain.intensity, rain.kind); }
        // Draw the smallest prefix mesh that holds the falling quads.
        let want = r::draw_quads(g.count, g.intensity, c.draw_levels);
        if let Some((_, h)) = m.1.iter().rev().find(|(n, _)| *n >= want) { if mesh.0 != *h { mesh.0 = h.clone(); } }
    }
}

/// Wet ground: darkens and glosses the `StandardMaterial` of entities marked [`WetGround`], from their dry roughness and reflectance (read once).
fn wet_ground(rain: Res<RainState>, wx: Res<Weather>, ground: Query<&MeshMaterial3d<StandardMaterial>, With<WetGround>>, mut mats: ResMut<Assets<StandardMaterial>>, mut last: Local<f32>, mut dry: Local<std::collections::HashMap<AssetId<StandardMaterial>, (Color, f32, f32)>>) {
    if (rain.wetness - *last).abs() < wx.cfg.rain.wet_epsilon { return; }
    for m in &ground {
        let Some(mut mat) = mats.get_mut(&m.0) else { continue };
        let (base, rough0, refl0) = *dry.entry(m.0.id()).or_insert((mat.base_color, mat.perceptual_roughness, mat.reflectance));
        let (k, rough, refl) = r::wet_look(&wx.cfg.rain, rain.wetness, rough0, refl0);
        let b = base.to_linear();
        (mat.base_color, mat.perceptual_roughness, mat.reflectance) = (Color::linear_rgba(b.red * k, b.green * k, b.blue * k, b.alpha), rough, refl);
        *last = rain.wetness;
    }
}

/// Writes the weather into every [`SurfaceMaterial`]: wetness and snow cover (`rain::snow_cover`: the live cover or the climate map's
/// ground snow at the camera). Only when either moved by `rain.wet_epsilon` or a material was added, so the materials are not rewritten each frame.
fn surface(rain: Res<RainState>, wx: Res<Weather>, mut mats: ResMut<Assets<SurfaceMaterial>>, mut last: Local<(f32, f32, usize)>) {
    let c = &wx.cfg.rain;
    let snow = r::snow_cover(c, rain.snow_cover, wx.climate_here.snow);
    let moved = (rain.wetness - last.0).abs() >= c.wet_epsilon || (snow - last.1).abs() >= c.wet_epsilon;
    if !moved && mats.len() == last.2 { return; }
    // A newly added material gets the last written values, so a rewrite for it does not move the weather.
    let (wet, snow) = if moved { (rain.wetness, snow) } else { (last.0, last.1) };
    for (_, m) in mats.iter_mut() { m.extension.gpu = surface_gpu(c, wet, snow); }
    *last = (wet, snow, mats.len());
}
