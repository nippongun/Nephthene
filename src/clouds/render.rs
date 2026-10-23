//! Clouds, render world: `clouds_march` renders the cloud layer (clouds_march.wesl) into a per-view buffer before the
//! opaque pass, `CloudComposite` blends it over the sky-view LUT sky as the first Transparent3d item (after Bevy's
//! render_sky, before glass), depth-tested per MSAA sample at the cloud front's depth (clouds.over_terrain), so clouds cover terrain behind them and terrain hides clouds behind it.
//! The march runs at `render_scale` with a per-frame jitter; `clouds_resolve` reprojects last frame's resolved buffer with the
//! previous view and blends it in (EMA, 3x3 clamp); the composite upsamples the resolved buffer. The composite needs no viewport
//! handling: the transparent pass sets the camera viewport (with MainPassResolutionOverride), so the fullscreen uv spans the drawn area.
use std::collections::HashMap;
use bevy::{
    camera::MainPassResolutionOverride,
    core_pipeline::{
        FullscreenShader,
        core_3d::{CORE_3D_DEPTH_FORMAT, Transparent3d, TransparentSortingInfo3d, main_opaque_pass_3d, main_transparent_pass_3d},
        schedule::Core3d, Core3dSystems,
    },
    diagnostic::FrameCount,
    ecs::{query::ROQueryItem, system::{SystemParamItem, lifetimeless::Read}},
    pbr::{ExtractedAtmosphere, GpuAtmosphereSettings, GpuScatteringMedium, ScatteringMediumSampler, resources::{AtmosphereSampler, AtmosphereTextures, AtmosphereTransform, AtmosphereTransforms, AtmosphereTransformsOffset, GpuAtmosphere}},
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_component::{ComponentUniforms, DynamicUniformIndex},
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        render_phase::{AddRenderCommand, DrawFunctions, PhaseItem, PhaseItemExtraIndex, RenderCommand, RenderCommandResult, SetItemPipeline, TrackedRenderPass, ViewSortedRenderPhases},
        render_resource::{binding_types::{sampler, texture_2d, texture_3d, uniform_buffer}, *},
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        sync_world::MainEntity,
        texture::{CachedTexture, GpuImage, TextureCache},
        view::{ExtractedView, Msaa, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms},
    },
    shader::Shader,
};

const MARCH: &str = "embedded://nephthene/shaders/clouds_march.wesl";
const COMPOSITE: &str = "embedded://nephthene/shaders/clouds_composite.wesl";
const RESOLVE: &str = "embedded://nephthene/shaders/clouds_resolve.wesl";
const SHADOW: &str = "embedded://nephthene/shaders/clouds_shadow.wesl";
const SHAFT: &str = "embedded://nephthene/shaders/clouds_shaft.wesl";
const SHAFT_RESOLVE: &str = "embedded://nephthene/shaders/clouds_shaft_resolve.wesl";
const SHAFT_COMP: &str = "embedded://nephthene/shaders/clouds_shaft_comp.wesl";

/// Uniform for clouds_march.wesl; field order identical in the shader. All values come from the config (untuned).
#[derive(Clone, Copy, Default, ShaderType)]
pub struct CloudGpu {
    pub sun_dir: Vec3, pub coverage_override: f32,
    pub sun_color: Vec3, pub ambient_scale: f32,
    pub moon_dir: Vec3, pub extinction: f32,
    pub moon_color: Vec3, pub max_distance: f32,
    pub map_origin: Vec2, pub map_size: Vec2,
    pub world_offset: Vec2, pub wind: Vec2,
    pub bottom: f32, pub top: f32, pub base_tile: f32, pub detail_tile: f32,
    pub detail_strength: f32, pub steps: u32, pub light_far: f32, pub light_first: f32,
    pub light_gain: f32, pub shoulder: f32, pub shoulder_limit: f32, pub frame: u32,
    /// debug_view probe mode (0 = the normal look).
    pub debug: u32,
    pub base_tile2: f32, pub cov_var: f32, pub curl: f32,
    /// Panel rain toggle: >= 0 replaces the map's precip, -1 = the map.
    pub precip_override: f32,
    /// HZD 4x4 update: this frame's offset in each 4x4 block and the render size (set per frame in prepare_textures_and_uniform).
    pub march_ox: f32, pub march_oy: f32, pub full_w: f32, pub full_h: f32, pub march_div: f32,
    /// Tier-2 towers: 4 (type, depth m) stops padded with the last, tower_d.x <= 0 = off. `background`: (coverage, precip, type, 1) outside the map uv, w = 0 = off.
    pub tower_t: Vec4, pub tower_d: Vec4, pub background: Vec4,
    /// clouds.look, resolve (light jitter) and storms.render constants: see clouds_common.wesl `Clouds`.
    pub cov_boost: f32, pub tower_vsq: f32, pub tower_billow: f32, pub background_blend: f32,
    pub vary_tile_mul: f32, pub horizon_step_boost: f32, pub horizon_step_y: f32, pub cheap_empty_steps: f32,
    pub light_jitter_shared: f32, pub jitter_step: f32, pub light_jitter_offset: Vec2,
    pub s_freq: f32, pub s_vsq: f32, pub s_vsq_low: f32, pub s_amb_floor: f32,
    pub s_amb_var: f32, pub s_erode: f32, pub s_gain: f32, pub s_od_per_m: f32,
    pub s_amb_k: f32, pub s_detail_m: f32, pub s_field_floor: f32, pub s_range: f32,
    pub s_steps: f32, pub s_ds_min: f32, pub s_near: f32, pub s_sun_len: f32,
    pub s_col_len: f32, pub s_core_flare: f32, pub s_anvil_wide: f32, pub s_anvil_p: f32,
    pub s_shelf_ceil: f32,
    /// storms.fx.curtain_full and storms.couple (1 = rain shafts fall under storm cells).
    pub s_curtain_full: f32, pub s_couple: f32,
    /// Orographic tile: world NW corner (m), side (m, 0 = none) and cover weight. See clouds_common.wesl `Clouds`.
    pub oro_origin: Vec2, pub oro_size: f32, pub oro_weight: f32,
    /// clouds.air_guard (1 = guard the air transmittance in front of clouds and shafts, see clouds_common.wesl `air_segment`).
    pub air_guard: u32,
    /// clouds.over_terrain (1 = clouds drawn in front of terrain, the march ends at it) and the terrain grid the march reads at cloud
    /// binding 10 (rain::HeightGrid: NW cell corner x and z, cell m, columns, rows).
    pub over_terrain: u32, pub g_min: Vec2, pub g_cell: f32, pub g_w: u32, pub g_h: u32,
}

