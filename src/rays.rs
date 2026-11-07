//! Screen-space god rays (shaders/rays.wesl): per view, `source` thresholds and caps the scene's bright light, box-averaged into a
//! low-resolution target (config rays.source_scale), `march` blurs that toward the sun's screen position into a half-resolution target
//! (config rays.render_scale), then `composite` adds it to the HDR view target. Runs after the ground fog pass, before
//! Bevy's post-processing. Idle while config rays.enabled is off or the sun is behind the camera, below the horizon or off screen.
use std::collections::HashMap;
use bevy::{
    asset::embedded_asset,
    camera::MainPassResolutionOverride,
    core_pipeline::{FullscreenShader, schedule::Core3d, Core3dSystems},
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_resource::{binding_types::{sampler, texture_2d, uniform_buffer}, *},
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        texture::{CachedTexture, TextureCache},
        view::{ExtractedView, ViewTarget},
    },
    shader::Shader,
};

const RAYS: &str = "embedded://nephthene/shaders/rays.wesl";

/// Uniform for rays.wesl; field order identical in the shader (`struct Rays`).
#[derive(Clone, Copy, Default, ShaderType)]
pub struct RaysGpu {
    /// The sun's screen position (uv, 0..1, y down), 0..1 visibility (behind the camera, below the horizon, off screen fade it), rays.strength.
    pub sun_uv: Vec2, pub fade: f32, pub strength: f32,
    pub threshold: f32, pub decay: f32, pub radius: f32, pub samples: u32,
    /// View width / height; rays.flat_source as 0/1.
    pub aspect: f32, pub flat_source: u32,
    /// rays.source_max.
    pub source_max: f32,
    /// rays.local_mean as 0/1, rays.mean_width.
    pub local_mean: u32, pub mean_width: f32,
}

/// Main world -> render world, every frame: config rays and the sun's direction (towards the sun, world axes).
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub struct RaysParams { pub cfg: nephthene_core::config::Rays, pub sun_dir: Vec3 }

pub struct RaysPlugin;
impl Plugin for RaysPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/rays.wesl");
        app.init_resource::<RaysParams>()
            .add_plugins(ExtractResourcePlugin::<RaysParams>::default())
            .add_systems(Update, drive.in_set(crate::NephtheneSet::Drive));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .add_systems(RenderStartup, init)
            .add_systems(Render, prepare.in_set(RenderSystems::PrepareResources))
            .add_systems(Core3d, rays_pass.after(crate::fog::render::fog_pass).before(Core3dSystems::EarlyPostProcess));
    }
}

fn drive(wx: Res<crate::sky::Weather>, mut params: ResMut<RaysParams>, sun: Query<&Transform, (With<crate::sky::NephtheneSun>, Without<crate::sky::Moon>)>) {
    params.cfg = wx.cfg.rays.clone();
    if let Some(t) = sun.iter().next() { params.sun_dir = t.back().as_vec3(); }
}

#[derive(Resource)]
pub(crate) struct RaysRender {
    layout: BindGroupLayoutDescriptor, shader: Handle<Shader>, fullscreen: FullscreenShader, smp: Sampler,
    source: Option<CachedRenderPipelineId>, march: Option<CachedRenderPipelineId>, composite: HashMap<TextureFormat, CachedRenderPipelineId>,
}

/// The view's pipelines, its source and half-resolution march targets and the uniform (`fade` 0 skips the passes).
#[derive(Component)]
pub(crate) struct RaysView { source: CachedRenderPipelineId, march: CachedRenderPipelineId, composite: CachedRenderPipelineId, src_tex: CachedTexture, low: CachedTexture, ubo: UniformBuffer<RaysGpu>, fade: f32 }

fn init(mut commands: Commands, device: Res<RenderDevice>, assets: Res<AssetServer>, fullscreen: Res<FullscreenShader>) {
    let layout = BindGroupLayoutDescriptor::new("rays", &BindGroupLayoutEntries::sequential(ShaderStages::FRAGMENT, (
        uniform_buffer::<RaysGpu>(false), texture_2d(TextureSampleType::default()), sampler(SamplerBindingType::Filtering),
    )));
    let smp = device.create_sampler(&SamplerDescriptor { address_mode_u: AddressMode::ClampToEdge, address_mode_v: AddressMode::ClampToEdge, address_mode_w: AddressMode::ClampToEdge, mag_filter: FilterMode::Linear, min_filter: FilterMode::Linear, ..default() });
    commands.insert_resource(RaysRender { layout, shader: assets.load(RAYS), fullscreen: fullscreen.clone(), smp, source: None, march: None, composite: HashMap::new() });
}

