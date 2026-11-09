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
//! `--sway deg` (camera sway), `--turn deg,frames` (yaw turn after the first shot), `--glide vx,vz` (m/s, the camera translates and keeps looking at the pose's `look_at`), `--timeout s`, `--panel-test` (scroll the panel with synthetic pointer input, print one PANEL_SCROLL line and exit;
//! `--panel-test-red`: the same with the old unclamped wheel, the gate's red arm).
//! `--terrain`: the test valley (nephthene_core::fog::test_valley) as the ground mesh and the plugin's `Ground`, instead of the flat plane.
//! `--mountains`: a mountain range (nephthene_core::terrain::mountains: 48 km square, peaks to about 2.9 km, plains either side) as the
//! ground instead; poses `mtn_west` (west plain, upwind of the default westerly), `mtn_east` (the lee) and `mtn_high` (overview).
//! `--pose <name>` (interactive): start the flying camera at that pose of examples/poses.json instead of the origin.
//! `--roof`: a 200 x 200 m slab 4 m up over the origin, marked `Shelter` (precipitation occlusion tests: no rain falls under it).
//! `--room` / `--room-window`: a closed 6 x 6 x 3.5 m room around the origin with a bright unlit ceiling over dim walls (opaque geometry
//! next to the sun on screen), the second with a 2 x 1.4 m window in the west wall; pose `room_sun` looks west toward the afternoon sun (G41).
//! `--print-config` prints the effective config as JSON and exits (the reference file).
//! `--describe` (shots): after the settle frames, prints one `DESCRIBE` line: the weather at the camera (`nephthene_core::dash::describe`).
//! Exit codes: 0 ok, 3 timeout, 4 bad argument.
use bevy::{
    asset::{load_internal_asset, uuid_handle},
    core_pipeline::tonemapping::Tonemapping,
    app::ScheduleRunnerPlugin,
    asset::RenderAssetUsages,
    camera::{ImageRenderTarget, RenderTarget, Exposure},
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
    light::{AtmosphereEnvironmentMapLight, CascadeShadowConfigBuilder, light_consts::lux},
    pbr::{AtmosphereSettings, ExtendedMaterial, MaterialExtension, MaterialPlugin},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
    render::{render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages}, view::screenshot::{Screenshot, save_to_disk}, RenderPlugin},
    window::ExitCondition,
    winit::WinitPlugin,
};
use nephthene::{Config, NephtheneSun, NephthenePlugin, Shelter, Surfaces, Weather};
use nephthene::core::{self as w, rain::Ground as _};
use serde::Deserialize;

/// The ground material this example draws the weather with. Nephthene itself ships no material and writes to none of the host's: it
/// publishes state, and `surface_system` below reads it through [`Surfaces`] and writes it in here. A host wanting a different look
/// (puddles, wet normal maps, drifted snow) replaces this file's worth of code and leaves the weather alone.
type SurfaceMaterial = ExtendedMaterial<StandardMaterial, WeatherSurface>;

const SURFACE_SHADER: Handle<Shader> = uuid_handle!("7e2a1c54-9b3d-4f16-8a07-5c1e9d2b4a38");