/// Uniform for clouds_shadow.wesl (field order identical in the shader): the cookie's light-plane frame, see nephthene_core shadow_*.
#[derive(Clone, Copy, Default, ShaderType)]
pub struct ShadowGpu {
    pub origin: Vec3, pub half_m: f32,
    pub right: Vec3, pub strength: f32,
    pub up: Vec3, pub samples: u32,
}

/// Uniform for clouds_shaft.wesl (field order identical in the shader): rain-shaft lean, on/off, and the ground height grid.
#[derive(Clone, Copy, Default, ShaderType)]
pub struct ShaftGpu {
    pub slant: Vec2, pub scale: f32, pub g_cell: f32,
    pub g_min: Vec2, pub g_w: u32, pub g_h: u32,
    /// rain.precip_lo/hi and shafts.*: see clouds_shaft.wesl `Shaft`.
    pub precip_lo: f32, pub precip_hi: f32, pub sigma: f32, pub near_m: f32,
    pub fade_m: f32, pub max_m: f32, pub xz_m: f32, pub y_m: f32,
    pub ground_keep: f32, pub cloud_lo: f32, pub cloud_hi: f32, pub streak_lo: f32,
    pub streak_hi: f32, pub gap: f32, pub albedo: f32, pub light_od: f32,
    pub steps: f32,
}

/// Uniform at cloud bind group binding 6 (clouds_common.wesl `Storms`): live cell count and `nephthene_core::storm::pack`'s 4 vec4 per cell, world xz.
#[derive(Clone, Copy, ShaderType)]
pub struct StormGpu { pub n: u32, pub pad0: u32, pub pad1: u32, pub pad2: u32, pub cells: [Vec4; 64] }
impl Default for StormGpu { fn default() -> Self { Self { n: 0, pad0: 0, pad1: 0, pad2: 0, cells: [Vec4::ZERO; 64] } } }

/// Uniform at cloud bind group binding 7 (clouds_march.wesl `Flashes`): live flashes lighting the clouds, world frame. `p[i]` = position
/// (xyz) and glow radius (w, m); `l[i]` = colour x `lightning::cloud_flash_scale` (rgb, lux m^2). Filled by src/lightning.rs.
#[derive(Clone, Copy, ShaderType)]
pub struct FlashGpu { pub n: u32, pub pad0: u32, pub pad1: u32, pub pad2: u32, pub p: [Vec4; 4], pub l: [Vec4; 4] }
impl Default for FlashGpu { fn default() -> Self { Self { n: 0, pad0: 0, pad1: 0, pad2: 0, p: [Vec4::ZERO; 4], l: [Vec4::ZERO; 4] } } }

/// Main world -> render world, every frame.
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub struct CloudParams {
    pub gpu: CloudGpu,
    pub base: Handle<Image>, pub detail: Handle<Image>, pub map: Handle<Image>,
    /// The orographic tile (Rgba8: R cover, G precip), a 1x1 empty image until one bakes (`gpu.oro_size` 0).
    pub oro: Handle<Image>,
    /// The map sampler repeats (config `map.wrap`) instead of clamping at the edge.
    pub map_wrap: bool,
    pub render_scale: f32, pub temporal_alpha: f32, pub update_div: u32,
    /// Cloud shadow cookie image (R8Unorm), its frame, and whether to redraw it this frame.
    pub shadow: Handle<Image>, pub shadow_gpu: ShadowGpu, pub shadow_draw: bool,
    /// Incremental redraw: this frame redraws band `.0` of `.1` horizontal bands of the cookie (config clouds.shadow_every bands, one per frame).
    pub shadow_band: (u32, u32),
    /// Rain shafts (clouds_shaft.wesl): uniform and the R32Float ground heights it stops on.
    pub shaft: ShaftGpu, pub ground: Handle<Image>,
    /// Selected storm cells (storms.rs).
    pub storm: StormGpu,
    /// Live lightning flashes lighting the clouds (src/lightning.rs).
    pub flash: FlashGpu,
    /// The resolve tuning and test switches (config resolve), and the pass-timing switch.
    pub resolve: nephthene_core::config::Resolve, pub pass_timing: bool,
}

/// Uniform for clouds_resolve.wesl (field order identical in the shader).
#[derive(Clone, Copy, Default, ShaderType)]
struct ResolveGpu {
    // Translation-free (camera-relative) reprojection: world-scale cam positions (terrain scenes, ~1e6 m) quantise a near-plane unproject.
    prev_clip_from_ray: Mat4, ray_from_clip: Mat4,
    delta: Vec3, alpha: f32, max_distance: f32, reset: u32, ox: u32, oy: u32,
    /// resolve.no_reproject: history sampled where the pixel is, not where it was (test red arm: a turning camera ghosts).
    noreproj: u32, div: u32, n: u32,
    reject_gain: f32, still_px: f32, fresh: f32, conf_max: f32, upsample_mean_frames: f32,
    motion_half_px: f32, wide_upsample: f32,
}

/// HZD's 16-frame cover order of a 4x4 block (value = x + 4y). untuned.
const BAYER16: [u32; 16] = [0, 10, 2, 8, 5, 15, 7, 13, 1, 11, 3, 9, 4, 14, 6, 12];

/// Whether to build the timestamp query set (config clouds.pass_timing), handed to the render world at startup.
#[derive(Resource, Clone, Copy)]
pub struct TimingOn(pub bool);

