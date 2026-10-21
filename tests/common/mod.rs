//! Shared helpers of the headless gates: run the `sky` example in shot mode (never opens a window) and read its PNGs.
#![allow(dead_code)]
use image::RgbImage;
use std::{path::PathBuf, process::Command};

pub type Px = image::Rgb<u8>;

pub fn luma(p: &Px) -> f32 { 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32 }

/// The built `sky` example: cargo builds examples before running integration tests, next to the test binary's `deps/`.
pub fn example() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    exe.parent().and_then(|d| d.parent()).expect("target dir").join("examples").join("sky")
}

pub fn root() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")) }

/// One shot run: `--shot pose --out shots/<label>` plus `args`. Returns the output directory; panics with the log on a nonzero exit.
pub fn shoot(label: &str, pose: &str, args: &[&str]) -> PathBuf {
    let dir = root().join("shots").join(label);
    let _ = std::fs::remove_dir_all(&dir);
    let out = Command::new(example()).args(["--shot", pose, "--out"]).arg(&dir).args(args).env("CLAUDECODE", "1").output().expect("run the sky example");
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{label}: `--shot {pose}` exit {:?}\n{log}", out.status.code());
    dir
}

pub fn read(dir: &std::path::Path, name: &str) -> RgbImage {
    let p = dir.join(format!("{name}.png"));
    image::open(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display())).to_rgb8()
}

/// A single frame.
pub fn still(label: &str, pose: &str, args: &[&str]) -> RgbImage { read(&shoot(label, pose, args), pose) }

/// `n` frames of a sequence (`--seq`/`--at` arguments in `args`): pose_00 .. pose_{n-1}.
pub fn seq(label: &str, pose: &str, n: usize, args: &[&str]) -> Vec<RgbImage> {
    let dir = shoot(label, pose, args);
    (0..n).map(|i| read(&dir, &format!("{pose}_{i:02}"))).collect()
}

/// Fails the test with the gate's own message when it is red.
pub fn finish(gate: &str, red: Vec<String>) { assert!(red.is_empty(), "{gate} red: {}", red.join("; ")); }