#[derive(Asset, TypePath, AsBindGroup, Clone, Default)]
struct WeatherSurface { #[uniform(100)] gpu: SurfaceGpu }
impl MaterialExtension for WeatherSurface {
    fn fragment_shader() -> ShaderRef { SURFACE_SHADER.into() }
}

/// Uniform for surface.wesl (field order identical in the shader).
#[derive(Clone, Copy, Default, ShaderType)]
struct SurfaceGpu {
    snow_color: Vec3, wet: f32, snow: f32, wet_darken: f32, wet_roughness: f32, snow_roughness: f32, slope_lo: f32, slope_hi: f32,
}

/// The look values come from `rain` in the config, which publishes them as suggestions; the shader mirrors
/// `core::rain::snow_mask` and the darkening half of `core::rain::wet_look`.
fn surface_gpu(c: &w::RainCfg, s: w::rain::Surface) -> SurfaceGpu {
    SurfaceGpu { snow_color: Vec3::from(c.snow_color), wet: s.wetness, snow: s.snow, wet_darken: c.wet_darken,
                 wet_roughness: c.wet_roughness, snow_roughness: c.snow_roughness, slope_lo: c.cover_slope_lo, slope_hi: c.cover_slope_hi }
}

/// Reads the weather at the camera through [`Surfaces`] and writes it into every [`SurfaceMaterial`]. Only when a value moved by
/// `rain.wet_epsilon` or a material was added, so the materials are not rewritten each frame. A host with per-tile ground materials
/// would call `surfaces.at` at each tile instead of once at the camera.
fn surface_system(surfaces: Surfaces, wx: Res<Weather>, cam: Query<&GlobalTransform, With<AtmosphereSettings>>,
    mut mats: ResMut<Assets<SurfaceMaterial>>, mut last: Local<(f32, f32, usize)>) {
    let c = &wx.cfg.rain;
    let p = cam.iter().next().map_or(Vec3::ZERO, |t| t.translation());
    let s = surfaces.at(p.x, p.z);
    let moved = (s.wetness - last.0).abs() >= c.wet_epsilon || (s.snow - last.1).abs() >= c.wet_epsilon;
    if !moved && mats.len() == last.2 { return; }
    // A newly added material gets the last written values, so a rewrite for it does not move the weather.
    let s = if moved { s } else { w::rain::Surface { wetness: last.0, snow: last.1 } };
    for (_, m) in mats.iter_mut() { m.extension.gpu = surface_gpu(c, s); }
    *last = (s.wetness, s.snow, mats.len());
}

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
/// The pose `name` of examples/poses.json; exit 4 if there is none.
fn pose(name: &str) -> Pose {
    let poses: Vec<Pose> = serde_json::from_str(include_str!("poses.json")).expect("poses.json");
    poses.into_iter().find(|p| p.name == name).unwrap_or_else(|| die(&format!("unknown pose {name}")))
}

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
    let mut cfg: Config = serde_json::from_value(v).unwrap_or_else(|e| die(&format!("config: {e}")));
    for kv in a.all("--set") {
        let (path, val) = kv.split_once('=').unwrap_or_else(|| die(&format!("--set {kv}: want path=json")));
        cfg.set_path(path, val).unwrap_or_else(|e| die(&e));
    }
    cfg
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
    let start = args.get("--pose").map(|n| pose(&n));
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
    if args.has("--mountains") { app.insert_resource(nephthene::Ground(nephthene::core::terrain::mountains())); }
    if args.has("--roof") { app.insert_resource(Roof); }
    if args.has("--room") || args.has("--room-window") { app.insert_resource(Room { window: args.has("--room-window") }); }
    app.add_plugins(NephthenePlugin::new(cfg.clone())).insert_resource(Cfg(cfg)).add_systems(Startup, scene);
    // The host owns its surfaces: this example's material, its shader and the system that fills it from Nephthene's published state.
    load_internal_asset!(app, SURFACE_SHADER, "surface.wesl", Shader::from_wesl);
    app.add_plugins(MaterialPlugin::<SurfaceMaterial>::default())
        .add_systems(Update, surface_system.after(nephthene::NephtheneSet::Drive));
    match shot {
        Some(name) => { app.add_plugins(ShotPlugin { name, args }); }
        None => { app.insert_resource(StartPose(start)).add_systems(Update, fly); }
    }
    if let AppExit::Error(c) = app.run() { std::process::exit(c.get() as i32); }
}

#[derive(Resource)]
struct Cfg(Config);
#[derive(Resource)]
struct Roof;
/// The G41 room: walls and ceiling, a west window when `window`.
#[derive(Resource)]
struct Room { window: bool }
#[derive(Resource)]
struct StartPose(Option<Pose>);
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

