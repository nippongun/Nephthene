//! Settings list for the weather panel: which config leaves a slider may change at runtime, in what range, and saving them. Pure; the
//! Bevy adapter is the `panel` module of the plugin crate.
use crate::config::Config;
use serde_json::{Map, Value};

/// Config leaves whose runtime change reaches the picture or the simulation. Ends in `.`: a whole section. Everything else is read once
/// at startup, so a slider for it would do nothing (or, like `clouds.shadow_half_m`, misplace what it feeds).
/// ponytail: hand list from a code audit; most of `clouds.*` is missing until the cloud uniforms are rebuilt on a config change.
const LIVE: &[&str] = &[
    "rays.", "ground_fog.", "fog.", "wind.", "climate.", "daylight_ev100", "ambient_day_brightness",
    "clock.latitude_deg", "clock.day_of_year", "clock.axial_tilt_deg", "moon.phase", "moon.full_lux",
    "sky.sun_occlusion", "sky.occlusion_keep", "sky.occlusion_steps", "sky.cloud_shafts", "sky.cloud_shafts_ignore_clouds",
    "map.advect", "clouds.wind_scale", "clouds.shadow_every", "clouds.shadow_samples", "clouds.shadow_step_m", "clouds.shadow_samples_max",
    "clouds.edge_lod.enabled", "clouds.edge_lod.step_scale", "resolve.cut_deg", "resolve.cut_m",
    "shafts.shear_frac", "shafts.slant_tau_s", "shafts.lean_intensity", "shafts.gust_lean",
    "air.mean_c", "air.season_c", "air.lag_days", "air.heat_k_per_h", "air.cool_k_per_h", "air.restore_per_h", "air.cloud_shade", "air.cloud_block",
    "air.dew_below_c", "air.wet_gain_c", "air.dust_dry_c", "air.ref_alt_m", "air.lapse_k_per_km", "air.dew_lapse_k_per_km", "air.step_h",
    "orographic.enabled", "orographic.cover_weight", "orographic.type_weight", "orographic.rebake_m", "orographic.rebake_deg", "orographic.rebake_frac", "orographic.rebake_k",
    "sim.here_tau_s",
    "rain.min_intensity", "rain.wind_drift", "rain.wind_drift_max", "rain.fall_min_mps", "rain.fall_gain_mps", "rain.shutter_s", "rain.wet_up_per_s", "rain.dry_per_s",
    "rain.wet_threshold", "rain.light_gain", "rain.sun_share", "rain.sky_share", "rain.night_floor", "rain.cloud_dim", "rain.level_scale",
    "rain.snow_fall_mps", "rain.snow_drift", "rain.snow_drift_max", "rain.flake_m", "rain.flake_alpha", "rain.flutter_m", "rain.flutter_hz", "rain.snow_half_m",
    "rain.cover_up_per_s", "rain.melt_per_s", "rain.melt_c",
];
/// Live, but left out: lo/hi pairs that give NaN when a slider makes them equal, a test switch, and a bake size that stalls a frame.
const EXCLUDED: &[&str] = &["rays.flat_source", "ground_fog.wind_lo_mps", "ground_fog.wind_hi_mps", "ground_fog.spread_full_c", "ground_fog.spread_none_c", "ground_fog.bake_px"];
/// Sensible ranges where the rule in `range` does not fit (untuned).
const RANGES: &[(&str, f32, f32)] = &[
    ("rays.strength", 0.0, 40.0), ("rays.threshold", 0.0, 4.0), ("rays.source_max", 0.1, 8.0), ("rays.radius", 0.05, 1.0), ("rays.decay", 0.8, 1.0), ("rays.mean_width", 0.005, 0.2),
    ("clock.latitude_deg", -90.0, 90.0), ("clock.day_of_year", 1.0, 365.0), ("clock.axial_tilt_deg", 0.0, 45.0), ("wind.from_deg", 0.0, 360.0), ("moon.phase", 0.0, 1.0),
];
/// Panel order and titles of the sections; a section not listed sorts last under its own name.
const SECTIONS: &[(&str, &str)] = &[("rays", "God rays"), ("ground_fog", "Ground fog (all)"), ("fog", "Distance haze"), ("sky", "Sky"), ("clouds", "Clouds (live part)"), ("shafts", "Rain shafts"),
    ("rain", "Rain and snow"), ("wind", "Wind"), ("air", "Air"), ("climate", "Climate"), ("orographic", "Terrain uplift"), ("clock", "Sun path"), ("moon", "Moon"), ("map", "Map"), ("resolve", "Fog history"), ("sim", "Smoothing"), ("misc", "Light")];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind { Float, Int, Bool }