pub fn plugin(app: &mut App, timing: bool) {
    app.add_plugins(ExtractResourcePlugin::<CloudParams>::default());
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app
        .insert_resource(TimingOn(timing))
        .add_render_command::<Transparent3d, (SetItemPipeline, DrawCloudComposite)>()
        .add_render_command::<Transparent3d, (SetItemPipeline, DrawCloudShaft)>()
        .add_systems(RenderStartup, init)
        .add_systems(Render, (
            read_timing.in_set(RenderSystems::Cleanup),
            prepare_textures_and_uniform.in_set(RenderSystems::PrepareResources),
            (queue_composite, queue_shaft).in_set(RenderSystems::Queue),
            prepare_bind_groups.in_set(RenderSystems::PrepareBindGroups),
        ))
        .add_systems(Core3d, clouds_march.in_set(Core3dSystems::MainPass).before(main_opaque_pass_3d))
        .add_systems(Core3d, (transparent_begin.before(main_transparent_pass_3d), transparent_end.after(main_transparent_pass_3d)).in_set(Core3dSystems::MainPass));
}

/// clouds.pass_timing: GPU pass-boundary timestamps (Metal on Apple Silicon has no inside-encoder ones) for six spans, timestamp pair i = 2i, 2i+1:
/// shadow, march, resolve, shaft_march, shaft_resolve, transparent (the whole main transparent pass, see transparent_begin).
/// `ran` bit i is set when pass i begins this frame; read_timing adds only the bits set, so a skipped pass counts 0 (not last frame's stale value).
/// Prints `PASSMS` every 60 frames (sum / 60: mean ms per pass per frame). Off: no query set, no readback.
#[derive(Resource)]
struct PassTiming { qs: wgpu::QuerySet, resolve: wgpu::Buffer, read: wgpu::Buffer, sums: [f64; 6], n: u32, ran: std::sync::atomic::AtomicU32 }
const PASSES: [&str; 6] = ["shadow", "march", "resolve", "shaft_march", "shaft_resolve", "transparent"];

#[derive(Resource)]
struct CloudRender {
    atmo: BindGroupLayoutDescriptor, cloud: BindGroupLayoutDescriptor, composite: BindGroupLayoutDescriptor, resolve: BindGroupLayoutDescriptor,
    march: CachedRenderPipelineId, shadow_layout: BindGroupLayoutDescriptor, shadow_pipeline: CachedRenderPipelineId, shadow_ubo: UniformBuffer<ShadowGpu>, storm_ubo: UniformBuffer<StormGpu>, flash_ubo: UniformBuffer<FlashGpu>, resolve_pipeline: CachedRenderPipelineId, composite_shader: Handle<Shader>, fullscreen: FullscreenShader,
    repeat: Sampler, clamp: Sampler, ubo: UniformBuffer<CloudGpu>, renderer: Entity,
    composite_pipelines: HashMap<(u32, TextureFormat), CachedRenderPipelineId>,
    shaft_layout: BindGroupLayoutDescriptor, shaft_comp: BindGroupLayoutDescriptor, shaft_march: CachedRenderPipelineId, shaft_resolve: CachedRenderPipelineId, shaft_shader: Handle<Shader>, shaft_ubo: UniformBuffer<ShaftGpu>, shaft_renderer: Entity,
    shaft_pipelines: HashMap<(u32, TextureFormat), CachedRenderPipelineId>,
}

