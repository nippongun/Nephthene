//! Lightning, the host half. Turns the storm flash schedule (`nephthene_core::storm_fx::flashes`, storm time) into live flashes
//! (real time: a flash and its thunder play out in seconds whatever the time-lapse): the strike and channel come from
//! `nephthene_core::lightning`, the newest cloud-to-ground channel is drawn as camera-facing ribbons (shaders/lightning.wesl,
//! the rain quad scheme), each live flash lights the scene with a point light, and the host gets [`LightningStrike`] when a flash
//! starts and [`Thunder`] when its sound reaches the camera. Prints `LIGHTNING cg=<b> d=<m> energy=<e>` per flash and
//! `THUNDER d=<m> energy=<e>` when its thunder arrives.
//! Config: `storms.lightning` (on by default); `force_age_s` holds a flash for frozen-clock shots.
//! ponytail: the clouds are lit by each live flash with no occlusion toward it (clouds_march.wesl `flash_light`), and the channel is
//! not hidden by nearer clouds; a short storm_od toward each flash is the upgrade.
use bevy::{
    asset::embedded_asset,
    camera::visibility::NoFrustumCulling,
    light::NotShadowCaster,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{AtmosphereSettings, Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin, MeshMaterial3d},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError},
    shader::ShaderRef,
};
use nephthene_core::{lightning::{self as lt, Seg, Strike, MAX_SEGMENTS}, rain::Ground as _, storm_fx::{self, Flash}};
use crate::{rain::Ground, sky::Weather, storms::Storms};

pub struct LightningPlugin;
impl Plugin for LightningPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/lightning.wesl");
        app.add_plugins(MaterialPlugin::<BoltMaterial>::default())
            .add_message::<LightningStrike>().add_message::<Thunder>()
            .init_resource::<Flashes>()
            .add_systems(PostStartup, spawn)
            .add_systems(Update, (flash, drive, cloud_flashes).chain().after(crate::storms::select_storms).before(crate::NephtheneSet::Drive));
    }
}

/// A flash started (sent on its first frame). World metres: the channel runs from `top` (cloud base, or one end in the cloud) to
/// `end` (the ground for cloud-to-ground); `distance_m` is from the camera to that line.
#[derive(Message, Clone, Debug)]
pub struct LightningStrike { pub top: Vec3, pub end: Vec3, pub cg: bool, pub energy: f32, pub distance_m: f32 }

/// A flash's thunder reaches the camera: sent `distance_m / storms.fx.sound_mps` seconds after its [`LightningStrike`].
#[derive(Message, Clone, Debug)]
pub struct Thunder { pub distance_m: f32, pub energy: f32, pub cg: bool }

/// One live flash: its strike (world frame), channel, start (real seconds) and brightness this frame (envelope x energy).
#[derive(Clone, Debug)]
pub struct Live { pub strike: Strike, pub bolt: Vec<Seg>, pub born_s: f64, pub light: f32 }

/// Live flashes, oldest first, and the thunder still on its way.
#[derive(Resource, Default)]
pub struct Flashes { pub live: Vec<Live>, thunder: Vec<(f64, Thunder)>, t_prev: Option<f64> }

/// Uniform for lightning.wesl (`struct Bolt`, field order identical in the shader). `radiance` already holds the flash brightness.
#[derive(Clone, Copy, Default, ShaderType)]
struct BoltGpu {
    color: Vec3, radiance: f32,
    extinction: Vec3, width: f32,
    min_px: f32, base_y: f32, base_fade: f32, n: u32,
}
/// The channel (lightning.wesl `struct Segs`): segment i is (a.xyz, brightness), (b.xyz, 0) at 2i, 2i + 1.
#[derive(Clone, Copy, ShaderType)]
struct BoltSegs { s: [Vec4; 2 * MAX_SEGMENTS] }
impl Default for BoltSegs { fn default() -> Self { Self { s: [Vec4::ZERO; 2 * MAX_SEGMENTS] } } }

