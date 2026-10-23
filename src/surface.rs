//! The optional weather ground material (feature surface). Nephthene publishes the weather (RainState, SurfaceWeather); how a host's surfaces look is the host's call. This is a ready-made look for hosts that want one, and the example's ground: [`SurfaceMaterialPlugin`] registers [`SurfaceMaterial`] and the [`WetGround`] system, with or without NephthenePlugin; without the weather resources its systems do not run.
use crate::{rain::RainState, sky::Weather};
use bevy::{
    asset::embedded_asset,
    pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin, MeshMaterial3d},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
};
use nephthene_core::{self as w, rain as r};

pub struct SurfaceMaterialPlugin;
impl Plugin for SurfaceMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/surface.wesl");
        // Runs only beside the weather (NephthenePlugin with rain on); alone, the material keeps its dry default.
        let weather = |wx: Option<Res<Weather>>, rain: Option<Res<RainState>>| rain.is_some() && wx.is_some_and(|w| w.cfg.rain.enabled);
        app.add_plugins(MaterialPlugin::<SurfaceMaterial>::default())
            .add_systems(Update, (wet_ground, surface).chain().after(crate::NephtheneSet::Drive).run_if(weather));
    }
}

/// Marks a mesh whose `StandardMaterial` gets wet (darker, glossier) in rain. Its dry roughness and reflectance are read at the first sight.
#[derive(Component)]
pub struct WetGround;

/// The weather surface: a [`StandardMaterial`] that darkens and glosses with wetness and turns white with snow cover where the ground
/// is flat enough to hold it. Hosts opt in by adding [`SurfaceMaterialPlugin`] and giving ground meshes a [`SurfaceMaterial`]; the plugin writes the weather into every one.
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
