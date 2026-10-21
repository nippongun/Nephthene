//! Ground fog, render world: `fog_pass` runs after the main pass (where Bevy's volumetric fog runs): `march_low` marches at
//! `ground_fog.render_scale` into a premultiplied fog target and a surface-distance target, reading the scene depth (sample 0 under MSAA)
//! to stop at the first surface; `composite` blends it over the view target with a depth-aware upsample.
use std::collections::HashMap;
use bevy::{
    camera::MainPassResolutionOverride,
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
        texture::{CachedTexture, GpuImage, TextureCache},
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
}

/// Main world -> render world, every frame: the uniform, the fog map (Rgba32Float), the ground heights (R32Float), the cloud cookie (R8).
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub struct FogParams { pub gpu: FogGpu, pub map: Handle<Image>, pub ground: Handle<Image>, pub cookie: Handle<Image>, pub noise: Handle<Image>, pub render_scale: f32, /// clouds.pass_timing: GPU timestamps for the two passes (see FogTiming).
    pub timing: bool }

pub fn plugin(app: &mut App) {
    app.add_plugins(ExtractResourcePlugin::<FogParams>::default());
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app
        .add_systems(RenderStartup, init)
        .add_systems(Render, (read_timing.in_set(RenderSystems::Cleanup), prepare.in_set(RenderSystems::PrepareResources)))
        .add_systems(Core3d, fog_pass.after(Core3dSystems::MainPass).before(Core3dSystems::EarlyPostProcess));
}

/// clouds.pass_timing: GPU timestamps for `ground_fog_march` and `ground_fog_composite`. Prints `FOGMS` every 60 frames (mean ms).
/// Created in `prepare` (the switch arrives with FogParams, after startup). Off or no TIMESTAMP_QUERY: no query set, no readback.
#[derive(Resource)]
struct FogTiming {
    qs: wgpu::QuerySet, resolve: wgpu::Buffer, read: wgpu::Buffer, sums: [f64; 2], n: u32,
    /// Set by fog_pass when it resolved this frame's timestamps; read_timing reads only then.
    recorded: bool,
}

#[derive(Resource)]
struct FogRender {
    atmo: BindGroupLayoutDescriptor, data: [BindGroupLayoutDescriptor; 2], comp: BindGroupLayoutDescriptor, shader: Handle<Shader>, fullscreen: FullscreenShader,
    smp: Sampler, repeat: Sampler, nearest: Sampler, ubo: UniformBuffer<FogGpu>,
    /// March pipeline per MSAA, composite pipeline per (MSAA, target format).
    march: HashMap<bool, CachedRenderPipelineId>, composite: HashMap<(bool, TextureFormat), CachedRenderPipelineId>,
}

/// The view's pipelines and its half-resolution march targets (premultiplied fog, surface distance).
#[derive(Component)]
struct FogView { march: CachedRenderPipelineId, composite: CachedRenderPipelineId, low: CachedTexture, t: CachedTexture }

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
    )));
    let comp = BindGroupLayoutDescriptor::new("fog_composite", &BindGroupLayoutEntries::sequential(S::FRAGMENT, (texture_2d(TextureSampleType::default()), texture_2d(unfiltered), sampler(SamplerBindingType::NonFiltering))));
    let sampler = |mode| device.create_sampler(&SamplerDescriptor { address_mode_u: mode, address_mode_v: mode, address_mode_w: mode, mag_filter: FilterMode::Linear, min_filter: FilterMode::Linear, ..default() });
    commands.insert_resource(FogRender {
        atmo, data: [data(false), data(true)], comp, shader: assets.load(FOG), fullscreen: fullscreen.clone(), smp: sampler(AddressMode::ClampToEdge), repeat: sampler(AddressMode::Repeat), nearest: device.create_sampler(&SamplerDescriptor::default()),
        ubo: UniformBuffer::default(), march: HashMap::new(), composite: HashMap::new(),
    });
}