#[derive(Asset, TypePath, AsBindGroup, Clone, Default)]
struct BoltMaterial { #[uniform(0)] gpu: BoltGpu, #[uniform(1)] segs: BoltSegs }
impl Material for BoltMaterial {
    fn vertex_shader() -> ShaderRef { "embedded://nephthene/shaders/lightning.wesl".into() }
    fn fragment_shader() -> ShaderRef { "embedded://nephthene/shaders/lightning.wesl".into() }
    fn alpha_mode(&self) -> AlphaMode { AlphaMode::Premultiplied }
    fn enable_prepass() -> bool { false }
    fn enable_shadows() -> bool { false }
    // Camera-facing ribbons: winding depends on the view, so draw both faces.
    fn specialize(_: &MaterialPipeline, d: &mut RenderPipelineDescriptor, _: &MeshVertexBufferLayoutRef, _: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        d.primitive.cull_mode = None;
        Ok(())
    }
}

#[derive(Component)]
struct BoltMesh(Handle<BoltMaterial>);
#[derive(Component)]
struct FlashLight(usize);

fn spawn(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<BoltMaterial>>, st: Res<Storms>) {
    let c = &st.cfg.lightning;
    if !c.enabled { return; }
    let m = mats.add(BoltMaterial::default());
    commands.spawn((Name::new("LightningBolt"), Mesh3d(meshes.add(crate::rain::quads(MAX_SEGMENTS as u32))), MeshMaterial3d(m.clone()),
        BoltMesh(m), NoFrustumCulling, NotShadowCaster, Visibility::Hidden));
    for i in 0..c.max_live as usize {
        commands.spawn((Name::new("LightningLight"), FlashLight(i), PointLight { intensity: 0.0, range: c.light_range_m, ..default() }, Transform::default(), Visibility::Hidden));
    }
}

/// New flashes from the schedule (or the forced one), their messages and thunder, and each live flash's brightness.
fn flash(
    time: Res<Time>, wx: Res<Weather>, st: Res<Storms>, ground: Option<Res<Ground>>, cam: Query<&GlobalTransform, With<AtmosphereSettings>>,
    mut fl: ResMut<Flashes>, mut strikes: MessageWriter<LightningStrike>, mut thunder: MessageWriter<Thunder>,
) {
    let c = &st.cfg.lightning;
    let fl = &mut *fl;
    if !c.enabled || st.hide { (fl.live, fl.thunder, fl.t_prev) = (vec![], vec![], None); return; }
    let Some(cam) = cam.iter().next().map(|g| g.translation()) else { return };
    let now = time.elapsed_secs_f64();
    // Storms live in the map frame (world + map.world_offset_m, the unshifted map; see storms.rs).
    let [sx, sz] = wx.cfg.map.world_offset_m;
    let off = Vec3::new(sx, 0.0, sz);
    let grid = crate::rain::ground_grid(ground.as_deref());
    let gh = |p: Vec2| grid.height(p.x - sx, p.y - sz);
    let mut fresh = vec![];
    if let Some(age) = c.force_age_s {
        // Rebuilt every frame at a fixed age, so a frozen-clock shot sees the same flash; reported once.
        if let Some(l) = fl.live.first_mut() { l.born_s = now - age as f64; }
        else if let Some((s, n)) = st.sel.first() {
            let k = lt::strike(c, s, n, &Flash { t_s: 0.0, cg: true, energy: 1.0 }, &gh);
            fl.live.push(Live { strike: k, bolt: vec![], born_s: now - age as f64, light: 0.0 });
            fresh.push(0);
        }
    } else {
        let t = st.t;
        let from = fl.t_prev.replace(t).filter(|&p| t >= p && t - p <= c.jump_s).unwrap_or(t);
        for (s, n) in &st.sel {
            for f in storm_fx::flashes(s, &st.cfg, from, t) {
                fl.live.push(Live { strike: lt::strike(c, s, n, &f, &gh), bolt: vec![], born_s: now, light: 0.0 });
                fresh.push(fl.live.len() - 1);
            }
        }
    }
    for i in fresh {
        let l = &mut fl.live[i];
        (l.strike.top, l.strike.end) = (l.strike.top - off, l.strike.end - off);
        l.bolt = lt::bolt(c, &l.strike);
        let k = l.strike;
        let d = lt::distance_m(&k, cam);
        println!("LIGHTNING cg={} d={d:.0}m energy={:.2}", k.cg, k.energy);
        strikes.write(LightningStrike { top: k.top, end: k.end, cg: k.cg, energy: k.energy, distance_m: d });
        fl.thunder.push((now + storm_fx::thunder_delay_s(&st.cfg, d) as f64, Thunder { distance_m: d, energy: k.energy, cg: k.cg }));
    }
    // Every new flash was reported; only the newest `max_live` stay drawn and lit.
    let over = fl.live.len().saturating_sub(c.max_live as usize);
    fl.live.drain(..over);
    fl.live.retain(|l| now - l.born_s < c.life_s as f64);
    for l in &mut fl.live { l.light = l.strike.energy * lt::envelope(c, l.strike.seed, (now - l.born_s) as f32); }
    fl.thunder.retain(|(due, t)| {
        if *due > now { return true; }
        println!("THUNDER d={:.0}m energy={:.2}", t.distance_m, t.energy);
        thunder.write(t.clone());
        false
    });
}

/// The newest cloud-to-ground channel into the bolt material, one point light per live flash.
fn drive(
    fl: Res<Flashes>, st: Res<Storms>, fog: Query<&DistanceFog, With<AtmosphereSettings>>, mut mats: ResMut<Assets<BoltMaterial>>,
    mut bolt: Query<(&BoltMesh, &mut Visibility), Without<FlashLight>>, mut lights: Query<(&FlashLight, &mut PointLight, &mut Transform, &mut Visibility)>,
) {
    let c = &st.cfg.lightning;
    for (FlashLight(i), mut p, mut tf, mut vis) in &mut lights {
        let l = fl.live.get(*i).filter(|l| l.light > 0.0);
        if let Some(l) = l {
            tf.translation = l.strike.end.lerp(l.strike.top, c.light_at);
            p.intensity = c.light_lm * l.light;
        }
        let v = if l.is_some() { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != v { *vis = v; }
    }
    let live = fl.live.iter().rev().find(|l| l.strike.cg && l.light > 0.0);
    for (m, mut vis) in &mut bolt {
        let v = if live.is_some() { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != v { *vis = v; }
        let (Some(l), Some(mut mat)) = (live, mats.get_mut(&m.0)) else { continue };
        // The camera's own distance fog dims the channel as it dims the scene (Atmospheric falloff only; others leave it clear).
        let extinction = match fog.iter().next().map(|f| &f.falloff) { Some(FogFalloff::Atmospheric { extinction, .. }) => *extinction, _ => Vec3::ZERO };
        mat.gpu = BoltGpu { color: Vec3::from(c.color), radiance: c.radiance * l.light, extinction, width: c.width_m,
                            min_px: c.min_px, base_y: l.strike.top.y, base_fade: c.base_fade_m, n: l.bolt.len() as u32 };
        for (i, s) in l.bolt.iter().enumerate() {
            (mat.segs.s[2 * i], mat.segs.s[2 * i + 1]) = (s.a.extend(s.bright), s.b.extend(0.0));
        }
    }
}

/// Live flashes into the cloud march's flash uniform (`CloudParams.flash`): each at its point light's place, glow radius
/// `cloud_radius_m`, colour x `lightning::cloud_flash_scale` (the clouds light up the same frame; nothing goes through the history).
fn cloud_flashes(fl: Res<Flashes>, st: Res<Storms>, params: Option<ResMut<crate::clouds::CloudParams>>) {
    let Some(mut params) = params else { return };
    let c = &st.cfg.lightning;
    let mut g = crate::clouds::render::FlashGpu::default();
    for l in fl.live.iter().filter(|l| l.light > 0.0).take(lt::MAX_CLOUD_FLASHES) {
        let i = g.n as usize;
        g.p[i] = l.strike.end.lerp(l.strike.top, c.light_at).extend(c.cloud_radius_m);
        g.l[i] = (Vec3::from(c.color) * lt::cloud_flash_scale(c, l.light)).extend(0.0);
        g.n += 1;
    }
    params.flash = g;
}
