//! Storm fronts, the host half. Evaluates `nephthene_core::storm` (authored + the seeded timetable) at the camera and the monotonic
//! storm clock, selects K, packs them into `CloudParams.storm` (clouds binding 6) and prints `STORMS n=<k>` when the count changes.
//! Config: `storms.enabled`; `storms.force = "<authored name>"` forces that authored storm alone (timetable off).
//! Storm time = monotonic game seconds x storms.clock_scale. The selected storms' rain, cover, gust front and sky light at the camera
//! go to `Weather.storm` (`nephthene_core::storm_fx::local`; `storms.couple` false leaves the weather alone).
//! storms.summon_at_start / `Storms::summon` add a mature cell upwind that drifts over the camera.
use bevy::{pbr::AtmosphereSettings, prelude::*};
use nephthene_core::{rain::smoothstep, storm::{self, Now, Storm, StormCfg}, storm_fx};
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
    /// Storms summoned by the panel's "Storm now" or `storms.summon_at_start` (map frame); they join the timetable until they die.
    pub summoned: Vec<Storm>,
    /// Summons so far (summoned ids never repeat).
    next_summon: u64,
    /// First-frame summon done (`storms.summon_at_start`).
    started: bool,
    /// Alive-or-upcoming storms near the camera (timetable + authored), the K selected with their state at `t`, and storm time.
    all: Vec<Storm>, pub sel: Vec<(Storm, Now)>, pub t: f64,
    /// World-frame scene centre for authored bearings (latched at the first camera), and what `all` was built for.
    centre: Option<Vec2>, built: Option<(f64, Vec2)>, logged: Option<usize>,
    /// The storm weather at the camera last printed (tenths of precip, cover, sky light).
    here_logged: Option<[i32; 3]>,
}

impl Storms {
    /// A mature cell `storms.summon_dist_m` upwind of the camera at world `cam`, drifting over it (the panel's "Storm now").
    pub fn summon(&mut self, wx: &Weather, cam: Vec3) {
        let [sx, sz] = wx.cfg.map.world_offset_m;
        let t = wx.game_s() * self.cfg.clock_scale as f64;
        // Upwind: the wind vector points where the air goes.
        let from = -Vec2::new(wx.wind.x, wx.wind.z);
        // Summoned ids: a tag in the top bits (timetable ids are hashes, authored ids carry 0xA000_0000) and a count that never
        // repeats, as `select` keys its hysteresis on ids.
        self.next_summon += 1;
        let id = 0xC000_0000_0000_0000 | self.next_summon;
        self.summoned.push(storm::summon(&self.cfg, id, t, Vec2::new(cam.x + sx, cam.z + sz), from));
        println!("STORM summoned {:.1} km upwind", self.cfg.summon_dist_m / 1000.0);
        self.built = None;
    }
}

pub(crate) fn select_storms(
    mut wx: ResMut<Weather>, mut st: ResMut<Storms>, cam: Query<&GlobalTransform, With<AtmosphereSettings>>, params: Option<ResMut<CloudParams>>,
) {
    let st = &mut *st;
    if !st.cfg.enabled {
        if st.logged.replace(0).is_none() { println!("STORMS off"); }
        if let Some(mut m) = params { m.storm = StormGpu::default(); }
        wx.storm = default();
        st.sel.clear();
        return;
    }
    if st.hide {
        if let Some(mut m) = params { m.storm = StormGpu::default(); }
        wx.storm = default();
        st.sel.clear();
        return;
    }
    let Some(p) = cam.iter().next().map(|g| g.translation()) else { return };
    if st.cfg.summon_at_start && !st.started { st.started = true; st.summon(&wx, p); }
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
        st.summoned.retain(|s| s.born_s + s.life_s as f64 >= t);
        all.extend(st.summoned.iter().cloned());
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
    // The weather the storms make at the camera (storm frame; at the ground, or the camera's height above it).
    let l = storm_fx::local(&st.cfg, &st.sel, Vec3::new(pos.x, p.y.max(0.0), pos.y));
    let key = [l.precip, l.cover, l.ambient].map(|v| (v * 10.0).round() as i32);
    if st.here_logged.replace(key) != Some(key) {
        println!("STORMHERE precip {:.2} cover {:.2} sky {:.2} gust {:.1} m/s", l.precip, l.cover, l.ambient, l.outflow.length());
    }
    wx.storm = l;
}
