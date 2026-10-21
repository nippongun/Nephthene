//! Nephthene example: a neutral scene (flat ground, sun, a flying camera) under the weather system.
//!
//! Interactive (your terminal only): `cargo run --release --example sky`. Hold the left mouse button and move to look around.
//! W/S move forward and back along the view, A/D strafe, E up, Q down, Left Shift is 5x, the wheel changes the base speed.
//! ` (Backquote) opens the weather panel (clock, coverage, wind, rain, map); while it is open the mouse look and wheel are off.
//! Headless (never opens a window; renders to an image): `cargo run --release --example sky -- --shot sky_west --out shots`.
//!
//! Shot options: `--shot <pose>` (examples/poses.json), `--out <dir>` (default shots), `--size WxH`, `--scale-factor f`,
//! `--hour h`, `--live` (clock and wind run; default frozen), `--config file.json`, `--set path=json` (repeatable, e.g.
//! `--set clouds.update_div=1`), `--settle frames` (default 60), `--seq count,every` / `--at a,b,c` (several frames per run),
//! `--sway deg` (camera sway), `--turn deg,frames` (yaw turn after the first shot), `--timeout s`, `--panel-test` (drive the panel with synthetic pointer input).
//! `--terrain`: the test valley (nephthene_core::fog::test_valley) as the ground mesh and the plugin's `Ground`, instead of the flat plane.
//! `--print-config` prints the effective config as JSON and exits (the reference file).
//! Exit codes: 0 ok, 3 timeout, 4 bad argument.
use bevy::{
    core_pipeline::tonemapping::Tonemapping,
    app::ScheduleRunnerPlugin,
    asset::RenderAssetUsages,
    camera::{ImageRenderTarget, RenderTarget, Exposure},
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
    light::{AtmosphereEnvironmentMapLight, CascadeShadowConfigBuilder, light_consts::lux},
    pbr::AtmosphereSettings,
    prelude::*,
    render::{render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages}, view::screenshot::{Screenshot, save_to_disk}, RenderPlugin},
    window::ExitCondition,
    winit::WinitPlugin,
};
use nephthene::{Config, NephtheneSun, NephthenePlugin, WetGround};
use serde::Deserialize;

