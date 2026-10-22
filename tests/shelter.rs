//! Shelter gate (headless; every shot is the `sky` example in `--shot` mode). Manual: `cargo test --release --test shelter --
//! --ignored --nocapture --test-threads=1`. G28: no rain falls under a roof. Every threshold is a budget with a stated reason; metrics print before asserting.
mod common;
use common::*;
use image::RgbImage;

/// Looks east at ground level from under the `--roof` slab (the example's 200 x 200 m roof 4 m up), 17.5 h, with no wind (the rain
/// streaks hang vertical), plus `sets`. The ground under the roof still gets wet (a ponytail in src/rain.rs) and wetness feeds the
/// fog, so the gate holds the ground material and the fog fixed: `rain.wet_epsilon` 2.0 keeps the ground dry, `ground_fog` off.
fn shot(label: &str, sets: &[&str]) -> RgbImage {
    let mut args = vec!["--roof", "--hour", "17.5", "--timeout", "900", "--set", "wind.speed_mps=0.0", "--set", "ground_fog.enabled=false", "--set", "rain.wet_epsilon=2.0"];
    for s in sets { args.extend(["--set", *s]); }
    read(&shoot(label, "rain_near", &args), "rain_near")
}

/// Share of all pixels whose luma differs by more than 2.0 levels between two frames (the measure of tests/scenes.rs `share`, over the
/// whole frame: that gate keeps to the sky rows because wet ground would count there; here `shot` pins the ground and the fog, and the
/// rain under a roof falls in front of the roof underside above the horizon and onto the floor below it).
fn share(a: &RgbImage, b: &RgbImage) -> f32 {
    let n = a.height() * a.width();
    let k = (0..a.height()).flat_map(|y| (0..a.width()).map(move |x| (x, y)))
        .filter(|&(x, y)| (luma(a.get_pixel(x, y)) - luma(b.get_pixel(x, y))).abs() > 2.0).count();
    k as f32 / n as f32
}

/// The camera stands 2.6 m under a 200 m roof and the rain window reaches 40 m, so no drop is in view; what is left must not read as
/// rain. A tenth of the rain scene's 1% budget (tests/scenes.rs RAIN_BUDGET).
const G28_UNDER: f32 = 0.001;
/// The rain scene budget: the red arm's rain under the roof must read, or the metric cannot see rain here.
const G28_RAIN: f32 = 0.01;

#[test]
#[ignore = "manual gate"]
fn g28_no_rain_under_a_roof() {
    let on = shot("g28_on", &["rain.force=1.0"]);
    let dry = shot("g28_dry", &["rain.force=0.0"]);
    let red = shot("g28_red", &["rain.force=1.0", "rain.ignore_shelter=true"]);
    let (s, r) = (share(&on, &dry), share(&red, &dry));
    println!("G28 under the roof: share {s:.4} (budget < {G28_UNDER})");
    println!("G28 red arm (rain.ignore_shelter): share {r:.4} (must be >= {G28_RAIN})");
    let mut f = vec![];
    if s >= G28_UNDER { f.push(format!("G28 under the roof: share {s:.4} >= {G28_UNDER}: rain falls under the roof")); }
    if r < G28_RAIN { f.push(format!("G28 red arm {r:.4} < {G28_RAIN}: rain under the roof does not read, so the metric cannot see it")); }
    finish("G28", f);
}
