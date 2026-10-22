//! The sky half: drives the sun, Atmosphere and DistanceFog from the clock and the weather map in `nephthene_core`, and adds a
//! moon (a second sun disk) and stars (a Skybox the Atmosphere attenuates: render_sky blends `inscatter + background * transmittance`,
//! so they fade out by day on their own). Clouds are the dome in clouds.rs, composited after the sky.
//! The clock is frozen when `clock.minutes_per_second` is 0.
use bevy::{
    asset::RenderAssetUsages,
    camera::Exposure,
    light::{Atmosphere, Skybox, SunDisk, atmosphere::{Falloff, PhaseFunction, ScatteringMedium, ScatteringTerm}, light_consts::lux},
    pbr::AtmosphereSettings,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension},
};
use nephthene_core::{self as w, Config, Local as Here, WeatherMap, air};

mod msaa;

/// Marks the one `DirectionalLight` that is the sun: the plugin turns it along the clock, and the cloud shadow cookie rides on it.
#[derive(Component, Default)]
pub struct NephtheneSun;

/// Live weather state. `here` is the smoothed weather-map sample at the atmosphere camera; `wind` is m/s in world XZ.
/// `cfg` is the live configuration; fields the systems read each frame (sky, wind, fog, exposure) may be edited at runtime.
#[derive(Resource)]
pub struct Weather {
    pub hour: f32, pub rate: f32, pub here: Here, pub wind: Vec3,
    /// Whole days elapsed (`hour` wraps at 24); the monotonic game clock is `game_s()`.
    pub day: u32,
    /// Wind at the camera: today `wind`; storm outflow may add to it. Cloud drift stays on `wind`.
    pub wind_local: Vec3,
    /// Mean wind speed at the camera, m/s, without gusts: the config's, or the panel's override.
    pub wind_mean_mps: f32,
    /// How far the weather map has moved downwind (m, world XZ; config `map.advect`). Map coordinates = world + `map.world_offset_m` - this.
    pub map_shift: Vec2,
    /// Air at the camera (config `air`): surface temperature, dew point, humidity, freezing level and snow line. Integrated on the game
    /// clock with the cover overhead; a frozen clock spins it up for the hour instead.
    pub air: air::State,
    pub stars: Option<Handle<Image>>, pub cfg: Config, pub map: WeatherMap,
}

impl Weather {
    /// Monotonic game seconds: never wraps, so storm motion (a pure function of this) does not jump at midnight.
    pub fn game_s(&self) -> f64 { (self.day as f64 * 24.0 + self.hour as f64) * 3600.0 }
}

#[derive(Component)]
pub struct Moon;

pub(crate) fn build(app: &mut App, cfg: &Config, mut map: WeatherMap) {
    if cfg.sky.msaa_fix { msaa::override_render_sky(app); }
    map.wrap = cfg.map.wrap;
    let map_shift = Vec2::from(cfg.map.start_shift_m);
    let [x, z] = w::map_xz(&cfg.map, map_shift, 0.0, 0.0);
    let here = map.sample(x, z);
    let air = air::state(&cfg.air, &cfg.clock, air::spin_up(&cfg.air, &cfg.clock, cfg.clock.start_hour, here.coverage), 0.0, here.dust);
    app.insert_resource(Weather { hour: cfg.clock.start_hour, day: 0, rate: cfg.clock.minutes_per_second / 60.0, here, wind: Vec3::ZERO, wind_local: Vec3::ZERO, wind_mean_mps: cfg.wind.speed_mps, map_shift, air, stars: None, cfg: cfg.clone(), map })
        .add_systems(PostStartup, setup)
        .add_systems(Update, (tick.in_set(crate::NephtheneSet::Tick), stars_follow_atmosphere))
        .add_systems(PostUpdate, ambient_follows_exposure);
}

