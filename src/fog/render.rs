//! Ground fog, render world: `fog_pass` runs after the main pass (where Bevy's volumetric fog runs) and blends shaders/fog.wesl over
//! the view target, reading the scene depth (sample 0 under MSAA) to stop the march at the first surface.
use std::collections::HashMap;
use bevy::{
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
        texture::GpuImage,
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
}

/// Main world -> render world, every frame: the uniform, the fog map (Rgba32Float), the ground heights (R32Float), the cloud cookie (R8).
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub struct FogParams { pub gpu: FogGpu, pub map: Handle<Image>, pub ground: Handle<Image>, pub cookie: Handle<Image> }

pub fn plugin(app: &mut App) {
    app.add_plugins(ExtractResourcePlugin::<FogParams>::default());
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app
        .add_systems(RenderStartup, init)
        .add_systems(Render, prepare.in_set(RenderSystems::PrepareResources))
        .add_systems(Core3d, fog_pass.after(Core3dSystems::MainPass).before(Core3dSystems::EarlyPostProcess));
}

#[derive(Resource)]
struct FogRender {
    atmo: BindGroupLayoutDescriptor, data: [BindGroupLayoutDescriptor; 2], shader: Handle<Shader>, fullscreen: FullscreenShader,
    smp: Sampler, ubo: UniformBuffer<FogGpu>, pipelines: HashMap<(bool, TextureFormat), CachedRenderPipelineId>,
}

#[derive(Component)]
struct FogPipeline(CachedRenderPipelineId);

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
        uniform_buffer::<FogGpu>(false), texture_2d(unfiltered), texture_2d(unfiltered), texture_2d(TextureSampleType::default()),
        sampler(SamplerBindingType::Filtering), if ms { texture_depth_2d_multisampled() } else { texture_depth_2d() },
    )));
    let smp = device.create_sampler(&SamplerDescriptor { mag_filter: FilterMode::Linear, min_filter: FilterMode::Linear, ..default() });
    commands.insert_resource(FogRender {
        atmo, data: [data(false), data(true)], shader: assets.load(FOG), fullscreen: fullscreen.clone(), smp, ubo: UniformBuffer::default(), pipelines: HashMap::new(),
    });
}

fn prepare(
    mut commands: Commands, params: Option<Res<FogParams>>, render: Option<ResMut<FogRender>>, cache: Res<PipelineCache>, device: Res<RenderDevice>, queue: Res<RenderQueue>,
    views: Query<(Entity, &Msaa, &ViewTarget), (With<ExtractedView>, With<ExtractedAtmosphere>)>,
) {
    let (Some(params), Some(mut render)) = (params, render) else { return };
    render.ubo.set(params.gpu);
    render.ubo.write_buffer(&device, &queue);
    for (e, msaa, target) in &views {
        let key = (*msaa != Msaa::Off, target.main_texture_format());
        let id = match render.pipelines.get(&key) {
            Some(p) => *p,
            None => {
                let p = cache.queue_render_pipeline(RenderPipelineDescriptor {
                    label: Some("ground_fog".into()),
                    layout: vec![render.atmo.clone(), render.data[key.0 as usize].clone()],
                    vertex: render.fullscreen.to_vertex_state(),
                    fragment: Some(FragmentState {
                        shader: render.shader.clone(), entry_point: Some("main".into()),
                        shader_defs: if key.0 { vec!["MULTISAMPLED".into()] } else { vec![] },
                        // Premultiplied over the scene; the target's alpha is kept.
                        targets: vec![Some(ColorTargetState { format: key.1, write_mask: ColorWrites::ALL, blend: Some(BlendState {
                            color: BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::OneMinusSrcAlpha, operation: BlendOperation::Add },
                            alpha: BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
                        }) })],
                        ..default()
                    }),
                    ..default()
                });
                render.pipelines.insert(key, p);
                p
            }
        };
        commands.entity(e).insert(FogPipeline(id));
    }
}

fn fog_pass(
    view: ViewQuery<(&ViewTarget, &ViewDepthStencilTexture, &Msaa, &FogPipeline, &AtmosphereTextures, &DynamicUniformIndex<GpuAtmosphere>, &DynamicUniformIndex<GpuAtmosphereSettings>, &AtmosphereTransformsOffset, &ViewUniformOffset)>,
    render: Option<Res<FogRender>>, params: Option<Res<FogParams>>, images: Res<RenderAssets<GpuImage>>, cache: Res<PipelineCache>,
    atmo_sampler: Res<AtmosphereSampler>, view_uniforms: Res<ViewUniforms>, transforms: Res<AtmosphereTransforms>,
    atmo_uniforms: Res<ComponentUniforms<GpuAtmosphere>>, settings_uniforms: Res<ComponentUniforms<GpuAtmosphereSettings>>, mut ctx: RenderContext,
) {
    let (Some(render), Some(params)) = (render, params) else { return };
    if params.gpu.y_hi <= params.gpu.y_lo && params.gpu.occ_on <= 0.0 { return; }
    let (target, depth, msaa, pipeline, atmo, atmo_i, settings_i, transforms_i, view_i) = view.into_inner();
    let Some(pipeline) = cache.get_render_pipeline(pipeline.0) else { return };
    let (Some(map), Some(ground), Some(cookie)) = (images.get(&params.map), images.get(&params.ground), images.get(&params.cookie)) else { return };
    let (Some(a), Some(s), Some(t), Some(v), Some(u)) = (atmo_uniforms.binding(), settings_uniforms.binding(), transforms.uniforms().binding(), view_uniforms.uniforms.binding(), render.ubo.binding()) else { return };
    let Some(depth_view) = depth.attachment.depth_stencil_views().depth_only_view() else { return };
    let device = ctx.render_device().clone();
    let atmo_bg = device.create_bind_group("fog_atmosphere", &cache.get_bind_group_layout(&render.atmo), &BindGroupEntries::with_indices((
        (0, a), (1, s), (2, t), (3, v), (8, &atmo.transmittance_lut.default_view), (10, &atmo.sky_view_lut.default_view), (11, &atmo.aerial_view_lut.default_view), (12, &**atmo_sampler),
    )));
    let data = device.create_bind_group("fog_data", &cache.get_bind_group_layout(&render.data[(*msaa != Msaa::Off) as usize]), &BindGroupEntries::sequential((
        u, &map.texture_view, &ground.texture_view, &cookie.texture_view, &render.smp, depth_view,
    )));
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("ground_fog"),
        color_attachments: &[Some(RenderPassColorAttachment { view: target.main_texture_view(), depth_slice: None, resolve_target: None, ops: Operations { load: LoadOp::Load, store: StoreOp::Store } })],
        depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &atmo_bg, &[atmo_i.index(), settings_i.index(), transforms_i.index(), view_i.offset]);
    pass.set_bind_group(1, &data, &[]);
    pass.draw(0..3, 0..1);
}
