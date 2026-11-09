//! Ground fog, render world: `fog_pass` runs after the main pass (where Bevy's volumetric fog runs): `march_low` marches at
//! `ground_fog.render_scale` into a premultiplied fog target and a surface-distance target, reading the scene depth (sample 0 under MSAA)
//! to stop at the first surface; `resolve` blends it into last frame's reprojected fog at the same resolution (ground_fog.temporal_alpha),
//! and `composite` blends the resolved fog over the view target with a depth-aware upsample.
use std::collections::HashMap;
use bevy::{
    camera::MainPassResolutionOverride,
    diagnostic::FrameCount,
    core_pipeline::{FullscreenShader, schedule::Core3d, Core3dSystems},
    pbr::{ExtractedAtmosphere, GpuAtmosphereSettings, resources::{AtmosphereSampler, AtmosphereTextures, AtmosphereTransform, AtmosphereTransforms, AtmosphereTransformsOffset, GpuAtmosphere}},
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_component::{ComponentUniforms, DynamicUniformIndex},
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        render_resource::{binding_types::{sampler, texture_2d, texture_3d, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer}, *},
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        texture::{CachedTexture, FallbackImageZero, GpuImage, TextureCache},
        view::{ExtractedView, Msaa, ViewDepthStencilTexture, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms},
    },
    shader::Shader,
};

const FOG: &str = "embedded://nephthene/shaders/fog.wesl";

/// Uniform for fog.wesl; field order identical in the shader (`struct Fog`). Values from config ground_fog and nephthene_core::fog.
#[derive(Clone, Copy, Default, ShaderType)]
pub struct FogGpu {
    pub sun_dir: Vec3, pub sigma: f32,
    pub sun_color: Vec3, pub sigma_shallow: f32,
    pub moon_dir: Vec3, pub albedo: f32,
    pub moon_color: Vec3, pub g: f32,
    /// Fog map west x / north z, texel size (m), texels per side.
    pub map_min: Vec2, pub map_cell: f32, pub map_n: u32,
    /// Ground height grid (nephthene_core::rain::HeightGrid).
    pub g_min: Vec2, pub g_cell: f32, pub g_w: u32,
    pub g_h: u32, pub steps: u32, pub frame: u32, pub max_m: f32,
    /// Altitude band holding fog (the march is clipped to it).
    pub y_lo: f32, pub y_hi: f32, pub top_soft: f32, pub edge_m: f32,
    pub top_wave: f32, pub wave_tile: f32, pub shallow_e: f32, pub lift_max: f32,
    pub lift_soft: f32, pub breakup: f32, pub breakup_tile: f32, pub breakup_y: f32,
    pub wind: Vec2, pub sun_soft: f32, pub ambient_scale: f32,
    pub ambient_od: f32, pub ms_a: f32, pub ms_b: f32, pub ms_octaves: u32,
    /// Cloud shadow cookie frame (clouds' ShadowGpu); sh_on 0 = no clouds.
    pub sh_origin: Vec3, pub sh_half: f32,
    pub sh_right: Vec3, pub sh_on: f32,
    pub sh_up: Vec3, pub edge_fade: f32,
    /// smoothstep(0, lift_full_deg, sun elevation): the burn-off lift where a texel's top is sunlit (nephthene_core::fog::lift_m).
    pub sun_k: f32,
    /// Ground albedo under the fog (sky light reflected back up into it).
    pub ground_albedo: f32,
    pub wisp_m: f32, pub wisp_erode: f32, pub diffuse_sun: f32, pub step_power: f32,
    /// Terrain sun occlusion of the aerial in-scatter (config sky.sun_occlusion; 0 = off or no terrain), kept share, samples per ray.
    pub occ_on: f32, pub occ_keep: f32, pub occ_steps: u32,
    pub top_fade: f32,
    /// Cloud cover at the eye (0 = clear, no clouds, or ground_fog.ignore_cloud_sky) and ground_fog.cloud_diffuse.
    pub cloud_cover: f32, pub cloud_diffuse: f32,
    pub top_billow: f32, pub billow_tile: f32,
    /// Air phase for the kept share of shadowed in-scatter: Mie asymmetry and scattering (config sky), Rayleigh scattering (per metre).
    pub mie_g: f32, pub mie_beta: f32, pub ray_beta: Vec3,
    pub hash_noise: f32,
    /// Bases of the f16 fog map and ground texture heights.
    pub map_y0: f32, pub g_y0: f32,
    /// Aerial occlusion terms (sky.sun_occlusion): occ_terrain 1 = terrain shadows the air; occ_cloud 1 = the cloud cookie shadows the air (sky.cloud_shafts); occ_cloud_y = clouds.bottom_m, above which the cookie term is skipped; occ_cloud_mask 1 = weight the correction by the share the clouds leave uncovered (0 = sky.cloud_shafts_ignore_clouds, red arm).
    pub occ_terrain: f32, pub occ_cloud: f32, pub occ_cloud_y: f32, pub occ_cloud_mask: f32,
    /// sky.cloud_shafts_unbounded: 1 = the correction ignores what the pixel holds (red arm), 0 = bounded by the scene colour.
    pub occ_unbounded: f32,
}