#[derive(Deserialize, Clone)]
struct Pose { name: String, pos: [f32; 3], look_at: [f32; 3], #[serde(default)] fov_deg: Option<f32>, #[serde(default)] size: Option<[u32; 2]> }

struct Args(Vec<String>);
impl Args {
    fn get(&self, k: &str) -> Option<String> { self.0.iter().position(|s| s == k).and_then(|i| self.0.get(i + 1).cloned()) }
    fn all(&self, k: &str) -> Vec<String> { self.0.iter().enumerate().filter(|(_, s)| *s == k).filter_map(|(i, _)| self.0.get(i + 1).cloned()).collect() }
    fn has(&self, k: &str) -> bool { self.0.iter().any(|s| s == k) }
    fn num<T: std::str::FromStr>(&self, k: &str) -> Option<T> { self.get(k).map(|v| v.parse().unwrap_or_else(|_| die(&format!("{k} {v}: not a number")))) }
}
fn die(m: &str) -> ! { eprintln!("{m}"); std::process::exit(4) }

/// Default config, then --config, then --hour / --live, then each --set path=json (dotted path into the config tree).
fn config(a: &Args, shot: bool) -> Config {
    let mut v = serde_json::to_value(Config::default()).unwrap();
    let merge = |v: &mut serde_json::Value, patch: serde_json::Value| fn_merge(v, patch);
    if let Some(p) = a.get("--config") {
        let s = std::fs::read_to_string(&p).unwrap_or_else(|e| die(&format!("{p}: {e}")));
        merge(&mut v, serde_json::from_str(&s).unwrap_or_else(|e| die(&format!("{p}: {e}"))));
    }
    if let Some(h) = a.num::<f32>("--hour") { v["clock"]["start_hour"] = h.into(); }
    if shot && !a.has("--live") { v["clock"]["minutes_per_second"] = 0.0.into(); }
    for kv in a.all("--set") {
        let (path, val) = kv.split_once('=').unwrap_or_else(|| die(&format!("--set {kv}: want path=json")));
        let val = serde_json::from_str(val).unwrap_or_else(|_| serde_json::Value::String(val.into()));
        let mut cur = &mut v;
        for k in path.split('.') { cur = cur.get_mut(k).unwrap_or_else(|| die(&format!("--set {path}: no such key {k}"))); }
        *cur = val;
    }
    serde_json::from_value(v).unwrap_or_else(|e| die(&format!("config: {e}")))
}
fn fn_merge(a: &mut serde_json::Value, b: serde_json::Value) {
    match (a, b) {
        (serde_json::Value::Object(x), serde_json::Value::Object(y)) => for (k, v) in y { match x.get_mut(&k) { Some(e) => fn_merge(e, v), None => { x.insert(k, v); } } },
        (a, b) => *a = b,
    }
}

fn main() {
    let args = Args(std::env::args().skip(1).collect());
    let shot = args.get("--shot");
    let cfg = config(&args, shot.is_some());
    if args.has("--print-config") { println!("{}", cfg.to_json()); return; }
    if shot.is_none() && std::env::var("CLAUDECODE").as_deref() == Ok("1") && std::env::var("NEPHTHENE_ALLOW_WINDOW").is_err() {
        eprintln!("refusing to open a window under an agent (CLAUDECODE=1); use --shot, or set NEPHTHENE_ALLOW_WINDOW=1");
        std::process::exit(5);
    }
    let headless = shot.is_some();
    let plugins = DefaultPlugins
        .set(WindowPlugin {
            primary_window: (!headless).then(|| Window { title: "Nephthene".into(), ..default() }),
            exit_condition: if headless { ExitCondition::DontExit } else { ExitCondition::OnAllClosed }, ..default()
        })
        .set(RenderPlugin { synchronous_pipeline_compilation: headless, ..default() });
    let mut app = App::new();
    if headless {
        app.add_plugins((plugins.disable::<WinitPlugin>().disable::<bevy::audio::AudioPlugin>(), ScheduleRunnerPlugin::run_loop(std::time::Duration::from_millis(1))));
    } else {
        app.add_plugins(plugins);
    }
    if args.has("--terrain") { app.insert_resource(nephthene::Ground(nephthene::core::fog::test_valley())); }
    app.add_plugins(NephthenePlugin::new(cfg.clone())).insert_resource(Cfg(cfg)).add_systems(Startup, scene);
    match shot {
        Some(name) => { app.add_plugins(ShotPlugin { name, args }); }
        None => { app.add_systems(Update, fly); }
    }
    if let AppExit::Error(c) = app.run() { std::process::exit(c.get() as i32); }
}

#[derive(Resource)]
struct Cfg(Config);
#[derive(Component)]
struct Cam;

fn camera_bundle(cfg: &Config) -> impl Bundle {
    let f = &cfg.fog;
    let c = |c: [f32; 3]| Color::srgb(c[0], c[1], c[2]);
    (
        Camera3d::default(), Cam,
        Projection::Perspective(PerspectiveProjection { far: 20_000.0, ..default() }),
        Tonemapping::AcesFitted, Exposure { ev100: cfg.daylight_ev100 },
        AtmosphereSettings::default(), AtmosphereEnvironmentMapLight::default(),
        DistanceFog { color: c(f.color).with_alpha(1.0), falloff: FogFalloff::from_visibility_colors(f.visibility_m, c(f.extinction), c(f.inscattering)), ..default() },
    )
}

/// Flat ground (or the `--terrain` mesh over a lowered plane) and the sun; the camera is spawned by the mode (look-around or shot).
fn scene(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>, cfg: Res<Cfg>, terrain: Option<Res<nephthene::Ground>>) {
    let mat = mats.add(StandardMaterial { base_color: Color::srgb(0.55, 0.46, 0.34), perceptual_roughness: 0.95, reflectance: 0.1, ..default() });
    // Under the terrain the plane sits just below its lowest point, so it shows only past the terrain's edge.
    let below = terrain.as_ref().map_or(0.0, |t| t.0.y.iter().cloned().fold(f32::MAX, f32::min) - 10.0);
    commands.spawn((Name::new("Ground"), WetGround, Mesh3d(meshes.add(Plane3d::default().mesh().size(60_000.0, 60_000.0))), MeshMaterial3d(mat.clone()), Transform::from_xyz(0.0, below, 0.0)));
    if let Some(t) = terrain { commands.spawn((Name::new("Terrain"), WetGround, Mesh3d(meshes.add(terrain_mesh(&t.0))), MeshMaterial3d(mat))); }
    commands.spawn((
        NephtheneSun, DirectionalLight { illuminance: lux::RAW_SUNLIGHT, shadow_maps_enabled: true, ..default() },
        CascadeShadowConfigBuilder { maximum_distance: 250.0, first_cascade_far_bound: 8.0, ..default() }.build(), Transform::default(),
    ));
    let _ = cfg;
}

/// A height grid as a triangle mesh, one vertex per cell centre (rows north to south), smooth normals.
fn terrain_mesh(g: &nephthene::core::rain::HeightGrid) -> Mesh {
    use bevy::{asset::RenderAssetUsages, mesh::{Indices, PrimitiveTopology}};
    let pos: Vec<[f32; 3]> = (0..g.w * g.h).map(|k| [g.min_x + ((k % g.w) as f32 + 0.5) * g.cell_m, g.y[k as usize], g.max_z - ((k / g.w) as f32 + 0.5) * g.cell_m]).collect();
    let idx: Vec<u32> = (0..g.h - 1).flat_map(|j| (0..g.w - 1).flat_map(move |i| { let a = j * g.w + i; [a, a + 1, a + g.w, a + 1, a + g.w + 1, a + g.w] })).collect();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos).with_inserted_indices(Indices::U32(idx)).with_computed_smooth_normals()
}

