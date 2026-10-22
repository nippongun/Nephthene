//! Rain FX, the render half. Streaks and ground splashes are quads of two static meshes whose corners the vertex shader
//! (shaders/rain.wesl) computes from the quad id, time and one uniform: no CPU particle state. The maths and hash are
//! `nephthene_core::rain` (pure, unit-tested; the shader mirrors it). Wet ground darkens and glosses entities marked [`WetGround`].
//! Reads only the `Weather` resource (map sample at the atmosphere camera, wind, clock) plus `RainState.panel` (panel toggle) and
//! `rain.force` from the config (forces rain everywhere). `RainState.intensity` / `.wetness` are for audio and gameplay.
//! Ground: streak bottoms and splashes sit on the optional [`Ground`] height grid, otherwise flat 0.
//! ponytail: no roof occlusion (rain also falls indoors).
use crate::sky::Weather;
use bevy::{
    asset::embedded_asset,
    asset::RenderAssetUsages,
    camera::visibility::NoFrustumCulling,
    light::NotShadowCaster,
    mesh::{Indices, PrimitiveTopology},
    mesh::MeshVertexBufferLayoutRef,
    pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin, MeshMaterial3d},
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
        let cfg = &app.world().resource::<Weather>().cfg.rain;
        let force = cfg.force.inspect(|x| assert!((0.0..=1.0).contains(x), "rain.force = {x}: want 0..=1"));
        // A forced rain has been falling "for a while": start wet.
        let wetness = if force.is_some_and(|e| e > cfg.wet_threshold) { 1.0 } else { 0.0 };
        app.add_plugins(MaterialPlugin::<RainMaterial>::default())
            .insert_resource(RainState { force, wetness, ..default() })
            .add_systems(PostStartup, spawn.after(crate::sky::setup))
            .add_systems(Update, (state.before(crate::NephtheneSet::Drive), drive, wet_ground).chain().after(crate::NephtheneSet::Tick));
    }
}

/// Marks a mesh whose `StandardMaterial` gets wet (darker, glossier) in rain. Its dry roughness and reflectance are read at the first sight.
#[derive(Component)]
pub struct WetGround;

/// Optional terrain heights the rain and rain shafts stop on; absent = flat 0. Insert before PostStartup.
#[derive(Resource)]
pub struct Ground(pub r::HeightGrid);

/// Live rain. `intensity` 0..1 at the camera, `wetness` 0..1 of the ground, `vel` the drop velocity (m/s). `panel` is the panel toggle's
/// forced precip, `force` is the config's `rain.force`; either replaces the map.
#[derive(Resource, Default, Clone, Debug)]
pub struct RainState { pub intensity: f32, pub wetness: f32, pub vel: Vec3, pub panel: Option<f32>, pub force: Option<f32> }
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
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct RainMaterial { #[uniform(0)] gpu: RainGpu, #[texture(1, sample_type = "float", filterable = false)] ground: Handle<Image> }
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

#[derive(Component)]
struct RainQuads(Handle<RainMaterial>);

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
    for (mode, n, width, alpha) in [(0, c.max_streaks, c.streak_width_m, c.streak_alpha), (1, c.max_splashes, 0.0, c.splash_alpha)] {
        let gpu = RainGpu { half: c.half_m, height: c.height_m, mode, width, alpha, life: c.splash_life_s, count: n,
            g_min: Vec2::new(grid.min_x, grid.max_z), g_cell: grid.cell_m, g_w: grid.w, g_h: grid.h, ..default() };
        let m = mats.add(RainMaterial { gpu, ground: tex.clone() });
        commands.spawn((Name::new(if mode == 0 { "RainStreaks" } else { "RainSplashes" }), Mesh3d(meshes.add(quads(n))), MeshMaterial3d(m.clone()),
            RainQuads(m), NoFrustumCulling, NotShadowCaster, Visibility::Hidden));
    }
}

fn state(time: Res<Time>, wx: Res<Weather>, mut rain: ResMut<RainState>) {
    let force = rain.panel.or(rain.force);
    let c = &wx.cfg.rain;
    rain.intensity = r::intensity(c, &wx.here, force);
    // The live wind (panel override included) plus the storms' gust front.
    rain.vel = r::velocity(c, wx.wind + Vec3::new(wx.storm.outflow.x, 0.0, wx.storm.outflow.y), rain.intensity);
    rain.wetness = r::wetness_step(c, rain.wetness, rain.intensity, time.delta_secs());
}

/// Rain light before exposure: the exposure curve's scene level (so streaks sit at the same display brightness the sun/sky give surfaces),
/// the sun part dimmed under cloud and by night. Constants: config rain.light_* (untuned).
fn drive(time: Res<Time>, wx: Res<Weather>, rain: Res<RainState>, mut mats: ResMut<Assets<RainMaterial>>, mut q: Query<(&RainQuads, &mut Visibility)>) {
    let c = &wx.cfg.rain;
    let (el, az) = w::sun(&wx.cfg.clock, wx.hour);
    let day = r::smoothstep(c.day_from_deg, c.day_to_deg, el);
    let sun_vis = 1.0 - c.cloud_dim * r::smoothstep(c.dim_from, c.dim_to, wx.here.coverage);
    let level = c.level_scale * 2f32.powf(w::curve(&wx.cfg.exposure, el));
    let light = Vec3::from(c.light_color) * c.light_gain * level * (day * (c.sky_share + c.sun_share * sun_vis) + c.night_floor);
    let on = rain.intensity > 0.01;
    for (m, mut vis) in &mut q {
        let v = if on { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != v { *vis = v; }
        if !on { continue; }
        let Some(mut mat) = mats.get_mut(&m.0) else { continue };
        let g = &mut mat.gpu;
        (g.light, g.time, g.vel, g.len, g.sun_dir, g.intensity, g.ground_y) =
            (light, time.elapsed_secs() % 4096.0, rain.vel, r::streak_len(c, rain.vel), w::dir(el, az), rain.intensity, 0.0);
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