/// Flat ground (or the `--terrain` mesh over a lowered curved disc) and the sun; the camera is spawned by the mode (look-around or shot).
fn scene(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>, mut surf: ResMut<Assets<SurfaceMaterial>>, cfg: Res<Cfg>, terrain: Option<Res<nephthene::Ground>>, roof: Option<Res<Roof>>, room: Option<Res<Room>>) {
    let ground = surf.add(SurfaceMaterial { base: StandardMaterial { base_color: Color::srgb(0.55, 0.46, 0.34), perceptual_roughness: 0.95, reflectance: 0.1, ..default() }, extension: WeatherSurface::default() });
    // Under the terrain the plane sits just below its lowest point, so it shows only past the terrain's edge.
    let below = terrain.as_ref().map_or(0.0, |t| t.0.y.iter().cloned().fold(f32::MAX, f32::min) - 10.0);
    // A curved disc: a flat plane's far edge showed as a line under the horizon from altitude (the planet's horizon dips below it).
    commands.spawn((Name::new("Ground"), Mesh3d(meshes.add(ground_disc())), MeshMaterial3d(ground.clone()), Transform::from_xyz(0.0, below, 0.0)));
    if let Some(t) = terrain { commands.spawn((Name::new("Terrain"), Mesh3d(meshes.add(terrain_mesh(&t.0))), MeshMaterial3d(ground))); }
    commands.spawn((
        NephtheneSun, DirectionalLight { illuminance: lux::RAW_SUNLIGHT, shadow_maps_enabled: true, ..default() },
        CascadeShadowConfigBuilder { maximum_distance: 250.0, first_cascade_far_bound: 8.0, ..default() }.build(), Transform::default(),
    ));
    if roof.is_some() { commands.spawn((Name::new("Roof"), Shelter, Mesh3d(meshes.add(Cuboid::new(200.0, 0.3, 200.0))), MeshMaterial3d(mats.add(StandardMaterial { base_color: Color::srgb(0.5, 0.5, 0.5), perceptual_roughness: 0.9, ..default() })), Transform::from_xyz(0.0, 4.15, 0.0))); }
    if let Some(room) = room {
        // ponytail: test geometry, not a look. Unlit, so the values hold under any exposure: a bright ceiling (HDR 3, above
        // rays.threshold) over dim walls (0.3), a strong edge at the seam next to the sun like a sunlit ceiling indoors. (Uniform bright
        // walls send nothing: rays.local_mean cancels them.)
        let wall = mats.add(StandardMaterial { base_color: Color::linear_rgb(0.3, 0.3, 0.3), unlit: true, ..default() });
        let ceiling = mats.add(StandardMaterial { base_color: Color::linear_rgb(3.0, 3.0, 3.0), unlit: true, ..default() });
        // (centre, size): interior x and z -3..3, y 0..3.5, 0.2 m thick; the west wall (x = -3.1) in pieces around the window
        // (z -1..1, y 1.2..2.6) or whole.
        let mut parts = vec![
            (Vec3::new(0.0, 3.6, 0.0), Vec3::new(6.4, 0.2, 6.4)), (Vec3::new(3.1, 1.75, 0.0), Vec3::new(0.2, 3.5, 6.4)),
            (Vec3::new(0.0, 1.75, 3.1), Vec3::new(6.4, 3.5, 0.2)), (Vec3::new(0.0, 1.75, -3.1), Vec3::new(6.4, 3.5, 0.2)),
        ];
        if room.window {
            parts.extend([(Vec3::new(-3.1, 0.6, 0.0), Vec3::new(0.2, 1.2, 6.4)), (Vec3::new(-3.1, 3.05, 0.0), Vec3::new(0.2, 0.9, 6.4)),
                (Vec3::new(-3.1, 1.9, 2.1), Vec3::new(0.2, 1.4, 2.2)), (Vec3::new(-3.1, 1.9, -2.1), Vec3::new(0.2, 1.4, 2.2))]);
        } else {
            parts.push((Vec3::new(-3.1, 1.75, 0.0), Vec3::new(0.2, 3.5, 6.4)));
        }
        for (i, (c, s)) in parts.into_iter().enumerate() {
            let m = if i == 0 { ceiling.clone() } else { wall.clone() };
            commands.spawn((Name::new("Room"), Mesh3d(meshes.add(Cuboid::new(s.x, s.y, s.z))), MeshMaterial3d(m), Transform::from_translation(c)));
        }
    }
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

/// The ground beyond the terrain: a disc about 540 km in radius that drops with the planet's curve (y = -d^2 / 2R), so from any camera
/// height up to about 23 km its edge lies below the horizon. Rings grow outward (fine near the centre, coarse far away).
fn ground_disc() -> Mesh {
    use bevy::{asset::RenderAssetUsages, mesh::{Indices, PrimitiveTopology}};
    // ponytail: R is Bevy's Atmosphere::earth inner radius (6360 km), which the plugin spawns; read it from the Atmosphere to support
    // other planets. The ring spacing (first ring 50 m, x1.2 per ring, 52 rings) and the 128 segments are mesh resolution, not look.
    const R: f32 = 6_360_000.0;
    let (segs, rings) = (128u32, 52u32);
    let mut pos = vec![[0.0f32, 0.0, 0.0]];
    let mut nrm = vec![[0.0f32, 1.0, 0.0]];
    for k in 0..rings {
        let d = 50.0 * 1.2f32.powi(k as i32);
        for s in 0..segs {
            let a = s as f32 / segs as f32 * std::f32::consts::TAU;
            let (x, z) = (d * a.cos(), d * a.sin());
            pos.push([x, -d * d / (2.0 * R), z]);
            nrm.push(Vec3::new(x / R, 1.0, z / R).normalize().to_array());
        }
    }
    // Counter-clockwise seen from above (+y faces up): centre fan, then quads between rings.
    let mut idx: Vec<u32> = (0..segs).flat_map(|s| [0, 1 + (s + 1) % segs, 1 + s]).collect();
    for k in 0..rings - 1 {
        for s in 0..segs {
            let (a, b) = (1 + k * segs + s, 1 + k * segs + (s + 1) % segs);
            idx.extend([a, b, a + segs, b, b + segs, a + segs]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos).with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm).with_inserted_indices(Indices::U32(idx))
}

// ---------------------------------------------------------------- interactive

fn fly(mut commands: Commands, cfg: Res<Cfg>, start: Res<StartPose>, ground: Option<Res<nephthene::Ground>>, cams: Query<Entity, With<Cam>>, mut cam: Query<&mut Transform, With<Cam>>,
    mouse: Res<ButtonInput<MouseButton>>, keys: Res<ButtonInput<KeyCode>>, motion: Res<AccumulatedMouseMotion>, scroll: Res<AccumulatedMouseScroll>,
    #[cfg(feature = "panel")] panel: Option<Res<nephthene::Panel>>, time: Res<Time>, mut speed: Local<Option<f32>>) {
    if cams.is_empty() {
        // At the pose given, else 1.7 m above the ground at the origin (the mountains' origin is a peak), looking up the valley.
        let t = match &start.0 {
            Some(p) => Transform::from_translation(Vec3::from(p.pos)).looking_at(Vec3::from(p.look_at), Vec3::Y),
            None => {
                let y = ground.as_ref().map_or(0.0, |g| g.0.height(0.0, 0.0)) + 1.7;
                Transform::from_xyz(0.0, y, 0.0).looking_at(Vec3::new(0.0, y + 400.0, 1000.0), Vec3::Y)
            }
        };
        commands.spawn((camera_bundle(&cfg.0), t));
        return;
    }
    let Ok(mut t) = cam.single_mut() else { return };
    // ponytail: base speed 20 m/s, 1.2x per wheel line, clamped to 1..2000 m/s; all untuned.
    let base = speed.get_or_insert(20.0);
    #[cfg(feature = "panel")] let open = panel.is_some_and(|p| p.0.open);
    #[cfg(not(feature = "panel"))] let open = false;
    if !open { // the panel owns the mouse while open
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
        let mut p = pose(&self.name);
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
        let glide = a.get("--glide").map(|v| { let (x, z) = v.split_once(',').unwrap_or_else(|| die("--glide vx,vz")); Vec3::new(x.trim().parse().unwrap_or_else(|_| die("--glide")), 0.0, z.trim().parse().unwrap_or_else(|_| die("--glide"))) });
        app.insert_resource(Shot { pose: p, out, frames: 0, fired: false, taken: 0, seq, sf: a.num("--scale-factor").unwrap_or(1.0), sway: a.num("--sway"), turn, glide,
                settle: a.num("--settle").unwrap_or(SETTLE_FRAMES), timeout: a.num("--timeout").unwrap_or(240.0), panel_test: a.has("--panel-test") || a.has("--panel-test-red"), panel_red: a.has("--panel-test-red"), describe: a.has("--describe") })
            // Deterministic frames: 60 Hz of simulated time per update, whatever the wall clock does.
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_micros(16_667)))
            .add_systems(Startup, spawn_cam)
            // The sway / turn phase reads the frame count `drive` advances: ordered, so adding systems elsewhere cannot flip which runs first
            // (an unordered pair moved the golden sway frames when the fog plugin added systems).
            .add_systems(Update, (cam_motion, drive).chain());
        #[cfg(feature = "panel")]
        app.add_systems(Update, panel_scroll_test);
    }
}

