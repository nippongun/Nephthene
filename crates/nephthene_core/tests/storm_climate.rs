//! How often storms form at the defaults ("occasionally", owner, 2026-10-22), measured on the built-in map over many game days with
//! the camera fixed at the map origin, through the same timetable, genesis and lifecycle the plugin uses.
use glam::{Vec2, Vec3};
use nephthene_core::{rain::smoothstep, storm::{self, Stage, StormCfg}, storm_fx, Config, WeatherMap};

/// (share of 12-20 h with a mature storm within `near_m` of the camera, storm-rain episodes at the camera, game days, most storms
/// alive in the selection window at once).
pub fn climate(cfg: &Config, days: u32, near_m: f32) -> (f32, u32, u32, usize) {
    let c = StormCfg { enabled: true, ..cfg.storms.clone() };
    let map = WeatherMap::procedural(&cfg.map);
    let (gt, gf) = (c.genesis_type, c.genesis_floor);
    let genesis = |q: Vec2| { let [_, pr, ty, _] = map.rgba(q.x, q.y); smoothstep(gt[0], gt[1], ty) * (gf + (1.0 - gf) * pr) };
    let k = c.clock_scale as f64;
    let end = days as f64 * 86400.0 * k;
    let all = storm::births(&c, &c.seed, &genesis, -(c.cell_life_min[1].max(c.line_life_min[1]) as f64) * 60.0, end, Vec2::ZERO);
    let (mut seen, mut afternoon, mut episodes, mut raining, mut most) = (0u32, 0u32, 0u32, false, 0usize);
    let step_game_s = 600.0;
    for i in 0..(days as f64 * 86400.0 / step_game_s) as u64 {
        let game_s = i as f64 * step_game_s;
        let t = game_s * k;
        let hour = (game_s / 3600.0).rem_euclid(24.0);
        let live: Vec<_> = all.iter().filter_map(|s| storm::now(s, t, &c).map(|n| (s.clone(), n))).collect();
        most = most.max(live.iter().filter(|(_, n)| n.a.length() <= c.window_m).count());
        if (12.0..20.0).contains(&hour) {
            afternoon += 1;
            if live.iter().any(|(_, n)| n.stage == Stage::Mature && (n.a.length() - n.core_r).max(0.0) <= near_m) { seen += 1; }
        }
        let rain = storm_fx::local(&c, &live, Vec3::ZERO).precip > cfg.rain.precip_lo;
        if rain && !raining { episodes += 1; }
        raining = rain;
    }
    (seen as f32 / afternoon as f32, episodes, days, most)
}

// "Occasionally" as budgets: a mature storm within 40 km (a 12 km top stands ~17 deg above the horizon there: plainly visible)
// for 15-40% of the afternoon (12-20 h, the diurnal peak; under 15% most afternoons show none, over 40% storms are the normal
// sky), and storm rain at the camera every 1 to 5 game days (more often is a rainy climate, rarer is never seen in a session:
// a game day is 96 real minutes at the default clock). 60 days keep the counts' sampling error well inside the bands.
const NEAR_M: f32 = 40000.0;
const AFTERNOON: [f32; 2] = [0.15, 0.40];
const RAIN_EVERY_DAYS: [f32; 2] = [1.0, 5.0];
const DAYS: u32 = 60;

fn occasional(cfg: &Config) -> Result<String, String> {
    let (share, episodes, days, most) = climate(cfg, DAYS, NEAR_M);
    let every = days as f32 / episodes.max(1) as f32;
    let msg = format!("rate {:e}: afternoon share {share:.3} (budget {AFTERNOON:?}), storm rain at the camera every {every:.2} days ({episodes} in {days}; budget {RAIN_EVERY_DAYS:?}), most alive in the window {most}", cfg.storms.rate_per_km2_hr);
    let ok = (AFTERNOON[0]..=AFTERNOON[1]).contains(&share) && episodes > 0 && (RAIN_EVERY_DAYS[0]..=RAIN_EVERY_DAYS[1]).contains(&every);
    if ok { Ok(msg) } else { Err(msg) }
}

#[test]
fn storms_form_occasionally_at_the_defaults() {
    let green = occasional(&Config::default());
    println!("{}", green.as_ref().unwrap_or_else(|e| e));
    assert!(green.is_ok(), "defaults: {}", green.unwrap_err());
    // Red arms on both sides: the old rate (storms never seen) and three times the default (storms all the time).
    for rate in [4.0e-5, 3.0 * Config::default().storms.rate_per_km2_hr] {
        let mut cfg = Config::default();
        cfg.storms.rate_per_km2_hr = rate;
        let red = occasional(&cfg);
        println!("red arm {}", red.as_ref().unwrap_or_else(|e| e));
        assert!(red.is_err(), "red arm passes: {}", red.unwrap());
    }
}