/// Uniform for fog.wesl's `resolve`; field order identical in the shader (`struct FogResolve`). Last frame's view, as the clouds' ResolveGpu.
#[derive(Clone, Copy, Default, ShaderType)]
pub struct FogResolveGpu {
    /// Camera-relative direction (world axes) -> last frame's clip, and this frame's clip -> camera-relative direction.
    pub prev_clip_from_ray: Mat4, pub ray_from_clip: Mat4,
    /// This frame's camera position minus last frame's (m).
    pub delta: Vec3,
    /// ground_fog.temporal_alpha (1 = no history), max_m (reprojection distance of sky texels), 1 = reset, ground_fog.still_px.
    pub alpha: f32, pub max_m: f32, pub reset: u32, pub still_px: f32,
}

/// Main world -> render world, every frame: the uniform, the fog map (Rgba32Float), the ground heights (R32Float), the cloud cookie (R8).
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub struct FogParams { pub gpu: FogGpu, pub map: Handle<Image>, pub ground: Handle<Image>, pub cookie: Handle<Image>, pub noise: Handle<Image>, pub render_scale: f32, pub temporal_alpha: f32, pub still_px: f32,
    /// History reset on a camera cut (config resolve.cut_deg, resolve.cut_m).
    pub cut_deg: f32, pub cut_m: f32,
    /// clouds.pass_timing: GPU timestamps for the three passes (see FogTiming).
    pub timing: bool }

pub fn plugin(app: &mut App) {
    app.add_plugins(ExtractResourcePlugin::<FogParams>::default());
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app
        .add_systems(RenderStartup, init)
        .add_systems(Render, (read_timing.in_set(RenderSystems::Cleanup), prepare.in_set(RenderSystems::PrepareResources)))
        .add_systems(Core3d, fog_pass.after(Core3dSystems::MainPass).before(Core3dSystems::EarlyPostProcess))
        .add_systems(Core3d, copy_opaque.in_set(Core3dSystems::MainPass).after(bevy::core_pipeline::core_3d::main_opaque_pass_3d).before(bevy::core_pipeline::core_3d::main_transparent_pass_3d));
}

/// clouds.pass_timing: GPU timestamps for `ground_fog_march`, `ground_fog_resolve` and `ground_fog_composite`. Prints `FOGMS` every 60 frames (mean ms).
/// Created in `prepare` (the switch arrives with FogParams, after startup). Off or no TIMESTAMP_QUERY: no query set, no readback.
#[derive(Resource)]
pub(crate) struct FogTiming {
    qs: wgpu::QuerySet, resolve: wgpu::Buffer, read: wgpu::Buffer, sums: [f64; 3], n: u32,
    /// Set by fog_pass when it resolved this frame's timestamps; read_timing reads only then.
    recorded: bool,
}

