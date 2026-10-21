//! Weather panel, the pure half (state + maths; the Bevy adapter is the `panel` module of the plugin crate). Runtime-only
//! overrides on top of the config: nothing here is saved and every default is "no override". Step sizes are untuned.
use crate::{Wind, WeatherMap};

/// Which weather-map channel the panel map shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Channel { Coverage, Rain, Type, Dust, Rgb }
impl Channel {
    pub const ALL: [Channel; 5] = [Self::Coverage, Self::Rain, Self::Type, Self::Dust, Self::Rgb];
    pub fn label(self) -> &'static str { match self { Self::Coverage => "coverage", Self::Rain => "rain", Self::Type => "cloud type", Self::Dust => "dust", Self::Rgb => "RGB" } }
    pub fn index(self) -> usize { Self::ALL.iter().position(|c| *c == self).unwrap_or(0) }
    /// Display pixel (opaque RGBA8) for a map texel: RGB shows the three colour channels, the others that channel as grey.
    pub fn pixel(self, t: [u8; 4]) -> [u8; 4] {
        match self { Self::Rgb => [t[0], t[1], t[2], 255], Self::Coverage => [t[0], t[0], t[0], 255], Self::Rain => [t[1], t[1], t[1], 255],
            Self::Type => [t[2], t[2], t[2], 255], Self::Dust => [t[3], t[3], t[3], 255] }
    }
    /// The channel's value (0..1) in a map texel, for the hover readout; RGB reads out all three.
    pub fn read(self, t: [u8; 4]) -> String {
        let f = |b: u8| b as f32 / 255.0;
        match self { Self::Rgb => format!("coverage {:.2}  rain {:.2}  type {:.2}", f(t[0]), f(t[1]), f(t[2])), Self::Coverage => format!("coverage {:.2}", f(t[0])),
            Self::Rain => format!("rain {:.2}", f(t[1])), Self::Type => format!("cloud type {:.2}", f(t[2])), Self::Dust => format!("dust {:.2}", f(t[3])) }
    }
}

/// The whole map recoloured for display.
pub fn channel_rgba(map: &WeatherMap, ch: Channel) -> Vec<u8> { map.rgba.chunks_exact(4).flat_map(|t| ch.pixel([t[0], t[1], t[2], t[3]])).collect() }

/// A map texel as the renderer samples it: R is the coverage override and G the precip override when set (the values `apply` hands the shader).
pub fn effective_texel(map: &WeatherMap, ctl: &Ctl, i: u32, j: u32) -> [u8; 4] {
    let k = ((j * map.width + i) * 4) as usize;
    let mut t: [u8; 4] = std::array::from_fn(|c| map.rgba[k + c]);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let (cov, rain) = (ctl.gpu_coverage(), ctl.gpu_precip());
    if cov >= 0.0 { t[0] = byte(cov); }
    if rain >= 0.0 { t[1] = byte(rain); }
    t
}

/// The whole map recoloured for display with the overrides applied; same layout as `channel_rgba`.
pub fn effective_rgba(map: &WeatherMap, ctl: &Ctl) -> Vec<u8> {
    (0..map.height).flat_map(|j| (0..map.width).map(move |i| (i, j)))
        .flat_map(|(i, j)| ctl.channel.pixel(effective_texel(map, ctl, i, j))).collect()
}

/// Map (x, z) metres -> (u, v) in 0..1 across the map image, v down from the north edge (rows run north to south); unclamped.
pub fn map_uv(map: &WeatherMap, x: f32, z: f32) -> [f32; 2] { [(x - map.min_x) / map.size_x, (map.max_z - z) / map.size_z] }

/// Texel (column, row) containing map (x, z), clamped onto the map.
pub fn texel_at(map: &WeatherMap, x: f32, z: f32) -> [u32; 2] {
    let [u, v] = map_uv(map, x, z);
    [((u * map.width as f32).floor().max(0.0) as u32).min(map.width - 1), ((v * map.height as f32).floor().max(0.0) as u32).min(map.height - 1)]
}

/// Map bounds of a texel: (min_x, max_x, min_z, max_z).
pub fn texel_bounds(map: &WeatherMap, i: u32, j: u32) -> [f32; 4] {
    let (w, h) = (map.size_x / map.width as f32, map.size_z / map.height as f32);
    let (x0, z1) = (map.min_x + i as f32 * w, map.max_z - j as f32 * h);
    [x0, x0 + w, z1 - h, z1]
}