fn prepare(
    mut commands: Commands, params: Option<Res<FogParams>>, render: Option<ResMut<FogRender>>, cache: Res<PipelineCache>, device: Res<RenderDevice>, queue: Res<RenderQueue>,
    mut textures: ResMut<TextureCache>, timing: Option<Res<FogTiming>>, views: Query<(Entity, &ExtractedView, &Msaa, &ViewTarget, Option<&MainPassResolutionOverride>), With<ExtractedAtmosphere>>,
) {
    let (Some(params), Some(mut render)) = (params, render) else { return };
    if params.timing && timing.is_none() && device.features().contains(WgpuFeatures::TIMESTAMP_QUERY) {
        let buf = |usage, label| device.wgpu_device().create_buffer(&BufferDescriptor { label: Some(label), size: 4 * 8, usage, mapped_at_creation: false });
        commands.insert_resource(FogTiming {
            qs: device.wgpu_device().create_query_set(&wgpu::QuerySetDescriptor { label: Some("fog_ts"), ty: wgpu::QueryType::Timestamp, count: 4 }),
            resolve: buf(BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC, "fog_ts_resolve"), read: buf(BufferUsages::COPY_DST | BufferUsages::MAP_READ, "fog_ts_read"),
            sums: [0.0; 2], n: 0, recorded: false,
        });
    }
    render.ubo.set(params.gpu);
    render.ubo.write_buffer(&device, &queue);
    let render = &mut *render;
    for (e, view, msaa, target, over) in &views {
        let (ms, format) = (*msaa != Msaa::Off, target.main_texture_format());
        let defs = if ms { vec!["MULTISAMPLED".into()] } else { vec![] };
        let march = *render.march.entry(ms).or_insert_with(|| cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("ground_fog_march".into()),
            layout: vec![render.atmo.clone(), render.data[ms as usize].clone()],
            vertex: render.fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: render.shader.clone(), entry_point: Some("march_low".into()), shader_defs: defs.clone(),
                targets: vec![Some(TextureFormat::Rgba16Float.into()), Some(TextureFormat::R32Float.into())], ..default()
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
        commands.entity(e).insert(FogView { march, composite, low: tex("fog_low", TextureFormat::Rgba16Float), t: tex("fog_low_t", TextureFormat::R32Float) });
    }
}

fn fog_pass(
    view: ViewQuery<(&ViewTarget, &ViewDepthStencilTexture, &Msaa, &FogView, &AtmosphereTextures, &DynamicUniformIndex<GpuAtmosphere>, &DynamicUniformIndex<GpuAtmosphereSettings>, &AtmosphereTransformsOffset, &ViewUniformOffset)>,
    render: Option<Res<FogRender>>, params: Option<Res<FogParams>>, images: Res<RenderAssets<GpuImage>>, cache: Res<PipelineCache>,
    atmo_sampler: Res<AtmosphereSampler>, view_uniforms: Res<ViewUniforms>, transforms: Res<AtmosphereTransforms>,
    atmo_uniforms: Res<ComponentUniforms<GpuAtmosphere>>, settings_uniforms: Res<ComponentUniforms<GpuAtmosphereSettings>>, mut ctx: RenderContext,
    timing: Option<ResMut<FogTiming>>,
) {
    let (Some(render), Some(params)) = (render, params) else { return };
    if params.gpu.y_hi <= params.gpu.y_lo && params.gpu.occ_on <= 0.0 { return; }
    let mut timing = timing.filter(|_| params.timing);
    let (target, depth, msaa, fv, atmo, atmo_i, settings_i, transforms_i, view_i) = view.into_inner();
    let (Some(march), Some(composite)) = (cache.get_render_pipeline(fv.march), cache.get_render_pipeline(fv.composite)) else { return };
    let (Some(map), Some(ground), Some(cookie), Some(noise)) = (images.get(&params.map), images.get(&params.ground), images.get(&params.cookie), images.get(&params.noise)) else { return };
    let (Some(a), Some(s), Some(t), Some(v), Some(u)) = (atmo_uniforms.binding(), settings_uniforms.binding(), transforms.uniforms().binding(), view_uniforms.uniforms.binding(), render.ubo.binding()) else { return };
    let Some(depth_view) = depth.attachment.depth_stencil_views().depth_only_view() else { return };
    let device = ctx.render_device().clone();
    let atmo_bg = device.create_bind_group("fog_atmosphere", &cache.get_bind_group_layout(&render.atmo), &BindGroupEntries::with_indices((
        (0, a), (1, s), (2, t), (3, v), (8, &atmo.transmittance_lut.default_view), (10, &atmo.sky_view_lut.default_view), (11, &atmo.aerial_view_lut.default_view), (12, &**atmo_sampler),
    )));
    let data = device.create_bind_group("fog_data", &cache.get_bind_group_layout(&render.data[(*msaa != Msaa::Off) as usize]), &BindGroupEntries::sequential((
        u, &map.texture_view, &ground.texture_view, &cookie.texture_view, &render.smp, depth_view, &noise.texture_view, &render.repeat,
    )));
    let comp = device.create_bind_group("fog_composite", &cache.get_bind_group_layout(&render.comp), &BindGroupEntries::sequential((&fv.low.default_view, &fv.t.default_view, &render.nearest)));
    let offsets = [atmo_i.index(), settings_i.index(), transforms_i.index(), view_i.offset];
    fn attach(view: &TextureView) -> Option<RenderPassColorAttachment<'_>> { Some(RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: Operations { load: LoadOp::Load, store: StoreOp::Store } }) }
    // Pass i (0 march, 1 composite) writes timestamps 2i (begin) and 2i + 1 (end).
    let ts = |i: u32| timing.as_ref().map(|t| wgpu::RenderPassTimestampWrites { query_set: &t.qs, beginning_of_pass_write_index: Some(2 * i), end_of_pass_write_index: Some(2 * i + 1) });
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("ground_fog_march"), color_attachments: &[attach(&fv.low.default_view), attach(&fv.t.default_view)],
        depth_stencil_attachment: None, timestamp_writes: ts(0), occlusion_query_set: None, multiview_mask: None,
    });
    pass.set_pipeline(march);
    pass.set_bind_group(0, &atmo_bg, &offsets);
    pass.set_bind_group(1, &data, &[]);
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
        e.resolve_query_set(&t.qs, 0..4, &t.resolve, 0);
        e.copy_buffer_to_buffer(&t.resolve, 0, &t.read, 0, 32);
        t.recorded = true;
    }
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
    for i in 0..2 { if ts[2 * i + 1] >= ts[2 * i] && ts[2 * i] > 0 { t.sums[i] += (ts[2 * i + 1] - ts[2 * i]) as f64 * period; } }
    t.n += 1;
    if t.n == 60 {
        let (march, composite) = (t.sums[0] / 60.0, t.sums[1] / 60.0);
        println!("FOGMS march {march:.3} | composite {composite:.3} | total {:.3}", march + composite);
        (t.sums, t.n) = ([0.0; 2], 0);
    }
}