#[derive(Resource)]
pub(crate) struct FogRender {
    atmo: BindGroupLayoutDescriptor, data: [BindGroupLayoutDescriptor; 2], comp: BindGroupLayoutDescriptor, resolve_layout: BindGroupLayoutDescriptor,
    /// march_low's group 2: the scene colour at binding 5 (fog.wesl `scene`; bindings 0..4 of group 2 are the resolve's).
    scene_layout: BindGroupLayoutDescriptor, shader: Handle<Shader>, fullscreen: FullscreenShader,
    smp: Sampler, repeat: Sampler, nearest: Sampler, ubo: UniformBuffer<FogGpu>,
    /// March pipeline per MSAA, resolve pipeline per MSAA, composite pipeline per (MSAA, target format).
    march: HashMap<bool, CachedRenderPipelineId>, resolve: HashMap<bool, CachedRenderPipelineId>, composite: HashMap<(bool, TextureFormat), CachedRenderPipelineId>,
}

/// The view's pipelines, its half-resolution march targets (premultiplied fog, surface distance) and the resolved history pair
/// (`read`/`write`, swapped every frame; the composite shows `write`) with the resolve uniform.
#[derive(Component)]
pub(crate) struct FogView { march: CachedRenderPipelineId, resolve: CachedRenderPipelineId, composite: CachedRenderPipelineId,
    /// The scene after the opaque pass, before transparents (copy_opaque): with the final scene it gives what the transparents let through.
    opaque: CachedTexture, low: CachedTexture, t: CachedTexture, read: CachedTexture, write: CachedTexture, ubo: UniformBuffer<FogResolveGpu> }

/// Last frame's view, kept on the render-world view entity. A size change, a skipped frame, a cut or a frame the fog pass skipped resets the history.
#[derive(Component)]
struct FogHistory { clip_from_view: Mat4, rot_inv: Mat4, cam: Vec3, size: UVec2, frame: u32, drawn: bool }

fn init(mut commands: Commands, device: Res<RenderDevice>, assets: Res<AssetServer>, fullscreen: Res<FullscreenShader>) {
    use ShaderStages as S;
    // Same indices as Bevy's render_sky layout (and the clouds'), so the shader imports bevy_pbr::atmosphere::bindings unchanged.
    let atmo = BindGroupLayoutDescriptor::new("fog_atmosphere", &BindGroupLayoutEntries::with_indices(S::FRAGMENT, (
        (0, uniform_buffer::<GpuAtmosphere>(true)), (1, uniform_buffer::<GpuAtmosphereSettings>(true)),
        (2, uniform_buffer::<AtmosphereTransform>(true)), (3, uniform_buffer::<ViewUniform>(true)),
        (8, texture_2d(TextureSampleType::default())), (10, texture_2d(TextureSampleType::default())),
        (11, texture_3d(TextureSampleType::default())), (12, sampler(SamplerBindingType::Filtering)),
    )));
    let unfiltered = TextureSampleType::Float { filterable: false };
    let data = |ms: bool| BindGroupLayoutDescriptor::new("fog_data", &BindGroupLayoutEntries::sequential(S::FRAGMENT, (
        uniform_buffer::<FogGpu>(false), texture_2d(TextureSampleType::default()), texture_2d(TextureSampleType::default()), texture_2d(TextureSampleType::default()),
        sampler(SamplerBindingType::Filtering), if ms { texture_depth_2d_multisampled() } else { texture_depth_2d() },
        texture_3d(TextureSampleType::default()), sampler(SamplerBindingType::Filtering),
        texture_2d(TextureSampleType::default()),
    )));
    let comp = BindGroupLayoutDescriptor::new("fog_composite", &BindGroupLayoutEntries::sequential(S::FRAGMENT, (texture_2d(TextureSampleType::default()), texture_2d(unfiltered), sampler(SamplerBindingType::NonFiltering))));
    let resolve_layout = BindGroupLayoutDescriptor::new("fog_resolve", &BindGroupLayoutEntries::sequential(S::FRAGMENT, (
        texture_2d(TextureSampleType::default()), texture_2d(unfiltered), sampler(SamplerBindingType::NonFiltering), texture_2d(TextureSampleType::default()), uniform_buffer::<FogResolveGpu>(false),
    )));
    let sampler = |mode| device.create_sampler(&SamplerDescriptor { address_mode_u: mode, address_mode_v: mode, address_mode_w: mode, mag_filter: FilterMode::Linear, min_filter: FilterMode::Linear, ..default() });
    commands.insert_resource(FogRender {
        atmo, data: [data(false), data(true)], comp, resolve_layout, scene_layout: BindGroupLayoutDescriptor::new("fog_scene", &BindGroupLayoutEntries::with_indices(S::FRAGMENT, ((5, texture_2d(TextureSampleType::default())), (6, texture_2d(TextureSampleType::default()))))), shader: assets.load(FOG), fullscreen: fullscreen.clone(), smp: sampler(AddressMode::ClampToEdge), repeat: sampler(AddressMode::Repeat), nearest: device.create_sampler(&SamplerDescriptor::default()),
        ubo: UniformBuffer::default(), march: HashMap::new(), resolve: HashMap::new(), composite: HashMap::new(),
    });
}