fn init(mut commands: Commands, device: Res<RenderDevice>, assets: Res<AssetServer>, fullscreen: Res<FullscreenShader>, cache: Res<PipelineCache>, timing_on: Res<TimingOn>) {
    use ShaderStages as S;
    // Same indices as Bevy's render_sky layout, so the shader imports bevy_pbr::atmosphere::bindings unchanged.
    let atmo = BindGroupLayoutDescriptor::new("cloud_atmosphere", &BindGroupLayoutEntries::with_indices(S::FRAGMENT, (
        (0, uniform_buffer::<GpuAtmosphere>(true)), (1, uniform_buffer::<GpuAtmosphereSettings>(true)),
        (2, uniform_buffer::<AtmosphereTransform>(true)), (3, uniform_buffer::<ViewUniform>(true)),
        (5, texture_2d(TextureSampleType::default())), (7, sampler(SamplerBindingType::Filtering)),
        (8, texture_2d(TextureSampleType::default())), (10, texture_2d(TextureSampleType::default())),
        (12, sampler(SamplerBindingType::Filtering)),
    )));
    let cloud = BindGroupLayoutDescriptor::new("cloud_data", &BindGroupLayoutEntries::sequential(S::FRAGMENT, (
        uniform_buffer::<CloudGpu>(false), texture_3d(TextureSampleType::default()), texture_3d(TextureSampleType::default()),
        sampler(SamplerBindingType::Filtering), texture_2d(TextureSampleType::default()), sampler(SamplerBindingType::Filtering),
        uniform_buffer::<StormGpu>(false), uniform_buffer::<FlashGpu>(false), texture_2d(TextureSampleType::default()), sampler(SamplerBindingType::Filtering),
        texture_2d(TextureSampleType::Float { filterable: false }),
    )));
    let composite = BindGroupLayoutDescriptor::new("cloud_composite", &BindGroupLayoutEntries::sequential(S::FRAGMENT, (
        texture_2d(TextureSampleType::default()), sampler(SamplerBindingType::Filtering), texture_2d(TextureSampleType::default()),
        texture_2d(TextureSampleType::Float { filterable: false }), uniform_buffer::<CloudGpu>(false),
    )));
    let resolve = BindGroupLayoutDescriptor::new("cloud_resolve", &BindGroupLayoutEntries::sequential(S::FRAGMENT, (
        uniform_buffer::<ResolveGpu>(false), texture_2d(TextureSampleType::default()),
        texture_2d(TextureSampleType::Float { filterable: false }), texture_2d(TextureSampleType::default()), sampler(SamplerBindingType::Filtering),
        texture_2d(TextureSampleType::Float { filterable: false }),
    )));
    let shaft_layout = BindGroupLayoutDescriptor::new("cloud_shaft", &BindGroupLayoutEntries::sequential(S::FRAGMENT, (
        uniform_buffer::<ShaftGpu>(false), texture_2d(TextureSampleType::Float { filterable: false }),
    )));
    let shaft_comp = BindGroupLayoutDescriptor::new("cloud_shaft_comp", &BindGroupLayoutEntries::sequential(S::FRAGMENT, (
        texture_2d(TextureSampleType::default()), texture_2d(TextureSampleType::Float { filterable: false }), sampler(SamplerBindingType::Filtering),
    )));
    let shadow_layout = BindGroupLayoutDescriptor::new("cloud_shadow", &BindGroupLayoutEntries::sequential(S::FRAGMENT, (uniform_buffer::<ShadowGpu>(false),)));
    let shadow_pipeline = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("cloud_shadow".into()),
        layout: vec![cloud.clone(), shadow_layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState { shader: assets.load(SHADOW), targets: vec![Some(ColorTargetState { format: TextureFormat::R8Unorm, blend: None, write_mask: ColorWrites::ALL })], ..default() }),
        ..default()
    });
    let target = |format| Some(ColorTargetState { format, blend: None, write_mask: ColorWrites::ALL });
    let march = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("cloud_march".into()),
        layout: vec![atmo.clone(), cloud.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: assets.load(MARCH), entry_point: Some("march".into()),
            targets: vec![target(TextureFormat::Rgba16Float), target(TextureFormat::R32Float), target(TextureFormat::Rgba16Float)], ..default()
        }),
        ..default()
    });
    let resolve_pipeline = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("cloud_resolve".into()),
        layout: vec![resolve.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState { shader: assets.load(RESOLVE), targets: vec![target(TextureFormat::Rgba16Float), target(TextureFormat::R16Float)], ..default() }),
        ..default()
    });
    let shaft_march = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("cloud_shaft_march".into()),
        layout: vec![atmo.clone(), cloud.clone(), shaft_layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState { shader: assets.load(SHAFT), entry_point: Some("main".into()), targets: vec![target(TextureFormat::Rgba16Float), target(TextureFormat::R32Float)], ..default() }),
        ..default()
    });
    let shaft_resolve = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("cloud_shaft_resolve".into()),
        layout: vec![resolve.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState { shader: assets.load(SHAFT_RESOLVE), targets: vec![target(TextureFormat::Rgba16Float)], ..default() }),
        ..default()
    });
    let sampler = |mode| device.create_sampler(&SamplerDescriptor {
        address_mode_u: mode, address_mode_v: mode, address_mode_w: mode,
        mag_filter: FilterMode::Linear, min_filter: FilterMode::Linear, ..default()
    });
    if timing_on.0 && device.features().contains(WgpuFeatures::TIMESTAMP_QUERY) {
        let size = 12 * 8;
        let buf = |usage, label| device.wgpu_device().create_buffer(&BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
        commands.insert_resource(PassTiming {
            qs: device.wgpu_device().create_query_set(&wgpu::QuerySetDescriptor { label: Some("cloud_ts"), ty: wgpu::QueryType::Timestamp, count: 12 }),
            resolve: buf(BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC, "cloud_ts_resolve"), read: buf(BufferUsages::COPY_DST | BufferUsages::MAP_READ, "cloud_ts_read"),
            sums: [0.0; 6], n: 0, ran: default(),
        });
    }
    let renderer = commands.spawn(Name::new("CloudRenderer")).id();
    let shaft_renderer = commands.spawn(Name::new("CloudShaftRenderer")).id();
    commands.insert_resource(CloudRender {
        atmo, cloud, composite, resolve, march, shadow_layout, shadow_pipeline, shadow_ubo: UniformBuffer::default(), storm_ubo: UniformBuffer::default(), flash_ubo: UniformBuffer::default(), resolve_pipeline, composite_shader: assets.load(COMPOSITE), fullscreen: fullscreen.clone(),
        repeat: sampler(AddressMode::Repeat), clamp: sampler(AddressMode::ClampToEdge), ubo: UniformBuffer::default(), renderer,
        composite_pipelines: HashMap::new(),
        shaft_layout, shaft_comp, shaft_march, shaft_resolve, shaft_shader: assets.load(SHAFT_COMP), shaft_ubo: UniformBuffer::default(), shaft_renderer, shaft_pipelines: HashMap::new(),
    });
}

/// `cur`: premultiplied exposed cloud colour and opacity. `t`: cloud depth in metres (0 = no cloud), for the W2 resolve.
/// `flash`: the march's flash light, low res, read only by the composite.
/// `read`/`write`: the resolved history pair, swapped every frame; the composite shows `write`.
#[derive(Component)]
struct CloudTextures { cur: CachedTexture, t: CachedTexture, flash: CachedTexture, read: CachedTexture, write: CachedTexture, scur: CachedTexture, st: CachedTexture, sread: CachedTexture, swrite: CachedTexture, cread: CachedTexture, cwrite: CachedTexture }

/// Last frame's view, kept on the render-world view entity. A size change or a skipped frame resets the history.
#[derive(Component)]
struct CloudHistory { clip_from_view: Mat4, rot_inv: Mat4, cam: Vec3, size: UVec2, frame: u32, since: u32 }

#[derive(Component)]
struct CloudResolveUbo(UniformBuffer<ResolveGpu>);

#[derive(Component)]
struct CloudBindGroups { atmo: BindGroup, cloud: BindGroup, composite: BindGroup, resolve: BindGroup, shadow: Option<BindGroup>, shaft: Option<BindGroup>, shaft_resolve: BindGroup, shaft_comp: BindGroup }

