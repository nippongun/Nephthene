//! Wind and rain sound mapping, pure (no audio device): wind speed + gust -> layer gains / pitch / brightness, rain intensity -> bed gains,
//! and the slew that keeps changes click-free. All numbers are untuned starting values (a mix, not car/gun feel).
//! bevy_audio sinks have no filter, so `brightness` is carried by the layer crossfade (calm bed dull, gale bed bright) and the pitch rate.
use std::f32::consts::FRAC_PI_2;

/// Untuned knobs.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct Cfg {
    /// Wind speed (m/s) where the steady bed alone plays and overall gain is 1.0 (the default wind, 6 m/s), and where the gale bed takes over.
    pub steady_mps: f32, pub gale_mps: f32,
    /// Overall wind gain floor at 0 m/s and exponent above `steady_mps`; gain is capped at `gain_max`.
    pub gain_floor: f32, pub gain_exp: f32, pub gain_max: f32,
    /// Playback rate at 0 m/s and at `gale_mps` and above.
    pub rate_calm: f32, pub rate_gale: f32,
    /// A gusty wind (fraction of speed) reads as this much faster for layer choice and gain.
    pub gust_bias: f32,
    /// Rain: intensity below `rain_min` is silent; light patter holds to `rain_light_to`, heavy roar is full from `rain_heavy_from`.
    pub rain_min: f32, pub rain_light_to: f32, pub rain_heavy_from: f32, pub rain_exp: f32,
    /// Slew time constants (s) for the wind speed and the rain intensity feeding the mapping.
    pub wind_tau_s: f32, pub rain_tau_s: f32,
}
impl Default for Cfg {
    fn default() -> Self {
        Cfg { steady_mps: 6.0, gale_mps: 15.0, gain_floor: 0.12, gain_exp: 0.8, gain_max: 2.2, rate_calm: 0.85, rate_gale: 1.25, gust_bias: 0.3,
            rain_min: 0.02, rain_light_to: 0.25, rain_heavy_from: 0.75, rain_exp: 0.8, wind_tau_s: 1.0, rain_tau_s: 1.5 }
    }
}

/// Wind bed mix. `calm` + `steady` + `gale` are equal-power crossfade weights (squares sum to 1); `gain` scales all three.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindMix { pub calm: f32, pub steady: f32, pub gale: f32, pub gain: f32, pub rate: f32, pub brightness: f32 }

/// `speed` m/s (live: the sampled wind, gust swell included, dashboard override included), `gust` = `Wind.gust` fraction of speed.
pub fn wind_mix(c: &Cfg, speed: f32, gust: f32) -> WindMix {
    let s = speed.max(0.0) * (1.0 + c.gust_bias * gust.max(0.0));
    // piecewise equal-power crossfade along calm (0) -> steady -> gale
    let (calm, steady, gale) = if s <= c.steady_mps {
        let f = (s / c.steady_mps).clamp(0.0, 1.0) * FRAC_PI_2; (f.cos(), f.sin(), 0.0)
    } else {
        let f = ((s - c.steady_mps) / (c.gale_mps - c.steady_mps)).clamp(0.0, 1.0) * FRAC_PI_2; (0.0, f.cos(), f.sin())
    };
    let gain = if s <= c.steady_mps { c.gain_floor + (1.0 - c.gain_floor) * (s / c.steady_mps) } else { (s / c.steady_mps).powf(c.gain_exp).min(c.gain_max) };
    let k = (s / c.gale_mps).clamp(0.0, 1.0);
    let rate = c.rate_calm + (c.rate_gale - c.rate_calm) * k;
    let brightness = (calm * calm * 0.0 + steady * steady * 0.5 + gale * gale * 1.0).clamp(0.0, 1.0);
    WindMix { calm, steady, gale, gain, rate, brightness }
}

/// Rain bed mix: light patter and heavy roar crossfade (equal power) over the middle of 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RainMix { pub light: f32, pub heavy: f32, pub gain: f32, pub rate: f32 }
pub fn rain_mix(c: &Cfg, intensity: f32) -> RainMix {
    let i = intensity.clamp(0.0, 1.0);
    if i <= c.rain_min { return RainMix { light: 0.0, heavy: 0.0, gain: 0.0, rate: 1.0 }; }
    let gain = ((i - c.rain_min) / (1.0 - c.rain_min)).powf(c.rain_exp);
    let f = ((i - c.rain_light_to) / (c.rain_heavy_from - c.rain_light_to)).clamp(0.0, 1.0) * FRAC_PI_2;
    RainMix { light: f.cos(), heavy: f.sin(), gain, rate: 0.95 + 0.1 * i }
}

