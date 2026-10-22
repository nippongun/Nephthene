//! Storm fronts, the host half. Evaluates `nephthene_core::storm` (authored + the seeded timetable) at the camera and the monotonic
//! storm clock, selects K, packs them into `CloudParams.storm` (clouds binding 6) and prints `STORMS n=<k>` when the count changes.
//! Config: `storms.enabled`; `storms.force = "<authored name>"` forces that authored storm alone (timetable off).
//! Storm time = monotonic game seconds x storms.clock_scale.
use bevy::{pbr::AtmosphereSettings, prelude::*};
use nephthene_core::{rain::smoothstep, storm::{self, Now, Storm, StormCfg}};
use crate::{clouds::{CloudParams, StormGpu}, sky::Weather};

pub struct StormPlugin;
impl Plugin for StormPlugin {
    fn build(&self, app: &mut App) {
        let mut cfg = app.world().resource::<Weather>().cfg.storms.clone();
        let force = cfg.force.clone();
        if force.is_some() { cfg.enabled = true; }
        if let Some(tag) = &force { assert!(cfg.authored.iter().any(|a| &a.name == tag), "storms.force = {tag}: not an authored storm"); }
        app.insert_resource(Storms { scene: cfg.scene.clone(), cfg, force, ..default() }).add_systems(Update, select_storms.after(crate::NephtheneSet::Tick).before(crate::NephtheneSet::Drive));
    }
}

/// Storm state. `scene` (set by a scene) also admits the authored storms tagged with it.
#[derive(Resource, Default)]
pub struct Storms {
    pub cfg: StormCfg, pub force: Option<String>, pub scene: Option<String>,
    /// Set by the dashboard Clouds toggle (off hides storms too).
    pub hide: bool,
    /// Alive-or-upcoming storms near the camera (timetable + authored), the K selected with their state at `t`, and storm time.
    all: Vec<Storm>, pub sel: Vec<(Storm, Now)>, pub t: f64,
    /// World-frame scene centre for authored bearings (latched at the first camera), and what `all` was built for.
    centre: Option<Vec2>, built: Option<(f64, Vec2)>, logged: Option<usize>,
}

pub(crate) fn select_storms(
    wx: Res<Weather>, mut st: ResMut<Storms>, cam: Query<&GlobalTransform, With<AtmosphereSettings>>, params: Option<ResMut<CloudParams>>,
) {
    let st = &mut *st;
    if !st.cfg.enabled {
        if st.logged.replace(0).is_none() { println!("STORMS off"); }
        return;
    }
    if st.hide {
        if let Some(mut m) = params { m.storm = StormGpu::default(); }
        return;
    }
    let Some(p) = cam.iter().next().map(|g| g.translation()) else { return };
    let [sx, sz] = wx.cfg.map.world_offset_m;
    let pos = Vec2::new(sx + p.x, sz + p.z);
    let centre = *st.centre.get_or_insert(pos);
    let t = wx.game_s() * st.cfg.clock_scale as f64;
    st.t = t;
    if st.built.is_none_or(|(bt, bp)| (t - bt).abs() >= st.cfg.rebuild_s || bp.distance(pos) >= st.cfg.rebuild_m) {
        let day0 = (wx.day as f64) * 86400.0 * st.cfg.clock_scale as f64;
        let mut all = if st.force.is_some() { vec![] } else {
            let map = &wx.map;
            let (gt, gf) = (st.cfg.genesis_type, st.cfg.genesis_floor);
            storm::timetable(&st.cfg, &st.cfg.seed, &|q| { let [_, pr, ty, _] = map.rgba(q.x, q.y); smoothstep(gt[0], gt[1], ty) * (gf + (1.0 - gf) * pr) }, t, pos)
        };
        let tag = st.force.clone().or(st.scene.clone());
        if let Some(tag) = tag {
            let mut cfg = st.cfg.clone();
            cfg.authored.retain(|a| st.force.as_ref().is_some_and(|f| f == &a.name) || a.scene == tag);
            all.extend(storm::authored(&cfg, day0, centre));
        }
        st.all = all;
        st.built = Some((t, pos));
    }
    let prev: Vec<u64> = st.sel.iter().map(|(s, _)| s.id).collect();
    st.sel = storm::select(&st.all, t, pos, storm::MAX_CELLS, &prev, &st.cfg);
    if st.logged != Some(st.sel.len()) {
        st.logged = Some(st.sel.len());
        println!("STORMS n={} ids={:?} t={:.0}s", st.sel.len(), st.sel.iter().map(|(s, _)| s.id).collect::<Vec<_>>(), t);
    }
    if let Some(mut m) = params {
        let mut g = StormGpu { n: st.sel.len() as u32, ..default() };
        for (i, v) in storm::pack(&st.sel).into_iter().enumerate() {
            g.cells[i] = Vec4::from(v);
            // v0 = (a.x, a.z, b.x, b.z) is map frame; the shader works in world xz.
            if i % storm::PACK_VEC4 == 0 { g.cells[i] -= Vec4::new(sx, sz, sx, sz); }
        }
        m.storm = g;
    }
}