fn prepare_textures_and_uniform(
    mut commands: Commands, params: Option<Res<CloudParams>>, render: Option<ResMut<CloudRender>>, frame: Res<FrameCount>,
    views: Query<(Entity, &ExtractedView, Option<&MainPassResolutionOverride>, Option<&CloudHistory>), With<ExtractedAtmosphere>>, device: Res<RenderDevice>, queue: Res<RenderQueue>, mut cache: ResMut<TextureCache>,
) {
    let (Some(params), Some(mut render)) = (params, render) else { return };
    render.shadow_ubo.set(params.shadow_gpu);
    render.shadow_ubo.write_buffer(&device, &queue);
    render.storm_ubo.set(params.storm);
    render.storm_ubo.write_buffer(&device, &queue);
    render.flash_ubo.set(params.flash);
    render.flash_ubo.write_buffer(&device, &queue);
    render.shaft_ubo.set(params.shaft);
    render.shaft_ubo.write_buffer(&device, &queue);
    // HZD update division from config clouds.update_div (default 4: unmarched pixels keep reprojected history, marched pixel is a running mean); 1 marches every pixel every frame (reference arm).
    let div = params.update_div;
    let (ox, oy) = match div { 1 => (0, 0), 2 => { let b = [0, 3, 1, 2][(frame.0 % 4) as usize]; (b % 2, b / 2) } _ => { let b = BAYER16[(frame.0 % 16) as usize]; (b % 4, b / 4) } };
    for (e, view, over, hist) in &views {
        let full = over.map_or(view.viewport.zw(), |o| o.0);
        let size = (full.as_vec2() * params.render_scale).ceil().as_uvec2().max(UVec2::ONE);
        let low = (size + UVec2::splat(div - 1)) / div;
        render.ubo.set(CloudGpu { frame: frame.0, march_ox: ox as f32, march_oy: oy as f32, full_w: size.x as f32, full_h: size.y as f32, march_div: div as f32, ..params.gpu });
        render.ubo.write_buffer(&device, &queue);
        let mut tex = |label, format, size: UVec2| cache.get(&device, TextureDescriptor {
            label: Some(label), size: Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: TextureDimension::D2, format, usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING, view_formats: &[],
        });
        let (a, b) = (tex("cloud_hist_a", TextureFormat::Rgba16Float, size), tex("cloud_hist_b", TextureFormat::Rgba16Float, size));
        let (read, write) = if frame.0 % 2 == 0 { (a, b) } else { (b, a) };
        let (ca, cb) = (tex("cloud_conf_a", TextureFormat::R16Float, size), tex("cloud_conf_b", TextureFormat::R16Float, size));
        let (cread, cwrite) = if frame.0 % 2 == 0 { (ca, cb) } else { (cb, ca) };
        let (sa, sb) = (tex("shaft_hist_a", TextureFormat::Rgba16Float, size), tex("shaft_hist_b", TextureFormat::Rgba16Float, size));
        let (sread, swrite) = if frame.0 % 2 == 0 { (sa, sb) } else { (sb, sa) };
        let rot = Mat4::from_mat3(Mat3::from(view.world_from_view.affine().matrix3));
        let cam = view.world_from_view.translation();
        // A cut (teleport, or a turn of more than resolve.cut_deg in one frame) leaves no usable history, so it resets like a resize.
        let cut = |h: &CloudHistory| (h.rot_inv.transpose().z_axis.truncate().dot(rot.z_axis.truncate())) < params.resolve.cut_deg.to_radians().cos() || cam.distance(h.cam) > params.resolve.cut_m;
        let reset = hist.is_none_or(|h| h.size != size || h.frame.wrapping_add(1) != frame.0 || cut(h));
        let mut ubo = UniformBuffer::from(ResolveGpu {
            prev_clip_from_ray: hist.map_or(view.clip_from_view * rot.transpose(), |h| h.clip_from_view * h.rot_inv), ray_from_clip: rot * view.clip_from_view.inverse(),
            delta: hist.map_or(Vec3::ZERO, |h| cam - h.cam), alpha: params.temporal_alpha, max_distance: params.gpu.max_distance, reset: reset as u32, ox, oy, div, n: if reset { 0 } else { hist.map_or(0, |h| h.since + 1) },
            noreproj: params.resolve.no_reproject as u32,
            reject_gain: params.resolve.reject_gain, still_px: params.resolve.still_px, fresh: params.resolve.fresh, conf_max: params.resolve.conf_max, upsample_mean_frames: params.resolve.upsample_mean_frames,
            motion_half_px: params.resolve.motion_half_px, wide_upsample: params.resolve.wide_upsample as u32 as f32,
        });
        ubo.write_buffer(&device, &queue);
        commands.entity(e).insert((
            CloudTextures { cur: tex("cloud_cur", TextureFormat::Rgba16Float, low), t: tex("cloud_t", TextureFormat::R32Float, low), flash: tex("cloud_flash", TextureFormat::Rgba16Float, low), read, write,
                scur: tex("shaft_cur", TextureFormat::Rgba16Float, size), st: tex("shaft_t", TextureFormat::R32Float, size), sread, swrite, cread, cwrite },
            CloudHistory { clip_from_view: view.clip_from_view, rot_inv: rot.transpose(), cam, size, frame: frame.0, since: if reset { 0 } else { hist.map_or(0, |h| h.since + 1) } }, CloudResolveUbo(ubo),
        ));
    }
}

fn queue_composite(
    render: Option<ResMut<CloudRender>>, draw_functions: Res<DrawFunctions<Transparent3d>>, cache: Res<PipelineCache>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>, views: Query<(&ExtractedView, &Msaa, &ViewTarget), With<CloudBindGroups>>,
) {
    let Some(mut render) = render else { return };
    let draw_function = draw_functions.read().get_id::<(SetItemPipeline, DrawCloudComposite)>().unwrap();
    for (view, msaa, target) in &views {
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else { continue };
        if skipped("composite") { continue; }
        let key = (msaa.samples(), target.main_texture_format());
        let pipeline = match render.composite_pipelines.get(&key) {
            Some(p) => *p,
            None => {
                let p = cache.queue_render_pipeline(RenderPipelineDescriptor {
                    label: Some("cloud_composite".into()),
                    layout: vec![render.atmo.clone(), render.composite.clone()],
                    vertex: render.fullscreen.to_vertex_state(),
                    fragment: Some(FragmentState {
                        shader: render.composite_shader.clone(),
                        targets: vec![Some(ColorTargetState { format: key.1, blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: ColorWrites::ALL })],
                        ..default()
                    }),
                    // Reverse-Z: the shader writes the cloud front's depth (0, the far plane, with no cloud depth or over_terrain off, so only sky samples pass); nearer scene samples fail.
                    depth_stencil: Some(DepthStencilState {
                        format: CORE_3D_DEPTH_FORMAT, depth_write_enabled: Some(false), depth_compare: Some(CompareFunction::GreaterEqual),
                        stencil: default(), bias: default(),
                    }),
                    multisample: MultisampleState { count: key.0, ..default() },
                    ..default()
                });
                render.composite_pipelines.insert(key, p);
                p
            }
        };
        // AlwaysOnTop sorts at -inf: drawn before the glass and every other transparent item.
        phase.add_transient(Transparent3d {
            sorting_info: TransparentSortingInfo3d::AlwaysOnTop, distance: 0.0, entity: (render.renderer, MainEntity::from(render.renderer)),
            draw_function, pipeline, batch_range: 0..1, extra_index: PhaseItemExtraIndex::None, indexed: false,
        });
    }
}