// ---------------------------------------------------------------- interactive

fn fly(mut commands: Commands, cfg: Res<Cfg>, cams: Query<Entity, With<Cam>>, mut cam: Query<&mut Transform, With<Cam>>,
    mouse: Res<ButtonInput<MouseButton>>, keys: Res<ButtonInput<KeyCode>>, motion: Res<AccumulatedMouseMotion>, scroll: Res<AccumulatedMouseScroll>,
    panel: Option<Res<nephthene::Panel>>, time: Res<Time>, mut speed: Local<Option<f32>>) {
    if cams.is_empty() {
        commands.spawn((camera_bundle(&cfg.0), Transform::from_xyz(0.0, 1.7, 0.0).looking_at(Vec3::new(0.0, 400.0, 1000.0), Vec3::Y)));
        return;
    }
    let Ok(mut t) = cam.single_mut() else { return };
    // ponytail: base speed 20 m/s, 1.2x per wheel line, clamped to 1..2000 m/s; all untuned.
    let base = speed.get_or_insert(20.0);
    if !panel.is_some_and(|p| p.0.open) { // the panel owns the mouse while open
        if mouse.pressed(MouseButton::Left) {
            let (mut yaw, mut pitch, _) = t.rotation.to_euler(EulerRot::YXZ);
            yaw -= motion.delta.x * 0.003;
            pitch = (pitch - motion.delta.y * 0.003).clamp(-1.5, 1.5);
            t.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
        }
        *base = (*base * 1.2f32.powf(scroll.delta.y)).clamp(1.0, 2000.0);
    }
    // Ctrl, Alt and Cmd chords (Cmd+Q, Cmd+W) must not also fly the camera.
    let chord = [KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::AltLeft, KeyCode::AltRight, KeyCode::SuperLeft, KeyCode::SuperRight].iter().any(|k| keys.pressed(*k));
    if !chord {
        let (f, r) = (*t.forward(), *t.right());
        let mut d = Vec3::ZERO;
        if keys.pressed(KeyCode::KeyW) { d += f; }
        if keys.pressed(KeyCode::KeyS) { d -= f; }
        if keys.pressed(KeyCode::KeyD) { d += r; }
        if keys.pressed(KeyCode::KeyA) { d -= r; }
        if keys.pressed(KeyCode::KeyE) { d += Vec3::Y; }
        if keys.pressed(KeyCode::KeyQ) { d -= Vec3::Y; }
        // ponytail: Left Shift is 5x, untuned.
        let boost = if keys.pressed(KeyCode::ShiftLeft) { 5.0 } else { 1.0 };
        t.translation += d.normalize_or_zero() * *base * boost * time.delta_secs();
    }
}