/// Gate G37 (tests/panel_scroll.rs): scrolls the weather panel with synthetic pointer input over its rows, stops at the end, then
/// comes back three notches, and prints one `PANEL_SCROLL` line. Red arm (`panel_red`): the old hand-written wheel, which moved
/// the scroll position by 100 px per notch with no upper clamp.
#[cfg(feature = "panel")]
#[derive(Default)]
struct PanelScroll { stage: u8, down: u32, up: u32, until: u32, max: f32, bottom: f32 }

#[cfg(feature = "panel")]
fn panel_scroll_test(shot: Res<Shot>, target: Res<Target>, mut input: MessageWriter<bevy::picking::pointer::PointerInput>,
    mut areas: Query<(&mut ScrollPosition, &ComputedNode), With<bevy::ui_widgets::ScrollArea>>, mut exit: MessageWriter<AppExit>,
    mut frame: Local<u32>, mut st: Local<PanelScroll>) {
    use bevy::picking::pointer::{Location, PointerAction, PointerId, PointerInput};
    use bevy::camera::NormalizedRenderTarget;
    if !shot.panel_test { return; }
    *frame += 1;
    let f = *frame;
    // Layout and hover need a few frames before the pointer is sent.
    if f == 30 {
        let location = Location { target: NormalizedRenderTarget::Image(ImageRenderTarget { handle: target.0.clone(), scale_factor: shot.sf }), position: Vec2::new(235.0, 300.0) };
        input.write(PointerInput::new(PointerId::Mouse, location, PointerAction::Move { delta: Vec2::ZERO }));
        return;
    }
    if f < 40 { return; }
    // Several ScrollAreas exist (feathers widgets, most of them empty); the panel's body is the one that overflows.
    let overflow = |n: &ComputedNode| n.content_size().y - n.size().y;
    let Some((mut pos, node)) = areas.iter_mut().max_by(|a, b| overflow(a.1).total_cmp(&overflow(b.1))) else { eprintln!("panel test: no ScrollArea"); exit.write(AppExit::error()); return; };
    let location = Location { target: NormalizedRenderTarget::Image(ImageRenderTarget { handle: target.0.clone(), scale_factor: shot.sf }), position: Vec2::new(235.0, 300.0) };
    // One notch per frame: a Line scroll of -1 (down) or +1 (up), or the red arm's direct write.
    let mut notch = |down: bool, pos: &mut ScrollPosition| {
        if shot.panel_red {
            let step = if down { 100.0 } else { -100.0 };
            pos.y = (pos.y + step).max(0.0);
        } else {
            let y = if down { -1.0 } else { 1.0 };
            input.write(PointerInput::new(PointerId::Mouse, location.clone(), PointerAction::Scroll { unit: bevy::input::mouse::MouseScrollUnit::Line, x: 0.0, y, phase: bevy::input::touch::TouchPhase::Moved }));
        }
    };
    match st.stage {
        0 => {
            // ponytail: 100 px per notch is bevy's default line height; 20 extra notches overshoot the end on purpose.
            let max = ((node.content_size() - node.size()).y * node.inverse_scale_factor).max(0.0);
            st.max = max;
            st.down = (max / 100.0).ceil() as u32 + 20;
            st.stage = 1;
            notch(true, &mut pos);
            st.down -= 1;
        }
        1 => {
            if st.down > 0 { notch(true, &mut pos); st.down -= 1; }
            if st.down == 0 { st.stage = 2; st.until = f + 5; }
        }
        2 => if f >= st.until {
            st.bottom = pos.y;
            st.up = 3;
            st.stage = 3;
            notch(false, &mut pos);
            st.up -= 1;
        },
        3 => {
            if st.up > 0 { notch(false, &mut pos); st.up -= 1; }
            if st.up == 0 { st.stage = 4; st.until = f + 5; }
        }
        _ => if f >= st.until {
            println!("PANEL_SCROLL max={:.1} bottom={:.1} back={:.1}", st.max, st.bottom, pos.y);
            exit.write(AppExit::Success);
        },
    }
}

