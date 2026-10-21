//! Bevy 0.20.0's `render_sky.wesl` reads depth sample 0 for every MSAA sample, so a geometry edge
//! leaves a pale hairline (the sky shades the other samples). The plugin swaps in `render_sky_msaa.wesl`, a copy
//! that shades per sample. Active only when `sky.msaa_fix` is on.
//! The copy is a port of one upstream file: the version guard below fails when bevy_pbr or that file changes, as
//! a reminder to re-port it. 
use bevy::{asset::io::embedded::EmbeddedAssetRegistry, prelude::*};
use std::path::{Path, PathBuf};

/// The asset path Bevy's `embedded_asset!(app, "render_sky.wesl")` registers in bevy_pbr.
const UPSTREAM: &str = "bevy_pbr/atmosphere/render_sky.wesl";
const SHADER: &[u8] = include_bytes!("../shaders/render_sky_msaa.wesl");

/// Replaces the embedded `render_sky.wesl`. Called from the plugin build, after AtmospherePlugin has registered it
/// (both run during App build) and before RenderSky loads the shader (during the render app's finish).
pub(crate) fn override_render_sky(app: &mut App) {
    let registry = app.world().get_resource::<EmbeddedAssetRegistry>().expect("EmbeddedAssetRegistry (AssetPlugin)");
    registry.insert_asset(PathBuf::new(), Path::new(UPSTREAM), SHADER);
}

/// The bevy_pbr version the copy was ported from.
#[cfg(test)]
const BEVY_PBR: &str = "0.20.0";
/// FNV-1a 64 of bevy_pbr-0.20.0 `src/atmosphere/render_sky.wesl` (2946 bytes), the text the copy was ported from.
#[cfg(test)]
const UPSTREAM_FNV1A: u64 = 0xce9b_a992_c7ec_05e4;

#[cfg(test)]
mod tests {
    use super::*;

    fn fnv1a(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
    }

    /// Fails when Cargo.lock moves bevy_pbr off the ported version.
    #[test]
    fn bevy_pbr_is_the_ported_version() {
        let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock")).expect("read Cargo.lock");
        let version = lock.split("[[package]]").find(|p| p.contains("name = \"bevy_pbr\"")).and_then(|p| {
            p.lines().find_map(|l| l.strip_prefix("version = \"")?.strip_suffix('"'))
        });
        assert_eq!(version, Some(BEVY_PBR), "bevy_pbr changed: re-port render_sky_msaa.wesl from the new render_sky.wesl");
    }

    /// Fails when the upstream render_sky.wesl in the cargo registry no longer matches the ported text.
    #[test]
    fn upstream_render_sky_is_the_ported_text() {
        let home = std::env::var("CARGO_HOME").map(std::path::PathBuf::from).unwrap_or_else(|_| {
            std::path::PathBuf::from(std::env::var("HOME").expect("HOME")).join(".cargo")
        });
        let src = home.join("registry/src");
        let found: Vec<_> = std::fs::read_dir(&src).expect("cargo registry src")
            .filter_map(|e| e.ok().map(|e| e.path().join(format!("bevy_pbr-{BEVY_PBR}/src/atmosphere/render_sky.wesl"))))
            .filter(|p| p.exists())
            .collect();
        assert!(!found.is_empty(), "bevy_pbr-{BEVY_PBR} render_sky.wesl not found under {src:?}");
        for path in found {
            let text = std::fs::read(&path).expect("read upstream render_sky.wesl");
            assert_eq!(fnv1a(&text), UPSTREAM_FNV1A, "upstream render_sky.wesl changed at {path:?}: re-port render_sky_msaa.wesl");
        }
    }
}