pub(crate) fn setup(mut commands: Commands, mut wx: ResMut<Weather>, mut media: ResMut<Assets<ScatteringMedium>>, mut images: ResMut<Assets<Image>>) {
    let s = &wx.cfg.sky;
    let mut m = ScatteringMedium::earth(256, 256).with_label("nephthene_atmosphere");
    // earth(): [Rayleigh, Mie, ozone]; the Mie term is the dust.
    m.terms[1] = ScatteringTerm {
        absorption: Vec3::splat(s.mie_absorption), scattering: Vec3::splat(s.mie_scattering),
        falloff: Falloff::Exponential { scale: s.mie_scale_height_km / 60.0 }, phase: PhaseFunction::Mie { asymmetry: s.mie_asymmetry },
    };
    // ponytail: 256x256 LUT size is Bevy's own default for the earth medium.
    commands.spawn(Atmosphere::earth(media.add(m)));
    let mo = &wx.cfg.moon;
    commands.spawn((Name::new("Moon"), Moon, DirectionalLight { illuminance: 0.0, shadow_maps_enabled: false, ..default() },
        SunDisk { angular_size: mo.angular_size_deg.to_radians(), intensity: 1.0 }));
    wx.stars = Some(images.add(star_cubemap(&wx.cfg.stars)));
    let (el, az) = w::sun(&wx.cfg.clock, wx.hour);
    println!("WEATHER hour {:.2} sun el {el:.1} az {az:.1} ev100 {:.2}", wx.hour, w::curve(&wx.cfg.exposure, el));
    println!("MAP shift ({:.0}, {:.0}) m advect {} wrap {} here coverage {:.3}", wx.map_shift.x, wx.map_shift.y, wx.cfg.map.advect, wx.cfg.map.wrap, wx.here.coverage);
    println!("AIR temp {:.1} C dew {:.1} C rh {:.2} freezing {:.0} m snow line {:.0} m", wx.air.temp_c, wx.air.dew_c, wx.air.rh, wx.air.freezing_m, wx.air.snow_line_m);
}

/// Look at a body: a light at `to` shines along -to.
fn facing(to: Vec3) -> Transform { Transform::IDENTITY.looking_to(-to, if to.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y }) }

pub(crate) fn tick(
    time: Res<Time>, mut wx: ResMut<Weather>, cam: Query<&GlobalTransform, With<AtmosphereSettings>>,
    mut sun: Query<(&mut DirectionalLight, &mut Transform), (With<NephtheneSun>, Without<Moon>)>, mut moon: Query<(&mut DirectionalLight, &mut Transform), With<Moon>>,
    mut exposure: Query<&mut Exposure>, mut fog: Query<&mut DistanceFog>, mut sky: Query<&mut Skybox>, rain: Option<Res<crate::rain::RainState>>,
) {
    let dt = time.delta_secs();
    let h0 = wx.hour;
    let h = wx.hour + dt * wx.rate;
    if h >= 24.0 { wx.day += 1; }
    wx.hour = h.rem_euclid(24.0);
    // The map rides last frame's mean wind (panel override included) on the game clock: frozen when the clock is.
    let mean = w::map_wind(wx.wind, wx.wind_mean_mps);
    let shift = w::advect(&wx.cfg.map, &wx.map, wx.map_shift, mean, dt * wx.rate * 3600.0);
    wx.map_shift = shift;
    wx.wind = w::wind(&wx.cfg.wind, time.elapsed_secs());
    wx.wind_local = wx.wind;
    wx.wind_mean_mps = wx.cfg.wind.speed_mps;
    if let Some(p) = cam.iter().next().map(|g| g.translation()) {
        let [x, z] = w::map_xz(&wx.cfg.map, wx.map_shift, p.x, p.z);
        let target = wx.map.sample(x, z);
        wx.here = w::smooth(wx.here, target, dt, wx.cfg.sim.here_tau_s);
    }
    // The air rides the game clock with the cover overhead (last frame's ground wetness); a frozen clock spins it up for the hour.
    let (wet, cov, dust) = (rain.map_or(0.0, |r| r.wetness), wx.here.coverage, wx.here.dust);
    let t = if wx.rate > 0.0 { air::step(&wx.cfg.air, &wx.cfg.clock, wx.air.temp_c, h0, dt * wx.rate, cov) } else { air::spin_up(&wx.cfg.air, &wx.cfg.clock, wx.hour, cov) };
    wx.air = air::state(&wx.cfg.air, &wx.cfg.clock, t, wet, dust);
    let (d, hour, here) = (&wx.cfg, wx.hour, wx.here);
    // No fade below the horizon: the Atmosphere occludes the disc for surfaces, and twilight needs the light to scatter.
    let (el, az) = w::sun(&d.clock, hour);
    for (mut l, mut t) in &mut sun {
        l.illuminance = lux::RAW_SUNLIGHT;
        *t = facing(w::dir(el, az));
    }
    let (mel, maz) = w::moon(&d.clock, &d.moon, hour);
    for (mut l, mut t) in &mut moon {
        (l.illuminance, l.color) = (w::moon_lux(&d.moon), Color::srgb(d.moon.color[0], d.moon.color[1], d.moon.color[2]));
        *t = facing(w::dir(mel, maz));
    }
    let ev = w::curve(&d.exposure, el);
    for mut e in &mut exposure { e.ev100 = ev; }
    let (f, b, k) = (&d.fog, w::curve(&d.fog_brightness, el), here.dust);
    let mix = |c: [f32; 3], s: f32| { let v = Vec3::from(c).lerp(Vec3::from(c) * Vec3::from(f.dust_tint), k) * s; Color::srgb(v.x, v.y, v.z) };
    for mut g in &mut fog {
        g.color = mix(f.color, b);
        g.falloff = FogFalloff::from_visibility_colors(f.visibility_m + (f.dust_visibility_m - f.visibility_m) * k, mix(f.extinction, 1.0), mix(f.inscattering, 1.0));
    }
    // Stars turn about the celestial pole (north, raised by the latitude). ponytail: solar day (15 deg/h), not sidereal.
    let pole = w::dir(d.clock.latitude_deg, 0.0);
    for mut s in &mut sky { s.rotation = Quat::from_axis_angle(pole, (15.0 * hour).to_radians()); }
}