/// The rain shafts: a second fullscreen item right after the cloud composite, depth-tested with its own fragment depth (clouds_shaft.wesl).
fn queue_shaft(
    render: Option<ResMut<CloudRender>>, draw_functions: Res<DrawFunctions<Transparent3d>>, cache: Res<PipelineCache>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>, views: Query<(&ExtractedView, &Msaa, &ViewTarget), With<CloudBindGroups>>, params: Option<Res<CloudParams>>,
) {
    let Some(mut render) = render else { return };
    let shafts_on = params.is_some_and(|p| p.shaft.scale > 0.0);
    let draw_function = draw_functions.read().get_id::<(SetItemPipeline, DrawCloudShaft)>().unwrap();
    for (view, msaa, target) in &views {
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else { continue };
        if !shafts_on { continue; }
        let key = (msaa.samples(), target.main_texture_format());
        let pipeline = match render.shaft_pipelines.get(&key) {
            Some(p) => *p,
            None => {
                let p = cache.queue_render_pipeline(RenderPipelineDescriptor {
                    label: Some("cloud_shaft".into()),
                    layout: vec![render.atmo.clone(), render.shaft_comp.clone()],
                    vertex: render.fullscreen.to_vertex_state(),
                    fragment: Some(FragmentState {
                        shader: render.shaft_shader.clone(),
                        targets: vec![Some(ColorTargetState { format: key.1, blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: ColorWrites::ALL })],
                        ..default()
                    }),
                    // Reverse-Z: the shader writes the depth of the first shaft sample; nearer scene samples fail.
                    depth_stencil: Some(DepthStencilState {
                        format: CORE_3D_DEPTH_FORMAT, depth_write_enabled: Some(false), depth_compare: Some(CompareFunction::GreaterEqual),
                        stencil: default(), bias: default(),
                    }),
                    multisample: MultisampleState { count: key.0, ..default() },
                    ..default()
                });
                render.shaft_pipelines.insert(key, p);
                p
            }
        };
        // Sorted far behind the glass but after the composite's -inf: the bias keeps it first among the ordinary items.
        phase.add_transient(Transparent3d {
            sorting_info: TransparentSortingInfo3d::Sorted { mesh_center: Vec3::ZERO, depth_bias: -1.0e9 }, distance: -1.0e9, entity: (render.shaft_renderer, MainEntity::from(render.shaft_renderer)),
            draw_function, pipeline, batch_range: 0..1, extra_index: PhaseItemExtraIndex::None, indexed: false,
        });
    }
}

fn prepare_bind_groups(
    mut commands: Commands, params: Option<Res<CloudParams>>, render: Option<Res<CloudRender>>, cache: Res<PipelineCache>, device: Res<RenderDevice>,
    images: Res<RenderAssets<GpuImage>>, atmo_sampler: Res<AtmosphereSampler>, view_uniforms: Res<ViewUniforms>, transforms: Res<AtmosphereTransforms>,
    atmo_uniforms: Res<ComponentUniforms<GpuAtmosphere>>, settings_uniforms: Res<ComponentUniforms<GpuAtmosphereSettings>>, gpu_media: Res<RenderAssets<GpuScatteringMedium>>, medium_sampler: Res<ScatteringMediumSampler>,
    views: Query<(Entity, &ExtractedAtmosphere, &AtmosphereTextures, &CloudTextures, &CloudResolveUbo)>,
) {
    let (Some(params), Some(render)) = (params, render) else { return };
    let (Some(base), Some(detail), Some(map), Some(ground), Some(oro)) = (images.get(&params.base), images.get(&params.detail), images.get(&params.map), images.get(&params.ground), images.get(&params.oro)) else { return };
    let (Some(a), Some(s), Some(t), Some(v), Some(u), Some(storms), Some(flash)) = (atmo_uniforms.binding(), settings_uniforms.binding(), transforms.uniforms().binding(), view_uniforms.uniforms.binding(), render.ubo.binding(), render.storm_ubo.binding(), render.flash_ubo.binding()) else { return };
    for (e, extracted, atmo, tex, ubo) in &views {
        let Some(medium) = gpu_media.get(extracted.medium) else { continue };
        let atmo_bg = device.create_bind_group("cloud_atmosphere", &cache.get_bind_group_layout(&render.atmo), &BindGroupEntries::with_indices((
            (0, a.clone()), (1, s.clone()), (2, t.clone()), (3, v.clone()), (5, &medium.density_lut_view), (7, medium_sampler.sampler()),
            (8, &atmo.transmittance_lut.default_view), (10, &atmo.sky_view_lut.default_view), (12, &**atmo_sampler),
        )));
        let cloud = device.create_bind_group("cloud_data", &cache.get_bind_group_layout(&render.cloud), &BindGroupEntries::sequential((
            u.clone(), &base.texture_view, &detail.texture_view, &render.repeat, &map.texture_view, if params.map_wrap { &render.repeat } else { &render.clamp }, storms.clone(), flash.clone(),
            &oro.texture_view, &render.clamp, &ground.texture_view,
        )));
        let composite = device.create_bind_group("cloud_composite", &cache.get_bind_group_layout(&render.composite), &BindGroupEntries::sequential((
            &tex.write.default_view, &render.clamp, &tex.flash.default_view, &tex.t.default_view, u.clone(),
        )));
        let Some(ru) = ubo.0.binding() else { continue };
        let resolve = device.create_bind_group("cloud_resolve", &cache.get_bind_group_layout(&render.resolve), &BindGroupEntries::sequential((
            ru, &tex.cur.default_view, &tex.t.default_view, &tex.read.default_view, &render.clamp, &tex.cread.default_view,
        )));
        let shadow = render.shadow_ubo.binding().map(|b| device.create_bind_group("cloud_shadow", &cache.get_bind_group_layout(&render.shadow_layout), &BindGroupEntries::sequential((b,))));
        let shaft = render.shaft_ubo.binding().map(|b| device.create_bind_group("cloud_shaft", &cache.get_bind_group_layout(&render.shaft_layout), &BindGroupEntries::sequential((b, &ground.texture_view))));
        let shaft_resolve = device.create_bind_group("cloud_shaft_resolve", &cache.get_bind_group_layout(&render.resolve), &BindGroupEntries::sequential((
            ubo.0.binding().unwrap(), &tex.scur.default_view, &tex.st.default_view, &tex.sread.default_view, &render.clamp, &tex.cread.default_view,
        )));
        let shaft_comp = device.create_bind_group("cloud_shaft_comp", &cache.get_bind_group_layout(&render.shaft_comp), &BindGroupEntries::sequential((
            &tex.swrite.default_view, &tex.st.default_view, &render.clamp,
        )));
        commands.entity(e).insert(CloudBindGroups { atmo: atmo_bg, cloud, composite, resolve, shadow, shaft, shaft_resolve, shaft_comp });
    }
}

