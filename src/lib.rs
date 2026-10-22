//! Nephthene: a weather system for Bevy. One plugin, one config.
//!
//! ```ignore
//! app.add_plugins(NephthenePlugin::default());                      // all defaults
//! app.add_plugins(NephthenePlugin::new(Config::load(path)?));       // from a JSON file
//! ```
//! The scene supplies a camera with [`bevy::pbr::AtmosphereSettings`] (plus `Exposure` / `DistanceFog` if it wants them driven) and one
//! `DirectionalLight` marked [`NephtheneSun`]. The plugin spawns the Atmosphere, the moon and the stars, moves the sun along the clock,
//! draws volumetric clouds with cloud shadows, rain, optional storms, optional terrain-following ground fog, and (feature `panel`) a mouse-driven weather panel.
//! Runtime state: [`Weather`], [`RainState`], [`Storms`], [`Flashes`], [`Orographic`]; messages [`LightningStrike`] and [`Thunder`]; mark a mesh [`WetGround`] to let rain wet it, or [`Shelter`] to keep rain out from under it.
//! Every tunable is in [`Config`] (re-exported from `nephthene_core`).
use bevy::prelude::*;
pub use nephthene_core::{self as core, Config, WeatherMap};

mod clouds;
mod fog;
mod lightning;
mod orographic;
#[cfg(feature = "panel")]
mod panel;
mod rain;
mod sky;
mod storms;

pub use clouds::CloudParams;
pub use fog::FogState;
pub use lightning::{Flashes, LightningStrike, Thunder};
pub use orographic::Orographic;
#[cfg(feature = "panel")]
pub use panel::{Panel, Wid};
pub use rain::{Ground, RainState, Shelter, WetGround};
pub use sky::{Moon, NephtheneSun, Weather};
pub use storms::Storms;

/// Ordering handles for host systems: `Tick` is the clock / wind / sun update, `Drive` the cloud uniforms (after the panel's overrides). The two sets run in that order: Tick, then Drive.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum NephtheneSet { Tick, Drive }

pub struct NephthenePlugin {
    pub config: Config,
    /// The weather map; None = the built-in procedural one from `config.map`.
    pub map: Option<WeatherMap>,
    /// The climate map (`WeatherMap::load_named(dir, "climate_map")`, fixed to the world); None = `config.climate.uniform` everywhere.
    pub climate: Option<WeatherMap>,
}
impl Default for NephthenePlugin {
    fn default() -> Self { Self { config: Config::default(), map: None, climate: None } }
}
impl NephthenePlugin {
    pub fn new(config: Config) -> Self { Self { config, map: None, climate: None } }
}

impl Plugin for NephthenePlugin {
    fn build(&self, app: &mut App) {
        // Drive reads what Tick and the overrides wrote this frame; a host system in Drive is ordered too, not only the plugin's own (which also say .after(Tick)).
        app.configure_sets(Update, (NephtheneSet::Tick, NephtheneSet::Drive).chain());
        let map = self.map.clone().unwrap_or_else(|| WeatherMap::procedural(&self.config.map));
        sky::build(app, &self.config, map, self.climate.clone());
        if self.config.clouds.enabled { app.add_plugins(clouds::CloudPlugin); }
        app.add_plugins((rain::RainPlugin, storms::StormPlugin, lightning::LightningPlugin));
        app.add_plugins(fog::FogPlugin);
        app.add_plugins(orographic::OroPlugin);
        #[cfg(feature = "panel")]
        app.add_plugins(panel::PanelPlugin);
    }
}