fn prepare(
    mut commands: Commands, params: Option<Res<FogParams>>, render: Option<ResMut<FogRender>>, cache: Res<PipelineCache>, device: Res<RenderDevice>, queue: Res<RenderQueue>, frame: Res<FrameCount>,
    mut textures: ResMut<TextureCache>, timing: Option<Res<FogTiming>>, views: Query<(Entity, &ExtractedView, &Msaa, &ViewTarget, Option<&MainPassResolutionOverride>, Option<&FogHistory>), With<ExtractedAtmosphere>>,
) {
    let (Some(params), Some(mut render)) = (params, render) else { return };
    if params.timing && timing.is_none() && device.features().contains(WgpuFeatures::TIMESTAMP_QUERY) {
        let buf = |usage, label| device.wgpu_device().create_buffer(&BufferDescriptor { label: Some(label), size: 6 * 8, usage, mapped_at_creation: false });
        commands.insert_resource(FogTiming {
            qs: device.wgpu_device().create_query_set(&wgpu::QuerySetDescriptor { label: Some("fog_ts"), ty: wgpu::QueryType::Timestamp, count: 6 }),
            resolve: buf(BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC, "fog_ts_resolve"), read: buf(BufferUsages::COPY_DST | BufferUsages::MAP_READ, "fog_ts_read"),
            sums: [0.0; 3], n: 0, recorded: false,
        });
    }
    render.ubo.set(params.gpu);
    render.ubo.write_buffer(&device, &queue);
    let render = &mut *render;
    for (e, view, msaa, target, over, hist) in &views {
        let (ms, format) = (*msaa != Msaa::Off, target.main_texture_format());
        let defs = if ms { vec!["MULTISAMPLED".into()] } else { vec![] };
        let march = *render.march.entry(ms).or_insert_with(|| cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("ground_fog_march".into()),
            layout: vec![render.atmo.clone(), render.data[ms as usize].clone(), render.scene_layout.clone()],
            vertex: render.fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: render.shader.clone(), entry_point: Some("march_low".into()), shader_defs: defs.clone(),
                targets: vec![Some(TextureFormat::Rgba16Float.into()), Some(TextureFormat::R32Float.into())], ..default()
            }),
            ..default()
        }));
        let resolve = *render.resolve.entry(ms).or_insert_with(|| cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("ground_fog_resolve".into()),
            layout: vec![render.atmo.clone(), render.data[ms as usize].clone(), render.resolve_layout.clone()],
            vertex: render.fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: render.shader.clone(), entry_point: Some("resolve".into()), shader_defs: defs.clone(),
                targets: vec![Some(TextureFormat::Rgba16Float.into())], ..default()
            }),
            ..default()
        }));
        let composite = *render.composite.entry((ms, format)).or_insert_with(|| cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("ground_fog_composite".into()),
            layout: vec![render.atmo.clone(), render.data[ms as usize].clone(), render.comp.clone()],
            vertex: render.fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: render.shader.clone(), entry_point: Some("composite".into()), shader_defs: defs,
                // Premultiplied over the scene; the target's alpha is kept.
                targets: vec![Some(ColorTargetState { format, write_mask: ColorWrites::ALL, blend: Some(BlendState {
                    color: BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::OneMinusSrcAlpha, operation: BlendOperation::Add },
                    alpha: BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
                }) })],
                ..default()
            }),
            ..default()
        }));
        let full = over.map_or(view.viewport.zw(), |o| o.0);
        let size = (full.as_vec2() * params.render_scale.clamp(0.1, 1.0)).ceil().as_uvec2().max(UVec2::ONE);
        let mut tex = |label, format| textures.get(&device, TextureDescriptor {
            label: Some(label), size: Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: TextureDimension::D2, format, usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING, view_formats: &[],
        });
        let (a, b) = (tex("fog_hist_a", TextureFormat::Rgba16Float), tex("fog_hist_b", TextureFormat::Rgba16Float));
        let (low, t) = (tex("fog_low", TextureFormat::Rgba16Float), tex("fog_low_t", TextureFormat::R32Float));
        drop(tex);
        let opaque = textures.get(&device, TextureDescriptor {
            label: Some("fog_opaque"), size: target.main_texture().size(), mip_level_count: 1, sample_count: 1, dimension: TextureDimension::D2,
            format, usage: TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING, view_formats: &[],
        });
        let (read, write) = if frame.0 % 2 == 0 { (a, b) } else { (b, a) };
        let rot = Mat4::from_mat3(Mat3::from(view.world_from_view.affine().matrix3));
        let cam = view.world_from_view.translation();
        // A cut (teleport, or a turn of more than resolve.cut_deg in one frame) leaves no usable history, so it resets like a resize.
        let cut = |h: &FogHistory| h.rot_inv.transpose().z_axis.truncate().dot(rot.z_axis.truncate()) < params.cut_deg.to_radians().cos() || cam.distance(h.cam) > params.cut_m;
        let reset = hist.is_none_or(|h| !h.drawn || h.size != size || h.frame.wrapping_add(1) != frame.0 || cut(h));
        let mut ubo = UniformBuffer::from(FogResolveGpu {
            prev_clip_from_ray: hist.map_or(view.clip_from_view * rot.transpose(), |h| h.clip_from_view * h.rot_inv), ray_from_clip: rot * view.clip_from_view.inverse(),
            delta: hist.map_or(Vec3::ZERO, |h| cam - h.cam), alpha: params.temporal_alpha, max_m: params.gpu.max_m, reset: reset as u32, still_px: params.still_px,
        });
        ubo.write_buffer(&device, &queue);
        // fog_pass skips the frame (and writes no history) when there is no fog band and no sun occlusion.
        let drawn = !(params.gpu.y_hi <= params.gpu.y_lo && params.gpu.occ_on <= 0.0);
        commands.entity(e).insert((
            FogView { march, resolve, composite, opaque, low, t, read, write, ubo },
            FogHistory { clip_from_view: view.clip_from_view, rot_inv: rot.transpose(), cam, size, frame: frame.0, drawn },
        ));
    }
}