/// Cleanup (after submit): map the resolved timestamps and print the running mean every 60 frames.
fn read_timing(timing: Option<ResMut<PassTiming>>, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    let Some(mut t) = timing else { return };
    let slice = t.read.slice(..);
    slice.map_async(MapMode::Read, |_| {});
    let _ = device.poll(PollType::wait_indefinitely());
    let ts: Vec<u64> = slice.get_mapped_range().expect("timestamps").chunks(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect();
    t.read.unmap();
    let period = queue.get_timestamp_period() as f64 * 1e-6;
    let ran = t.ran.swap(0, std::sync::atomic::Ordering::Relaxed);
    for i in 0..6 { if ts[2 * i + 1] >= ts[2 * i] && ts[2 * i] > 0 && ran & (1 << i) != 0 { t.sums[i] += (ts[2 * i + 1] - ts[2 * i]) as f64 * period; } }
    t.n += 1;
    if t.n == 60 {
        println!("PASSMS {}", PASSES.iter().zip(t.sums).map(|(n, s)| format!("{n} {:.3}", s / 60.0)).collect::<Vec<_>>().join(" | "));
        (t.sums, t.n) = ([0.0; 6], 0);
    }
}

/// Pass skipping was an env switch for cost tables; not carried over.
fn skipped(_pass: &str) -> bool { false }

fn clouds_march(
    view: ViewQuery<(&CloudTextures, &CloudBindGroups, &DynamicUniformIndex<GpuAtmosphere>, &DynamicUniformIndex<GpuAtmosphereSettings>, &AtmosphereTransformsOffset, &ViewUniformOffset)>,
    render: Option<Res<CloudRender>>, params: Option<Res<CloudParams>>, images: Res<RenderAssets<GpuImage>>, cache: Res<PipelineCache>, mut ctx: RenderContext,
    drawn: Local<std::sync::atomic::AtomicU32>, frame: Res<FrameCount>, timing: Option<Res<PassTiming>>,
) {
    let Some(render) = render else { return };
    let Some(pipeline) = cache.get_render_pipeline(render.march) else { return };
    let ts = |i: u32| timing.as_ref().map(|t| wgpu::RenderPassTimestampWrites { query_set: &t.qs, beginning_of_pass_write_index: Some(2 * i), end_of_pass_write_index: Some(2 * i + 1) });
    let (tex, bg, atmo_i, settings_i, transforms_i, view_i) = view.into_inner();
    // Cloud shadow cookie: once per frame (the first view), every `shadow_every` frames.
    if let (Some(params), Some(sp), Some(sbg)) = (&params, cache.get_render_pipeline(render.shadow_pipeline), &bg.shadow) {
        if let (true, Some(img)) = (!skipped("shadow"), images.get(&params.shadow)) {
            if drawn.swap(frame.0 + 1, std::sync::atomic::Ordering::Relaxed) != frame.0 + 1 {
                let (band, bands) = (params.shadow_band.0, params.shadow_band.1.max(1));
                let load = if bands > 1 { LoadOp::Load } else { LoadOp::Clear(default()) };
                if let Some(t) = &timing { t.ran.fetch_or(1 << 0, std::sync::atomic::Ordering::Relaxed); }
                let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
                    label: Some("cloud_shadow"),
                    color_attachments: &[Some(RenderPassColorAttachment { view: &img.texture_view, depth_slice: None, resolve_target: None, ops: Operations { load, store: StoreOp::Store } })],
                    depth_stencil_attachment: None, timestamp_writes: ts(0), occlusion_query_set: None, multiview_mask: None,
                });
                if bands > 1 {
                    let h = img.texture.height();
                    pass.set_scissor_rect(0, h * band / bands, img.texture.width(), h * (band + 1) / bands - h * band / bands);
                }
                pass.set_pipeline(sp);
                pass.set_bind_group(0, &bg.cloud, &[]);
                pass.set_bind_group(1, sbg, &[]);
                pass.draw(0..3, 0..1);
            }
        }
    }
    fn attachment(t: &CachedTexture) -> Option<RenderPassColorAttachment<'_>> {
        Some(RenderPassColorAttachment { view: &t.default_view, depth_slice: None, resolve_target: None, ops: Operations::default() })
    }
    if skipped("march") { return; }
    if let Some(t) = &timing { t.ran.fetch_or(1 << 1, std::sync::atomic::Ordering::Relaxed); }
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("cloud_march"), color_attachments: &[attachment(&tex.cur), attachment(&tex.t), attachment(&tex.flash)],
        depth_stencil_attachment: None, timestamp_writes: ts(1), occlusion_query_set: None, multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &bg.atmo, &[atmo_i.index(), settings_i.index(), transforms_i.index(), view_i.offset]);
    pass.set_bind_group(1, &bg.cloud, &[]);
    pass.draw(0..3, 0..1);
    drop(pass);

    let Some(pipeline) = cache.get_render_pipeline(render.resolve_pipeline) else { return };
    if skipped("resolve") { return; }
    if let Some(t) = &timing { t.ran.fetch_or(1 << 2, std::sync::atomic::Ordering::Relaxed); }
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("cloud_resolve"), color_attachments: &[attachment(&tex.write), attachment(&tex.cwrite)],
        depth_stencil_attachment: None, timestamp_writes: ts(2), occlusion_query_set: None, multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &bg.resolve, &[]);
    pass.draw(0..3, 0..1);
    drop(pass);
    // Rain shafts: half-res march + temporal resolve, drawn later by queue_shaft's item (clouds_shaft_comp.wesl).
    if let (Some(params), Some(sh), Some(smarch), Some(sres)) = (&params, &bg.shaft, cache.get_render_pipeline(render.shaft_march), cache.get_render_pipeline(render.shaft_resolve)) {
        if params.shaft.scale > 0.0 && !skipped("shaft") {
            if let Some(t) = &timing { t.ran.fetch_or(1 << 3, std::sync::atomic::Ordering::Relaxed); }
            let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
                label: Some("cloud_shaft_march"), color_attachments: &[attachment(&tex.scur), attachment(&tex.st)],
                depth_stencil_attachment: None, timestamp_writes: ts(3), occlusion_query_set: None, multiview_mask: None,
            });
            pass.set_pipeline(smarch);
            pass.set_bind_group(0, &bg.atmo, &[atmo_i.index(), settings_i.index(), transforms_i.index(), view_i.offset]);
            pass.set_bind_group(1, &bg.cloud, &[]);
            pass.set_bind_group(2, sh, &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            if let Some(t) = &timing { t.ran.fetch_or(1 << 4, std::sync::atomic::Ordering::Relaxed); }
            let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
                label: Some("cloud_shaft_resolve"), color_attachments: &[attachment(&tex.swrite)],
                depth_stencil_attachment: None, timestamp_writes: ts(4), occlusion_query_set: None, multiview_mask: None,
            });
            pass.set_pipeline(sres);
            pass.set_bind_group(0, &bg.shaft_resolve, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

/// Timing span "transparent" begins: an empty pass on the view target, timestamp 10 at its end, so the span is the whole main
/// transparent pass (cloud composite, shaft composite and rain FX quads); rain FX cost is that span with rain minus without (tests/cost.rs).
fn transparent_begin(view: ViewQuery<&ViewTarget>, timing: Option<Res<PassTiming>>, mut ctx: RenderContext) {
    let Some(t) = &timing else { return };
    let target = view.into_inner();
    drop(ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("cloud_ts_transparent"), color_attachments: &[Some(target.get_color_attachment())], depth_stencil_attachment: None,
        timestamp_writes: Some(wgpu::RenderPassTimestampWrites { query_set: &t.qs, beginning_of_pass_write_index: None, end_of_pass_write_index: Some(10) }),
        occlusion_query_set: None, multiview_mask: None,
    }));
}