// ---------------------------------------------------------------- headless shots

const SETTLE_FRAMES: u32 = 60;

struct ShotPlugin { name: String, args: Args }
impl Plugin for ShotPlugin {
    fn build(&self, app: &mut App) {
        let a = &self.args;
        let poses: Vec<Pose> = serde_json::from_str(include_str!("poses.json")).expect("poses.json");
        let Some(mut p) = poses.into_iter().find(|p| p.name == self.name) else { die(&format!("unknown pose {}", self.name)) };
        if let Some(s) = a.get("--size") {
            let (w, h) = s.split_once('x').unwrap_or_else(|| die("--size WxH"));
            p.size = Some([w.parse().unwrap_or_else(|_| die("--size")), h.parse().unwrap_or_else(|_| die("--size"))]);
        }
        let seq = a.get("--at").map(|v| (v.split(',').filter_map(|x| x.trim().parse::<u32>().ok()).collect::<Vec<_>>(), 1))
            .map(|(l, _)| (l.len() as u32 + 1, 1, Some(l)))
            .or_else(|| a.get("--seq").map(|v| { let (c, e) = v.split_once(',').unwrap_or_else(|| die("--seq count,every")); (c.trim().parse().unwrap_or(1), e.trim().parse::<u32>().unwrap_or(1).max(1), None) }));
        let out = a.get("--out").unwrap_or_else(|| "shots".into());
        std::fs::create_dir_all(&out).ok();
        // A stale PNG would satisfy the "file written" check, so a failed shot would exit 0.
        let _ = std::fs::remove_file(format!("{out}/{}.png", p.name));
        for i in 0..seq.as_ref().map_or(0, |s| s.0) { let _ = std::fs::remove_file(format!("{out}/{}_{i:02}.png", p.name)); }
        let turn = a.get("--turn").map(|v| { let (d, n) = v.split_once(',').unwrap_or_else(|| die("--turn deg,frames")); (d.trim().parse::<f32>().unwrap_or(0.0), n.trim().parse::<f32>().unwrap_or(1.0)) });
        app.insert_resource(Shot { pose: p, out, frames: 0, fired: false, taken: 0, seq, sf: a.num("--scale-factor").unwrap_or(1.0), sway: a.num("--sway"), turn,
                settle: a.num("--settle").unwrap_or(SETTLE_FRAMES), timeout: a.num("--timeout").unwrap_or(240.0), panel_test: a.has("--panel-test") })
            // Deterministic frames: 60 Hz of simulated time per update, whatever the wall clock does.
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_micros(16_667)))
            .add_systems(Startup, spawn_cam)
            // The sway / turn phase reads the frame count `drive` advances: ordered, so adding systems elsewhere cannot flip which runs first
            // (an unordered pair moved the golden sway frames when the fog plugin added systems).
            .add_systems(Update, (cam_motion, drive).chain());
    }
}

#[derive(Resource)]
struct Target(Handle<Image>);

#[derive(Resource)]
struct Shot { pose: Pose, out: String, frames: u32, fired: bool, taken: u32, seq: Option<(u32, u32, Option<Vec<u32>>)>, sf: f32, sway: Option<f32>, turn: Option<(f32, f32)>, settle: u32, timeout: f32, panel_test: bool }

