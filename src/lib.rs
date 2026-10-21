//! Nephthene: a weather system for Bevy. One plugin, one config.
//!
//! ```ignore
//! app.add_plugins(NephthenePlugin::default());                      // all defaults
//! app.add_plugins(NephthenePlugin::new(Config::load(path)?));       // from a JSON file
//! ```
//! The scene supplies a camera with [`bevy::pbr::AtmosphereSettings`] (plus `Exposure` / `DistanceFog` if it wants them driven) and one
//! `DirectionalLight` marked [`NephtheneSun`]. The plugin spawns the Atmosphere, the moon and the stars, moves the sun along the clock,
//! draws volumetric clouds with cloud shadows, rain, optional storms, optional terrain-following ground fog, and (feature `panel`) a mouse-driven weather panel.
//! Runtime state: [`Weather`], [`RainState`], [`Storms`]; mark a mesh [`WetGround`] to let rain wet it.
//! Every tunable is in [`Config`] (re-exported from `nephthene_core`).
use bevy::prelude::*;
pub use nephthene_core::{self as core, Config, WeatherMap};

mod clouds;
mod fog;
#[cfg(feature = "panel")]
mod panel;
mod rain;
mod sky;
mod storms;

pub use clouds::CloudParams;
pub use fog::FogState;
#[cfg(feature = "panel")]
pub use panel::{Panel, Wid};
pub use rain::{Ground, RainState, WetGround};
pub use sky::{Moon, NephtheneSun, Weather};
pub use storms::Storms;

/// Ordering handles for host systems: `Tick` is the clock / wind / sun update, `Drive` the cloud uniforms (after the panel's overrides).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum NephtheneSet { Tick, Drive }

pub struct NephthenePlugin {
    pub config: Config,
    /// The weather map; None = the built-in procedural one from `config.map`.
    pub map: Option<WeatherMap>,
}
impl Default for NephthenePlugin {
    fn default() -> Self { Self { config: Config::default(), map: None } }
}
impl NephthenePlugin {
    pub fn new(config: Config) -> Self { Self { config, map: None } }
}

impl Plugin for NephthenePlugin {
    fn build(&self, app: &mut App) {
        let map = self.map.clone().unwrap_or_else(|| WeatherMap::procedural(&self.config.map));
        sky::build(app, &self.config, map);
        if self.config.clouds.enabled { app.add_plugins(clouds::CloudPlugin); }
        app.add_plugins((rain::RainPlugin, storms::StormPlugin));
        app.add_plugins(fog::FogPlugin);
        #[cfg(feature = "panel")]
        app.add_plugins(panel::PanelPlugin);
    }
}