/// Timing span "transparent" ends: an empty pass on the view target, timestamp 11 at its start, then the timestamp queries are resolved
/// (all 12, so this frame's transparent pair is read). The span is the whole main transparent pass; see transparent_begin.
fn transparent_end(view: ViewQuery<&ViewTarget>, timing: Option<Res<PassTiming>>, mut ctx: RenderContext) {
    let Some(t) = &timing else { return };
    let target = view.into_inner();
    t.ran.fetch_or(1 << 5, std::sync::atomic::Ordering::Relaxed);
    drop(ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("cloud_ts_transparent"), color_attachments: &[Some(target.get_color_attachment())], depth_stencil_attachment: None,
        timestamp_writes: Some(wgpu::RenderPassTimestampWrites { query_set: &t.qs, beginning_of_pass_write_index: Some(11), end_of_pass_write_index: None }),
        occlusion_query_set: None, multiview_mask: None,
    }));
    let e = ctx.command_encoder();
    e.resolve_query_set(&t.qs, 0..12, &t.resolve, 0);
    e.copy_buffer_to_buffer(&t.resolve, 0, &t.read, 0, 96);
}

struct DrawCloudComposite;
impl<P: PhaseItem> RenderCommand<P> for DrawCloudComposite {
    type Param = ();
    type ViewQuery = (Read<CloudBindGroups>, Read<DynamicUniformIndex<GpuAtmosphere>>, Read<DynamicUniformIndex<GpuAtmosphereSettings>>, Read<AtmosphereTransformsOffset>, Read<ViewUniformOffset>);
    type ItemQuery = ();
    fn render<'w>(_: &P, (bg, atmo_i, settings_i, transforms_i, view_i): ROQueryItem<'w, '_, Self::ViewQuery>, _: Option<ROQueryItem<'w, '_, Self::ItemQuery>>, _: SystemParamItem<'w, '_, Self::Param>, pass: &mut TrackedRenderPass<'w>) -> RenderCommandResult {
        pass.set_bind_group(0, &bg.atmo, &[atmo_i.index(), settings_i.index(), transforms_i.index(), view_i.offset]);
        pass.set_bind_group(1, &bg.composite, &[]);
        pass.draw(0..3, 0..1);
        RenderCommandResult::Success
    }
}

struct DrawCloudShaft;
impl<P: PhaseItem> RenderCommand<P> for DrawCloudShaft {
    type Param = ();
    type ViewQuery = (Read<CloudBindGroups>, Read<DynamicUniformIndex<GpuAtmosphere>>, Read<DynamicUniformIndex<GpuAtmosphereSettings>>, Read<AtmosphereTransformsOffset>, Read<ViewUniformOffset>);
    type ItemQuery = ();
    fn render<'w>(_: &P, (bg, atmo_i, settings_i, transforms_i, view_i): ROQueryItem<'w, '_, Self::ViewQuery>, _: Option<ROQueryItem<'w, '_, Self::ItemQuery>>, _: SystemParamItem<'w, '_, Self::Param>, pass: &mut TrackedRenderPass<'w>) -> RenderCommandResult {
        pass.set_bind_group(0, &bg.atmo, &[atmo_i.index(), settings_i.index(), transforms_i.index(), view_i.offset]);
        pass.set_bind_group(1, &bg.shaft_comp, &[]);
        pass.draw(0..3, 0..1);
        RenderCommandResult::Success
    }
}
