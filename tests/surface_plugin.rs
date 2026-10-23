//! `SurfaceMaterialPlugin` stands alone: a host without `NephthenePlugin` (a headless harness without weather) gets
//! `Assets<SurfaceMaterial>` and runs. Red arm: the same app without the plugin has no such asset store (the reported panic).
use bevy::prelude::*;
use nephthene::{SurfaceMaterial, SurfaceMaterialPlugin, WeatherSurface};

fn app(surface: bool) -> App {
    let mut a = App::new();
    a.add_plugins((MinimalPlugins, AssetPlugin::default()));
    if surface {
        a.add_plugins(SurfaceMaterialPlugin);
    }
    a
}

#[test]
fn surface_plugin_without_weather() {
    assert!(!app(false).world().contains_resource::<Assets<SurfaceMaterial>>(), "red arm: the store exists without the plugin, so the check cannot fail");
    let mut a = app(true);
    a.finish();
    a.cleanup();
    let h = a.world_mut().resource_mut::<Assets<SurfaceMaterial>>().add(SurfaceMaterial { base: StandardMaterial::default(), extension: WeatherSurface::default() });
    for _ in 0..3 {
        a.update();
    }
    assert!(a.world().resource::<Assets<SurfaceMaterial>>().get(&h).is_some());
}