/// One settings row.
#[derive(Clone, PartialEq, Debug)]
pub struct Knob { pub path: String, pub kind: Kind, pub min: f32, pub max: f32 }

fn live(path: &str) -> bool { !EXCLUDED.contains(&path) && LIVE.iter().any(|p| *p == path || (p.ends_with('.') && path.starts_with(p))) }

/// Slider range round a default. Never reaches 0 from a non-zero default (many of these divide) and never leaves 0..1 from a default
/// inside it (blend weights feed back above 1). Zero defaults get 0..1.
fn range(path: &str, d: f32) -> (f32, f32) {
    if let Some((_, lo, hi)) = RANGES.iter().find(|r| r.0 == path) { return (*lo, *hi); }
    if d == 0.0 { (0.0, 1.0) } else if d < 0.0 { (d * 4.0, d / 4.0) } else if d <= 1.0 { (d / 4.0, 1.0) } else { (d / 4.0, d * 4.0) }
}

/// Every row, in panel order (section, then path).
pub fn knobs() -> Vec<Knob> {
    let mut out: Vec<Knob> = Config::paths().into_iter().filter(|(p, _)| live(p)).filter_map(|(path, json)| {
        let v: Value = serde_json::from_str(&json).ok()?;
        let (kind, min, max) = match &v {
            Value::Bool(_) => (Kind::Bool, 0.0, 1.0),
            Value::Number(n) if n.is_f64() => { let (a, b) = range(&path, n.as_f64()? as f32); (Kind::Float, a, b) }
            Value::Number(n) => { let d = n.as_f64()? as f32; let (a, b) = range(&path, d); (Kind::Int, a.ceil().max(d.min(1.0)), b.ceil()) }
            _ => return None, // arrays, strings and unset options have no slider
        };
        Some(Knob { path, kind, min, max })
    }).collect();
    out.sort_by_key(|k| (order(section(&k.path)), k.path.clone()));
    out
}
fn order(s: &str) -> usize { SECTIONS.iter().position(|x| x.0 == s).unwrap_or(SECTIONS.len()) }

/// Section key of a path; a top-level leaf is `misc`.
pub fn section(path: &str) -> &str { path.split_once('.').map_or("misc", |x| x.0) }
/// Panel title of a section key.
pub fn title(s: &str) -> &str { SECTIONS.iter().find(|x| x.0 == s).map_or(s, |x| x.1) }
/// Row label: the path without its section.
pub fn label(path: &str) -> &str { path.split_once('.').map_or(path, |x| x.1) }
/// Section keys that have rows, in panel order.
pub fn sections(list: &[Knob]) -> Vec<String> { let mut v: Vec<String> = list.iter().map(|k| section(&k.path).to_string()).collect(); v.dedup(); v }

impl Knob {
    /// Slider decimals: two significant digits over the range.
    pub fn precision(&self) -> i32 { if self.kind != Kind::Float { 0 } else { (2.0 - (self.max - self.min).max(1e-6).log10().floor()).clamp(0.0, 6.0) as i32 } }
    /// The leaf in `v` (from `value`); a bool reads 0 or 1.
    pub fn read(&self, v: &Value) -> Option<f32> {
        let leaf = self.path.split('.').try_fold(v, |c, k| c.get(k))?;
        leaf.as_bool().map(|b| b as u32 as f32).or(leaf.as_f64().map(|x| x as f32))
    }
    /// Set the leaf, clamped to the range.
    pub fn write(&self, c: &mut Config, v: f32) -> Result<(), String> {
        let v = v.clamp(self.min, self.max);
        let json = match self.kind { Kind::Bool => (v > 0.5).to_string(), Kind::Int => (v.round() as i64).to_string(), Kind::Float => v.to_string() };
        c.set_path(&self.path, &json)
    }
}

/// The config as a JSON tree, for `Knob::read`.
pub fn value(c: &Config) -> Value { serde_json::to_value(c).expect("config serialises") }