pub(crate) fn fog_pass(
    view: ViewQuery<(&ViewTarget, &ViewDepthStencilTexture, &Msaa, &FogView, &AtmosphereTextures, &DynamicUniformIndex<GpuAtmosphere>, &DynamicUniformIndex<GpuAtmosphereSettings>, &AtmosphereTransformsOffset, &ViewUniformOffset, Option<&crate::clouds::render::CloudTextures>)>,
    render: Option<Res<FogRender>>, params: Option<Res<FogParams>>, images: Res<RenderAssets<GpuImage>>, cache: Res<PipelineCache>, fallback: Res<FallbackImageZero>,
    atmo_sampler: Res<AtmosphereSampler>, view_uniforms: Res<ViewUniforms>, transforms: Res<AtmosphereTransforms>,
    atmo_uniforms: Res<ComponentUniforms<GpuAtmosphere>>, settings_uniforms: Res<ComponentUniforms<GpuAtmosphereSettings>>, mut ctx: RenderContext,
    timing: Option<ResMut<FogTiming>>,
) {
    let (Some(render), Some(params)) = (render, params) else { return };
    if params.gpu.y_hi <= params.gpu.y_lo && params.gpu.occ_on <= 0.0 { return; }
    let mut timing = timing.filter(|_| params.timing);
    let (target, depth, msaa, fv, atmo, atmo_i, settings_i, transforms_i, view_i, clouds) = view.into_inner();
    let (Some(march), Some(resolve), Some(composite)) = (cache.get_render_pipeline(fv.march), cache.get_render_pipeline(fv.resolve), cache.get_render_pipeline(fv.composite)) else { return };
    let (Some(map), Some(ground), Some(cookie), Some(noise)) = (images.get(&params.map), images.get(&params.ground), images.get(&params.cookie), images.get(&params.noise)) else { return };
    let (Some(a), Some(s), Some(t), Some(v), Some(u)) = (atmo_uniforms.binding(), settings_uniforms.binding(), transforms.uniforms().binding(), view_uniforms.uniforms.binding(), render.ubo.binding()) else { return };
    let Some(ru) = fv.ubo.binding() else { return };
    let Some(depth_view) = depth.attachment.depth_stencil_views().depth_only_view() else { return };
    let device = ctx.render_device().clone();
    let atmo_bg = device.create_bind_group("fog_atmosphere", &cache.get_bind_group_layout(&render.atmo), &BindGroupEntries::with_indices((
        (0, a), (1, s), (2, t), (3, v), (8, &atmo.transmittance_lut.default_view), (10, &atmo.sky_view_lut.default_view), (11, &atmo.aerial_view_lut.default_view), (12, &**atmo_sampler),
    )));
    let data = device.create_bind_group("fog_data", &cache.get_bind_group_layout(&render.data[(*msaa != Msaa::Off) as usize]), &BindGroupEntries::sequential((
        u, &map.texture_view, &ground.texture_view, &cookie.texture_view, &render.smp, depth_view, &noise.texture_view, &render.repeat,
        clouds.map_or(&fallback.texture_view, |c| &c.write.default_view),
    )));
    let scene = device.create_bind_group("fog_scene", &cache.get_bind_group_layout(&render.scene_layout), &BindGroupEntries::with_indices(((5, target.main_texture_view()), (6, &fv.opaque.default_view))));
    let res = device.create_bind_group("fog_resolve", &cache.get_bind_group_layout(&render.resolve_layout), &BindGroupEntries::sequential((&fv.low.default_view, &fv.t.default_view, &render.nearest, &fv.read.default_view, ru)));
    let comp = device.create_bind_group("fog_composite", &cache.get_bind_group_layout(&render.comp), &BindGroupEntries::sequential((&fv.write.default_view, &fv.t.default_view, &render.nearest)));
    let offsets = [atmo_i.index(), settings_i.index(), transforms_i.index(), view_i.offset];
    fn attach(view: &TextureView) -> Option<RenderPassColorAttachment<'_>> { Some(RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: Operations { load: LoadOp::Load, store: StoreOp::Store } }) }
    // Pass i (0 march, 1 composite, 2 resolve) writes timestamps 2i (begin) and 2i + 1 (end).
    let ts = |i: u32| timing.as_ref().map(|t| wgpu::RenderPassTimestampWrites { query_set: &t.qs, beginning_of_pass_write_index: Some(2 * i), end_of_pass_write_index: Some(2 * i + 1) });
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("ground_fog_march"), color_attachments: &[attach(&fv.low.default_view), attach(&fv.t.default_view)],
        depth_stencil_attachment: None, timestamp_writes: ts(0), occlusion_query_set: None, multiview_mask: None,
    });
    pass.set_pipeline(march);
    pass.set_bind_group(0, &atmo_bg, &offsets);
    pass.set_bind_group(1, &data, &[]);
    pass.set_bind_group(2, &scene, &[]);
    pass.draw(0..3, 0..1);
    drop(pass);
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("ground_fog_resolve"), color_attachments: &[attach(&fv.write.default_view)],
        depth_stencil_attachment: None, timestamp_writes: ts(2), occlusion_query_set: None, multiview_mask: None,
    });
    pass.set_pipeline(resolve);
    pass.set_bind_group(0, &atmo_bg, &offsets);
    pass.set_bind_group(1, &data, &[]);
    pass.set_bind_group(2, &res, &[]);
    pass.draw(0..3, 0..1);
    drop(pass);
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("ground_fog_composite"), color_attachments: &[attach(target.main_texture_view())],
        depth_stencil_attachment: None, timestamp_writes: ts(1), occlusion_query_set: None, multiview_mask: None,
    });
    pass.set_pipeline(composite);
    pass.set_bind_group(0, &atmo_bg, &offsets);
    pass.set_bind_group(1, &data, &[]);
    pass.set_bind_group(2, &comp, &[]);
    pass.draw(0..3, 0..1);
    drop(pass);
    if let Some(t) = timing.as_mut() {
        let e = ctx.command_encoder();
        e.resolve_query_set(&t.qs, 0..6, &t.resolve, 0);
        e.copy_buffer_to_buffer(&t.resolve, 0, &t.read, 0, 48);
        t.recorded = true;
    }
}