/// Compass azimuth (deg clockwise from north = +Z) of a world direction's horizontal part.
pub fn heading_deg(d: [f32; 3]) -> f32 { d[0].atan2(d[2]).to_degrees().rem_euclid(360.0) + 0.0 } // + 0.0 turns -0 into 0

/// Clock-rate multipliers on the config rate: pause, 1x, 10x, 60x (untuned).
pub const RATES: [f32; 4] = [0.0, 1.0, 10.0, 60.0];
pub const RATE_NAMES: [&str; 4] = ["paused", "1x (config rate)", "10x", "60x"];
/// Index into `RATES` nearest `mult`.
pub fn rate_index(mult: f32) -> usize { (0..RATES.len()).min_by(|&a, &b| (RATES[a] - mult).abs().total_cmp(&(RATES[b] - mult).abs())).unwrap_or(1) }
/// "HH:MM" of an hour 0..24.
pub fn hhmm(h: f32) -> String { let m = (h.rem_euclid(24.0) * 60.0).floor() as u32 % 1440; format!("{:02}:{:02}", m / 60, m % 60) }

/// Runtime wind override: either part may be set; the config value fills the rest.
pub fn wind_with(base: &Wind, from_deg: Option<f32>, speed: Option<f32>) -> Wind {
    Wind { from_deg: from_deg.unwrap_or(base.from_deg), speed_mps: speed.unwrap_or(base.speed_mps), ..base.clone() }
}
/// Wind direction wrapped 0..360, and speed clamped 0..30 m/s (untuned), for the number inputs.
pub fn wrap_deg(d: f32) -> f32 { d.rem_euclid(360.0) }
pub fn clamp_speed(s: f32) -> f32 { s.clamp(0.0, 30.0) }

/// Precip override: the rain toggle forces full precip.
pub const RAIN_PRECIP: f32 = 1.0;
pub fn precip_override(rain: bool) -> Option<f32> { rain.then_some(RAIN_PRECIP) }

/// Cloud debug views (debug modes 0..=5), named in words.
pub const DEBUG_NAMES: [&str; 6] = ["off (normal look)", "exposed max channel", "direct light", "sky ambient", "cloud alpha", "cloud depth"];

/// Gizmo legend: word label and colour (sRGB) of each gizmo group. The checkbox swatch and the drawn lines use the same colour.
pub const GIZMOS: [(&str, [f32; 3]); 3] = [("sun direction", [1.0, 0.9, 0.2]), ("wind / cloud drift", [0.3, 1.0, 0.5]), ("cloud-shadow area", [1.0, 0.5, 0.1])];

