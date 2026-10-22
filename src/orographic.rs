//! Orographic precipitation, main world: bakes `nephthene_core::orographic` over the `Ground` around the camera and keeps the latest
//! tile (fixed to the terrain); `sky::tick` folds it into `Weather.here`, so rain, fog and the panel read it. The first bake runs at
//! once (deterministic shots), later ones in the background when the tile goes stale. Off (`orographic.enabled` false) or without a
//! `Ground` it holds no tile and costs nothing. The cloud shader does not read it yet (slice 2).
use std::sync::Arc;
use bevy::{pbr::AtmosphereSettings, prelude::*, tasks::{AsyncComputeTaskPool, Task, futures::check_ready}};
use nephthene_core::{self as w, orographic as o};
use crate::{rain::Ground, sky::Weather};

/// The latest orographic tile, if any (world frame, fixed to the terrain).
#[derive(Resource, Default)]
pub struct Orographic {
    pub tile: Option<Arc<o::Tile>>,
    /// Tiles landed so far: a reader re-uploads when it changes.
    pub bakes: u32,
    task: Option<Task<o::Tile>>,
}

pub(crate) struct OroPlugin;
impl Plugin for OroPlugin {
    fn build(&self, app: &mut App) { app.init_resource::<Orographic>().add_systems(Update, bake.after(crate::NephtheneSet::Tick)); }
}

fn bake(mut st: ResMut<Orographic>, wx: Res<Weather>, ground: Option<Res<Ground>>, cam: Query<&GlobalTransform, With<AtmosphereSettings>>) {
    let c = &wx.cfg.orographic;
    let Some(ground) = ground.filter(|_| c.enabled) else {
        if st.tile.is_some() || st.task.is_some() { *st = Orographic::default(); }
        return;
    };
    if let Some(t) = st.task.as_mut().and_then(check_ready) { st.task = None; land(&mut st, t); }
    if st.task.is_some() { return; }
    let Some(p) = cam.iter().next().map(|g| g.translation()) else { return };
    let m = w::map_wind(wx.wind, wx.wind_mean_mps);
    // The air at the reference altitude over the camera (the bake lapses it to the tile's lowest ground); the climate's ground
    // wetness, not the rain's (a wetting shower would rebake the tile).
    let [mx, mz] = w::map_xz(&wx.cfg.map, wx.map_shift, p.x, p.z);
    let a = wx.air_field.state(&wx.cfg, wx.climate.as_ref(), wx.map_shift, mx, mz, wx.cfg.air.ref_alt_m, 0.0, wx.here.dust);
    let (centre, wind, temp, dew) = ([p.x, p.z], [m.x, m.y], a.temp_c, a.dew_c);
    if st.tile.as_ref().is_some_and(|t| !t.stale(c, centre, wind, temp, dew)) { return; }
    let (c, air, grid) = (c.clone(), wx.cfg.air.clone(), ground.0.clone());
    if st.tile.is_none() {
        let t = o::bake(&c, &air, &grid, centre, wind, temp, dew);
        land(&mut st, t);
        return;
    }
    st.task = Some(AsyncComputeTaskPool::get().spawn(async move { o::bake(&c, &air, &grid, centre, wind, temp, dew) }));
}

/// Keep a finished tile and say so (one line per bake).
fn land(st: &mut Orographic, t: o::Tile) {
    let max = |f: &[f32]| f.iter().cloned().fold(0.0, f32::max);
    println!("OROGRAPHIC tile ({:.0}, {:.0}) wind ({:.1}, {:.1}) m/s air {:.1} C dew {:.1} C (at the reference altitude): max precip {:.2} cover {:.2}", t.centre[0], t.centre[1], t.wind[0], t.wind[1], t.temp_c, t.dew_c, max(&t.precip), max(&t.cover));
    st.bakes += 1;
    st.tile = Some(Arc::new(t));
}