/// After the opaque pass, before transparents: copy the scene, so march_low can tell what the transparents (glass) let through.
fn copy_opaque(view: ViewQuery<(&ViewTarget, &FogView)>, mut ctx: RenderContext) {
    let (target, fv) = view.into_inner();
    ctx.command_encoder().copy_texture_to_texture(target.main_texture().as_image_copy(), fv.opaque.texture.as_image_copy(), target.main_texture().size());
}

/// Cleanup (after submit): map the resolved timestamps and print the running mean every 60 frames. Skipped on frames fog_pass did not record.
fn read_timing(timing: Option<ResMut<FogTiming>>, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    let Some(mut t) = timing else { return };
    if !std::mem::take(&mut t.recorded) { return; }
    let slice = t.read.slice(..);
    slice.map_async(MapMode::Read, |_| {});
    let _ = device.poll(PollType::wait_indefinitely());
    let ts: Vec<u64> = slice.get_mapped_range().expect("timestamps").chunks(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect();
    t.read.unmap();
    let period = queue.get_timestamp_period() as f64 * 1e-6;
    for i in 0..3 { if ts[2 * i + 1] >= ts[2 * i] && ts[2 * i] > 0 { t.sums[i] += (ts[2 * i + 1] - ts[2 * i]) as f64 * period; } }
    t.n += 1;
    if t.n == 60 {
        // march = the low-resolution passes (march + resolve); the printed line keeps its format (gate G19 parses it).
        let (march, composite) = ((t.sums[0] + t.sums[2]) / 60.0, t.sums[1] / 60.0);
        println!("FOGMS march {march:.3} | composite {composite:.3} | total {:.3}", march + composite);
        (t.sums, t.n) = ([0.0; 3], 0);
    }
}