#[derive(Resource)]
struct Target(Handle<Image>);

#[derive(Resource)]
struct Shot { pose: Pose, out: String, frames: u32, fired: bool, taken: u32, seq: Option<(u32, u32, Option<Vec<u32>>)>, sf: f32, sway: Option<f32>, turn: Option<(f32, f32)>, glide: Option<Vec3>, settle: u32, timeout: f32, panel_test: bool, panel_red: bool, describe: bool }

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
    if let Some(v) = shot.glide {
        // 60 Hz like the manual clock, from the first frame: the settle frames run at speed too, so the cookie is already moving.
        t.translation += v / 60.0;
        *t = t.looking_at(Vec3::from(shot.pose.look_at), Vec3::Y);
        return;
    }
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

fn drive(mut commands: Commands, mut shot: ResMut<Shot>, target: Res<Target>, wx: Res<Weather>, mut exit: MessageWriter<AppExit>, mut started: Local<Option<std::time::Instant>>, mut ready_frames: Local<u32>) {
    // The panel test takes no shot and exits on its own; only the wall-clock timeout still applies.
    if shot.panel_test {
        if started.get_or_insert_with(std::time::Instant::now).elapsed().as_secs_f32() > shot.timeout { eprintln!("timeout"); exit.write(AppExit::from_code(3)); }
        return;
    }
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
            if shot.describe {
                let s = nephthene::core::dash::State { hour: wx.hour, day: wx.day, coverage: wx.here.coverage, precip: wx.here.precip, dust: wx.here.dust,
                    wind_from_deg: nephthene::core::dash::heading_deg([-wx.wind_local.x, 0.0, -wx.wind_local.z]), wind_mps: wx.wind_mean_mps, map_shift_m: [wx.map_shift.x, wx.map_shift.y] };
                println!("DESCRIBE {}", nephthene::core::dash::describe(&s));
            }
        }
    }
    // OS wall clock: the simulated clock is manual.
    if started.get_or_insert_with(std::time::Instant::now).elapsed().as_secs_f32() > shot.timeout { eprintln!("timeout"); exit.write(AppExit::from_code(3)); }
}