fn prepare(
    mut commands: Commands, params: Option<Res<RaysParams>>, render: Option<ResMut<RaysRender>>, cache: Res<PipelineCache>, device: Res<RenderDevice>, queue: Res<RenderQueue>,
    mut textures: ResMut<TextureCache>, views: Query<(Entity, &ExtractedView, &ViewTarget, Option<&MainPassResolutionOverride>)>,
) {
    let (Some(params), Some(mut render)) = (params, render) else { return };
    let render = &mut *render;
    let c = &params.cfg;
    let source = *render.source.get_or_insert_with(|| cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("rays_source".into()),
        layout: vec![render.layout.clone()],
        vertex: render.fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: render.shader.clone(), entry_point: Some("source".into()),
            targets: vec![Some(TextureFormat::Rgba16Float.into())], ..default()
        }),
        ..default()
    }));
    let march = *render.march.get_or_insert_with(|| cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("rays_march".into()),
        layout: vec![render.layout.clone()],
        vertex: render.fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: render.shader.clone(), entry_point: Some("march".into()),
            targets: vec![Some(TextureFormat::Rgba16Float.into())], ..default()
        }),
        ..default()
    }));
    for (e, view, target, over) in &views {
        let format = target.main_texture_format();
        let composite = *render.composite.entry(format).or_insert_with(|| cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("rays_composite".into()),
            layout: vec![render.layout.clone()],
            vertex: render.fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: render.shader.clone(), entry_point: Some("composite".into()),
                // Additive over the scene; the target's alpha is kept.
                targets: vec![Some(ColorTargetState { format, write_mask: ColorWrites::ALL, blend: Some(BlendState {
                    color: BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
                    alpha: BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
                }) })],
                ..default()
            }),
            ..default()
        }));
        let full = over.map_or(view.viewport.zw(), |o| o.0);
        let src_size = (full.as_vec2() * c.source_scale.clamp(0.01, 1.0)).ceil().as_uvec2().max(UVec2::ONE);
        let src_tex = textures.get(&device, TextureDescriptor {
            label: Some("rays_source"), size: Extent3d { width: src_size.x, height: src_size.y, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: TextureDimension::D2, format: TextureFormat::Rgba16Float, usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING, view_formats: &[],
        });
        let size = (full.as_vec2() * c.render_scale.clamp(0.1, 1.0)).ceil().as_uvec2().max(UVec2::ONE);
        let low = textures.get(&device, TextureDescriptor {
            label: Some("rays_low"), size: Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: TextureDimension::D2, format: TextureFormat::Rgba16Float, usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING, view_formats: &[],
        });
        // The sun's screen position: view space, then clip. A point behind the camera (w <= 0) has no screen position.
        let rot = Mat3::from(view.world_from_view.affine().matrix3);
        let v = rot.transpose() * params.sun_dir;
        let clip = view.clip_from_view * v.extend(0.0);
        let (uv, mut fade) = if clip.w <= 1e-6 { (Vec2::ZERO, 0.0) } else {
            let ndc = clip.truncate().truncate() / clip.w;
            let uv = Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
            let out = (-uv.x).max(uv.x - 1.0).max((-uv.y).max(uv.y - 1.0)).max(0.0);
            (uv, 1.0 - nephthene_core::rain::smoothstep(0.0, c.edge_fade, out))
        };
        // ponytail: same sun-below-horizon cutoff as fog.wesl (-0.05); a config knob if it ever needs tuning.
        fade *= nephthene_core::rain::smoothstep(-0.05, 0.0, params.sun_dir.y);
        if !c.enabled { fade = 0.0; }
        let mut ubo = UniformBuffer::from(RaysGpu {
            sun_uv: uv, fade, strength: c.strength, threshold: c.threshold, decay: c.decay, radius: c.radius, samples: c.samples,
            aspect: full.x as f32 / full.y.max(1) as f32, flat_source: c.flat_source as u32, source_max: c.source_max,
            local_mean: c.local_mean as u32, mean_width: c.mean_width,
        });
        ubo.write_buffer(&device, &queue);
        commands.entity(e).insert(RaysView { source, march, composite, src_tex, low, ubo, fade });
    }
}

pub(crate) fn rays_pass(view: ViewQuery<(&ViewTarget, &RaysView)>, render: Option<Res<RaysRender>>, cache: Res<PipelineCache>, mut ctx: RenderContext) {
    let Some(render) = render else { return };
    let (target, rv) = view.into_inner();
    if rv.fade <= 0.0 { return; }
    let (Some(source), Some(march), Some(composite)) = (cache.get_render_pipeline(rv.source), cache.get_render_pipeline(rv.march), cache.get_render_pipeline(rv.composite)) else { return };
    let (Some(u), Some(u2), Some(u3)) = (rv.ubo.binding(), rv.ubo.binding(), rv.ubo.binding()) else { return };
    let device = ctx.render_device().clone();
    let source_bg = device.create_bind_group("rays_source", &cache.get_bind_group_layout(&render.layout), &BindGroupEntries::sequential((u, target.main_texture_view(), &render.smp)));
    let march_bg = device.create_bind_group("rays_march", &cache.get_bind_group_layout(&render.layout), &BindGroupEntries::sequential((u2, &rv.src_tex.default_view, &render.smp)));
    let composite_bg = device.create_bind_group("rays_composite", &cache.get_bind_group_layout(&render.layout), &BindGroupEntries::sequential((u3, &rv.low.default_view, &render.smp)));
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("rays_source"), color_attachments: &[Some(RenderPassColorAttachment { view: &rv.src_tex.default_view, depth_slice: None, resolve_target: None, ops: Operations { load: LoadOp::Clear(wgpu::Color::TRANSPARENT), store: StoreOp::Store } })],
        depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
    });
    pass.set_pipeline(source);
    pass.set_bind_group(0, &source_bg, &[]);
    pass.draw(0..3, 0..1);
    drop(pass);
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("rays_march"), color_attachments: &[Some(RenderPassColorAttachment { view: &rv.low.default_view, depth_slice: None, resolve_target: None, ops: Operations { load: LoadOp::Clear(wgpu::Color::TRANSPARENT), store: StoreOp::Store } })],
        depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
    });
    pass.set_pipeline(march);
    pass.set_bind_group(0, &march_bg, &[]);
    pass.draw(0..3, 0..1);
    drop(pass);
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("rays_composite"), color_attachments: &[Some(RenderPassColorAttachment { view: target.main_texture_view(), depth_slice: None, resolve_target: None, ops: Operations { load: LoadOp::Load, store: StoreOp::Store } })],
        depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
    });
    pass.set_pipeline(composite);
    pass.set_bind_group(0, &composite_bg, &[]);
    pass.draw(0..3, 0..1);
    drop(pass);
}