fn spawn_cam(mut commands: Commands, shot: Res<Shot>, cfg: Res<Cfg>, mut images: ResMut<Assets<Image>>) {
    let p = &shot.pose;
    let [w, h] = p.size.unwrap_or([1600, 900]);
    let mut img = Image::new_uninit(Extent3d { width: w, height: h, depth_or_array_layers: 1 }, TextureDimension::D2, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
    img.texture_descriptor.usage |= TextureUsages::RENDER_ATTACHMENT;
    let target = images.add(img);
    commands.insert_resource(Target(target.clone()));
    let mut cam = commands.spawn((camera_bundle(&cfg.0), RenderTarget::Image(ImageRenderTarget { handle: target, scale_factor: shot.sf }),
        Transform::from_translation(Vec3::from(p.pos)).looking_at(Vec3::from(p.look_at), Vec3::Y), bevy::ui::IsDefaultUiCamera));
    if let Some(f) = p.fov_deg { cam.insert(Projection::Perspective(PerspectiveProjection { fov: f.to_radians(), far: 20_000.0, ..default() })); }
}

/// `--sway deg`: ~1 Hz yaw+pitch sway (60 fps frames), to reproduce temporal smear a static shot hides.
/// `--turn deg,frames`: yaw by deg over that many frames from the first shot, then hold (turn onto a cloud, stand still).
fn cam_motion(mut cams: Query<&mut Transform, With<Cam>>, shot: Res<Shot>, mut base: Local<Option<Quat>>) {
    let Ok(mut t) = cams.single_mut() else { return };
    let b = *base.get_or_insert(t.rotation);
    if let Some((deg, n)) = shot.turn {
        let k = if shot.fired { (shot.frames as f32 / n).min(1.0) } else { 0.0 };
        t.rotation = Quat::from_rotation_y((deg * k).to_radians()) * b;
    } else if let Some(deg) = shot.sway {
        // Deterministic across runs (load time varies): zero until the shot is ready, then counted in settled frames.
        let n = shot.frames + if shot.fired { shot.settle } else { 0 };
        let ph = n as f32 / 60.0 * std::f32::consts::TAU;
        // ponytail: sway pitch 0.6x and 1.3x pitch frequency are fixed.
        t.rotation = b * Quat::from_euler(EulerRot::YXZ, deg.to_radians() * ph.sin(), deg.to_radians() * 0.6 * (ph * 1.3).sin(), 0.0);
    }
}

fn drive(mut commands: Commands, mut shot: ResMut<Shot>, target: Res<Target>, mut exit: MessageWriter<AppExit>, mut started: Local<Option<std::time::Instant>>, mut ready_frames: Local<u32>) {
    let (out, name) = (shot.out.clone(), shot.pose.name.clone());
    let path = format!("{out}/{name}.png");
    let screenshot = |commands: &mut Commands, to: String, sf: f32| { commands.spawn(Screenshot(RenderTarget::Image(ImageRenderTarget { handle: target.0.clone(), scale_factor: sf }))).observe(save_to_disk(to)); };
    if shot.fired && let Some((count, every, at)) = shot.seq.clone() {
        shot.frames += 1;
        let file = |i: u32| format!("{out}/{name}_{i:02}.png");
        let due = at.as_ref().map_or(shot.frames % every == 0, |l| l.get(shot.taken as usize - 1) == Some(&shot.frames));
        if shot.taken < count && due {
            screenshot(&mut commands, file(shot.taken), shot.sf);
            shot.taken += 1;
        } else if shot.taken >= count && std::fs::metadata(file(count - 1)).is_ok_and(|m| m.len() > 0) && shot.frames > count * every + 10 { exit.write(AppExit::Success); }
    } else if shot.fired {
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 0) {
            shot.frames += 1;
            if shot.frames > 10 { exit.write(AppExit::Success); }
        }
    } else {
        *ready_frames += 1;
        shot.frames = *ready_frames;
        if shot.frames >= shot.settle {
            if shot.seq.is_some() { screenshot(&mut commands, format!("{out}/{name}_00.png"), shot.sf); shot.taken = 1; } else { screenshot(&mut commands, path, shot.sf); }
            shot.fired = true;
            shot.frames = 0;
        }
    }
    // OS wall clock: the simulated clock is manual.
    if started.get_or_insert_with(std::time::Instant::now).elapsed().as_secs_f32() > shot.timeout { eprintln!("timeout"); exit.write(AppExit::from_code(3)); }
}