fn diff(v: &Value, base: &Value) -> Option<Value> {
    match (v, base) {
        (Value::Object(a), Value::Object(b)) => {
            let m: Map<String, Value> = a.iter().filter_map(|(k, x)| b.get(k).map_or(Some(x.clone()), |y| diff(x, y)).map(|d| (k.clone(), d))).collect();
            (!m.is_empty()).then_some(Value::Object(m))
        }
        _ => (v != base).then(|| v.clone()),
    }
}
/// What differs from the default config, as JSON. `Config::load` reads it back: missing keys take their defaults.
pub fn diff_json(c: &Config) -> String {
    let d = diff(&value(c), &value(&Config::default())).unwrap_or(Value::Object(Map::new()));
    serde_json::to_string_pretty(&d).expect("json")
}

/// `cur` with every knob taken from `file` (clamped to its range). Startup leaves stay: they apply when the game loads the file at launch.
/// Returns the config and how many knobs changed. On an error `cur` is the result so far.
pub fn merge_live(cur: &Config, file: &Config) -> (Config, usize) {
    let (mut out, v) = (cur.clone(), value(file));
    let before = value(cur);
    let mut n = 0;
    for k in knobs() {
        if let Some(x) = k.read(&v) && k.read(&before) != Some(x) && k.write(&mut out, x).is_ok() { n += 1; }
    }
    (out, n)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_live_entry_and_range_names_a_real_leaf() {
        let paths = Config::paths();
        for p in LIVE { assert!(paths.iter().any(|(x, _)| x == p || (p.ends_with('.') && x.starts_with(p))), "LIVE {p}: no such leaf"); }
        let list = knobs();
        for (p, _, _) in RANGES { assert!(list.iter().any(|k| k.path == *p), "RANGES {p}: no such knob"); }
        for p in EXCLUDED { assert!(paths.iter().any(|(x, _)| x == p), "EXCLUDED {p}: no such leaf"); }
    }
    #[test]
    fn defaults_sit_in_their_ranges_and_positive_defaults_cannot_reach_zero() {
        let v = value(&Config::default());
        for k in knobs() {
            let d = k.read(&v).unwrap();
            assert!(k.min <= d && d <= k.max, "{k:?} default {d}");
            if d != 0.0 && !RANGES.iter().any(|r| r.0 == k.path) && k.kind != Kind::Bool { assert!(k.min * k.max > 0.0, "{k:?} spans 0"); }
        }
    }
    #[test]
    fn startup_leaves_and_nan_pairs_get_no_row() {
        let list = knobs();
        for p in ["clouds.shadow_half_m", "clouds.shadow_res", "clouds.top_m", "clouds.look.albedo", "stars.brightness", "rain.precip_lo", "rain.force", "ground_fog.wind_lo_mps"] {
            assert!(!list.iter().any(|k| k.path == p), "{p} must not be a knob");
        }
        assert!(list.iter().any(|k| k.path == "rays.strength"));
    }
    #[test]
    fn write_read_round_trip_clamps_and_keeps_types() {
        let list = knobs();
        let k = |p: &str| list.iter().find(|x| x.path == p).unwrap().clone();
        let mut c = Config::default();
        k("rays.strength").write(&mut c, 3.5).unwrap();
        k("rays.samples").write(&mut c, 40.2).unwrap();
        k("rays.enabled").write(&mut c, 0.0).unwrap();
        k("rays.decay").write(&mut c, 9.0).unwrap();
        assert_eq!((c.rays.strength, c.rays.samples, c.rays.enabled, c.rays.decay), (3.5, 40, false, 1.0));
        assert_eq!(k("rays.strength").read(&value(&c)), Some(3.5));
    }
    #[test]
    fn diff_is_empty_at_defaults_and_round_trips() {
        assert_eq!(diff_json(&Config::default()), "{}");
        let mut c = Config::default();
        c.rays.strength = 3.0;
        c.clouds.coverage_override = Some(0.4);
        let s = diff_json(&c);
        assert!(s.contains("strength") && !s.contains("threshold"), "{s}");
        assert_eq!(Config::from_json(&s).unwrap(), c);
    }
    #[test]
    fn merge_takes_knobs_clamps_them_and_leaves_startup_values() {
        let mut f = Config::default();
        f.rays.strength = 1e9;
        f.clouds.shadow_half_m = 1.0;
        let (out, n) = merge_live(&Config::default(), &f);
        assert_eq!((out.rays.strength, out.clouds.shadow_half_m, n), (40.0, Config::default().clouds.shadow_half_m, 1));
    }
}