/// Frame-rate independent approach of `cur` toward `target` (time constant `tau_s`): the input to the mappings, so no gain ever steps.
pub fn slew(cur: f32, target: f32, dt: f32, tau_s: f32) -> f32 { cur + (target - cur) * (1.0 - (-dt / tau_s.max(1e-4)).exp()) }

#[cfg(test)]
mod tests {
    use super::*;
    fn c() -> Cfg { Cfg::default() }

    #[test]
    fn wind_louder_brighter_faster_with_speed() {
        let mut prev: Option<WindMix> = None;
        for i in 0..=300 {
            let m = wind_mix(&c(), i as f32 * 0.1, 0.5);
            assert!(((m.calm.powi(2) + m.steady.powi(2) + m.gale.powi(2)) - 1.0).abs() < 1e-4, "not equal power at {i}");
            if let Some(p) = prev {
                assert!(m.gain >= p.gain - 1e-6 && m.rate >= p.rate - 1e-6 && m.brightness >= p.brightness - 1e-6, "fell at step {i}: {p:?} -> {m:?}");
                assert!(m.gale >= p.gale - 1e-6 && m.calm <= p.calm + 1e-6, "layer crossfade backwards at {i}");
            }
            prev = Some(m);
        }
        let (lo, hi) = (wind_mix(&c(), 1.0, 0.0), wind_mix(&c(), 18.0, 0.0));
        assert!(hi.gain > 2.0 * lo.gain && hi.brightness > lo.brightness + 0.5 && hi.rate > lo.rate && lo.calm > 0.9 && hi.gale > 0.99);
    }

    #[test]
    fn shipped_default_speed_is_the_steady_bed_at_unit_gain() {
        let m = wind_mix(&c(), 6.0, 0.0);
        assert!((m.gain - 1.0).abs() < 1e-5 && m.steady > 0.999 && m.calm.abs() < 1e-3 && m.gale == 0.0);
    }

    #[test]
    fn gust_pushes_toward_louder() {
        assert!(wind_mix(&c(), 8.0, 0.8).gain > wind_mix(&c(), 8.0, 0.0).gain);
        assert!(wind_mix(&c(), 8.0, 0.8).brightness > wind_mix(&c(), 8.0, 0.0).brightness);
    }

    #[test]
    fn rain_silent_at_zero_and_monotone_with_layers() {
        let z = rain_mix(&c(), 0.0);
        assert_eq!((z.light, z.heavy, z.gain), (0.0, 0.0, 0.0));
        assert_eq!(rain_mix(&c(), 0.02).gain, 0.0);
        let mut prev = z;
        for i in 1..=100 {
            let m = rain_mix(&c(), i as f32 / 100.0);
            assert!(m.gain >= prev.gain - 1e-6 && m.heavy >= prev.heavy - 1e-6 && (prev.gain == 0.0 || m.light <= prev.light + 1e-6), "step {i}");
            prev = m;
        }
        let (l, h) = (rain_mix(&c(), 0.15), rain_mix(&c(), 1.0));
        assert!(l.light > 0.99 && l.heavy == 0.0 && h.heavy > 0.99 && h.light < 1e-3 && h.gain > l.gain);
    }

    #[test]
    fn slew_never_steps_and_converges() {
        let (mut x, mut worst) = (0.0f32, 0.0f32);
        for _ in 0..600 { let n = slew(x, 20.0, 1.0 / 60.0, 1.0); worst = worst.max(n - x); x = n; }
        assert!(worst < 0.4 && (x - 20.0).abs() < 0.01, "worst step {worst}, end {x}");
        assert!((slew(0.0, 1.0, 0.5, 1.0) - slew(slew(0.0, 1.0, 0.25, 1.0), 1.0, 0.25, 1.0)).abs() < 1e-6, "not frame-rate independent");
    }
}