/// Ambient light is authored for daylight (`ambient_day_brightness`). Scaled by the exposure drop from `daylight_ev100`,
/// it stays put by day and stops flooding the night.
fn ambient_follows_exposure(wx: Res<Weather>, mut global: ResMut<GlobalAmbientLight>, mut cams: Query<(&mut AmbientLight, &Exposure)>) {
    let s = |ev: f32| 2f32.powf((ev - wx.cfg.daylight_ev100).min(0.0));
    global.brightness = wx.cfg.ambient_day_brightness * s(w::curve(&wx.cfg.exposure, w::sun(&wx.cfg.clock, wx.hour).0));
    for (mut a, e) in &mut cams { a.brightness *= s(e.ev100); }
}

/// Stars only on cameras that draw the Atmosphere; without it nothing would attenuate them by day.
fn stars_follow_atmosphere(mut commands: Commands, wx: Res<Weather>, add: Query<Entity, (With<AtmosphereSettings>, Without<Skybox>)>, drop: Query<Entity, (With<Skybox>, Without<AtmosphereSettings>)>) {
    let Some(img) = &wx.stars else { return };
    for e in &add { commands.entity(e).insert(Skybox { image: Some(img.clone()), brightness: wx.cfg.stars.brightness, rotation: Quat::IDENTITY }); }
    for e in &drop { commands.entity(e).remove::<Skybox>(); }
}

/// Black cubemap with `count` single-texel stars, uniform over the sphere, brightness skewed so few are bright.
fn star_cubemap(st: &nephthene_core::Stars) -> Image {
    let n = st.face_px as usize;
    let mut data = vec![0u8; 6 * n * n * 4];
    for i in 0..data.len() / 4 { data[i * 4 + 3] = 255; }
    let mut h = st.seed.wrapping_mul(2654435761) | 1;
    let mut rnd = || { h ^= h << 13; h ^= h >> 17; h ^= h << 5; h as f32 / u32::MAX as f32 };
    for _ in 0..st.count {
        let (z, a) = (rnd() * 2.0 - 1.0, rnd() * std::f32::consts::TAU);
        let r = (1.0 - z * z).sqrt();
        let v = Vec3::new(r * a.cos(), z, r * a.sin());
        let (b, warm) = (rnd().powf(st.skew), rnd());
        let m = v.abs().max_element();
        // Face order +X -X +Y -Y +Z -Z, (s, t) per the cubemap convention.
        let (face, s, t) = if m == v.x.abs() { if v.x > 0.0 { (0, -v.z, -v.y) } else { (1, v.z, -v.y) } }
            else if m == v.y.abs() { if v.y > 0.0 { (2, v.x, v.z) } else { (3, v.x, -v.z) } }
            else if v.z > 0.0 { (4, v.x, -v.y) } else { (5, -v.x, -v.y) };
        let px = |c: f32| (((c / m + 1.0) * 0.5 * n as f32) as usize).min(n - 1);
        let o = (face * n * n + px(t) * n + px(s)) * 4;
        // ponytail: warm/cool tint ranges of a star are fixed.
        let c = Vec3::new(1.0, 0.92 + 0.08 * warm, 0.8 + 0.2 * (1.0 - warm)) * (st.floor + (1.0 - st.floor) * b);
        data[o..o + 3].copy_from_slice(&[(c.x * 255.0) as u8, (c.y * 255.0) as u8, (c.z * 255.0) as u8]);
    }
    let mut img = Image::new(Extent3d { width: n as u32, height: n as u32, depth_or_array_layers: 6 }, TextureDimension::D2, data, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
    img.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });
    img
}