/// Everything the panel changes at runtime. Default = no override, panel closed.
#[derive(Clone, Debug, PartialEq)]
pub struct Ctl {
    pub open: bool, pub channel: Channel,
    pub coverage: Option<f32>, pub wind_from: Option<f32>, pub wind_speed: Option<f32>, pub rain: bool,
    pub clouds: bool, pub shadows: bool, pub debug: u32, pub gizmos: [bool; 3],
    /// Map (x, z) pinned by a map click.
    pub pin: Option<[f32; 2]>,
}
impl Default for Ctl {
    fn default() -> Self { Ctl { open: false, channel: Channel::Rgb, coverage: None, wind_from: None, wind_speed: None, rain: false, clouds: true, shadows: true, debug: 0, gizmos: [true, true, false], pin: None } }
}
impl Ctl {
    /// [Reset all]: weather back to the map / config (wind, coverage, rain). Clock, view and gizmos stay.
    pub fn reset_weather(&mut self) { (self.wind_from, self.wind_speed, self.coverage, self.rain) = (None, None, None, false); }
    /// Coverage the cloud shader should use: -1 = the map. Clouds off forces 0.
    pub fn gpu_coverage(&self) -> f32 { if !self.clouds { 0.0 } else { self.coverage.unwrap_or(-1.0) } }
    /// Precip the shader should use: -1 = the map.
    pub fn gpu_precip(&self) -> f32 { precip_override(self.rain).unwrap_or(-1.0) }
    /// Shadow strength multiplier source: the config value, or 0 with shadows off.
    pub fn shadow_strength(&self, json: f32) -> f32 { if self.shadows { json } else { 0.0 } }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn map() -> WeatherMap {
        // 2 x 2 texels, 100 m each, NW corner at (-100, 100): rows north to south.
        WeatherMap { width: 2, height: 2, min_x: -100.0, max_z: 100.0, size_x: 200.0, size_z: 200.0,
            rgba: vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160] }
    }
    #[test]
    fn channels_pick_their_byte() {
        let m = map();
        assert_eq!(Channel::Rain.pixel([10, 20, 30, 40]), [20, 20, 20, 255]);
        assert_eq!(Channel::Dust.pixel([10, 20, 30, 40]), [40, 40, 40, 255]);
        assert_eq!(&channel_rgba(&m, Channel::Coverage)[4..8], &[50, 50, 50, 255]);
        assert_eq!(Channel::Rgb.index(), 4);
        assert_eq!(Channel::Rain.read([0, 255, 0, 0]), "rain 1.00");
    }
    #[test]
    fn map_is_north_up() {
        let m = map();
        assert_eq!(map_uv(&m, -100.0, 100.0), [0.0, 0.0]);
        assert_eq!(map_uv(&m, 100.0, -100.0), [1.0, 1.0]);
        assert_eq!(texel_at(&m, 50.0, 50.0), [1, 0]);
        assert_eq!(texel_at(&m, 5000.0, -5000.0), [1, 1]);
        assert_eq!(texel_bounds(&m, 1, 0), [0.0, 100.0, 0.0, 100.0]);
    }
    #[test]
    fn compass_heading() {
        assert!((heading_deg([0.0, 0.0, 1.0]) - 0.0).abs() < 1e-4 && (heading_deg([1.0, 0.0, 0.0]) - 90.0).abs() < 1e-4);
        assert!((heading_deg([-1.0, 0.0, 0.0]) - 270.0).abs() < 1e-4);
    }
    #[test]
    fn clock_and_rates() {
        assert_eq!((hhmm(17.5), hhmm(24.0), hhmm(0.25), hhmm(23.999)), ("17:30".into(), "00:00".into(), "00:15".into(), "23:59".into()));
        assert_eq!((rate_index(0.0), rate_index(1.0), rate_index(9.0), rate_index(60.0)), (0, 1, 2, 3));
    }
    #[test]
    fn wind_override_keeps_the_rest() {
        let b = Wind::default();
        let w = wind_with(&b, None, Some(12.0));
        assert_eq!((w.from_deg, w.speed_mps, w.gust), (225.0, 12.0, 0.5));
        assert_eq!((wrap_deg(-30.0), wrap_deg(390.0), clamp_speed(-1.0), clamp_speed(99.0)), (330.0, 30.0, 0.0, 30.0));
    }
    #[test]
    fn reset_clears_only_the_weather() {
        let mut c = Ctl { coverage: Some(0.4), wind_speed: Some(9.0), rain: true, clouds: false, ..Ctl::default() };
        c.reset_weather();
        assert_eq!((c.coverage, c.wind_speed, c.rain, c.clouds), (None, None, false, false));
    }
    #[test]
    fn overrides_reach_the_shader_values() {
        let mut c = Ctl::default();
        assert_eq!((c.gpu_coverage(), c.gpu_precip(), c.shadow_strength(1.0)), (-1.0, -1.0, 1.0));
        c.coverage = Some(0.5); c.rain = true;
        assert_eq!((c.gpu_coverage(), c.gpu_precip()), (0.5, RAIN_PRECIP));
        c.clouds = false; c.shadows = false;
        assert_eq!((c.gpu_coverage(), c.shadow_strength(1.0)), (0.0, 0.0));
        assert_eq!(DEBUG_NAMES.len(), 6);
    }
    #[test]
    fn panel_map_shows_the_effective_weather() {
        let m = WeatherMap::procedural(&crate::config::MapCfg::default());
        let mut c = Ctl { channel: Channel::Coverage, ..Ctl::default() };
        // Red arm: the old recolour ignores the coverage override, so it must stay the same.
        let old = channel_rgba(&m, Channel::Coverage);
        let before = effective_rgba(&m, &c);
        c.coverage = Some(0.9);
        assert_eq!(channel_rgba(&m, Channel::Coverage), old);
        let after = effective_rgba(&m, &c);
        assert_ne!(before, after);
        let v = after[0];
        assert!((229..=230).contains(&v) && after.chunks_exact(4).all(|p| p == [v, v, v, 255]));
        // Clouds off forces zero coverage, whatever the override says.
        c.clouds = false;
        assert!(effective_rgba(&m, &c).chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
        // Rain toggle forces full precip on the rain channel, and off gives the map back.
        c.clouds = true; c.coverage = None; c.channel = Channel::Rain;
        let rain_before = effective_rgba(&m, &c);
        c.rain = true;
        assert_ne!(effective_rgba(&m, &c), rain_before);
        assert!(effective_rgba(&m, &c).chunks_exact(4).all(|p| p == [255, 255, 255, 255]));
        c.rain = false;
        assert_eq!(effective_rgba(&m, &c), rain_before);
    }
}
