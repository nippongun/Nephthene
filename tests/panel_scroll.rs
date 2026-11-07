//! Gate G37: the weather panel scrolls to its end, stops there and comes straight back. Headless: the `sky` example in
//! `--panel-test` mode drives the panel with synthetic pointer input and prints one `PANEL_SCROLL` line. The red arm
//! (`--panel-test-red`) moves the scroll position by hand with no upper clamp, as the removed wheel system did, and must overshoot
//! the end. Run: `cargo test --release --test panel_scroll -- --nocapture`. Both lines print before any assert.
mod common;
use common::*;
use std::process::Command;

/// One panel-test run (`extra` adds `--panel-test-red`). Prints its PANEL_SCROLL line and returns (max, bottom, back).
fn run(extra: &[&str]) -> (f32, f32, f32) {
    let dir = root().join("shots").join("g37");
    let mut cmd = Command::new(example());
    cmd.args(["--shot", "sky_west", "--out"]).arg(&dir)
        .args(["--panel-test", "--set", "sim.panel_open=true", "--set", r#"sim.panel_expand=["rays","fog","clouds"]"#]);
    cmd.args(extra);
    let out = cmd.env("CLAUDECODE", "1").output().expect("run the sky example");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let log = format!("{stdout}{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "G37 `--panel-test {extra:?}` exit {:?}\n{log}", out.status.code());
    let line = stdout.lines().find(|l| l.starts_with("PANEL_SCROLL ")).unwrap_or_else(|| panic!("G37: no PANEL_SCROLL line\n{log}"));
    println!("{line}");
    let field = |k: &str| line.split_whitespace().find_map(|w| w.strip_prefix(k)).and_then(|v| v.parse::<f32>().ok())
        .unwrap_or_else(|| panic!("G37: no {k} in `{line}`"));
    (field("max="), field("bottom="), field("back="))
}

#[test]
#[cfg(feature = "panel")]
fn g37_panel_scroll() {
    let (max, bottom, back) = run(&[]);
    let (red_max, red_bottom, _) = run(&["--panel-test-red"]);
    assert!(max > 200.0, "G37: the panel does not overflow (max {max:.1}); the gate proves nothing");
    assert!((bottom - max).abs() <= 1.0, "G37: the wheel did not stop at the end (bottom {bottom:.1}, max {max:.1}); budget 1 px of layout rounding");
    assert!((max - back - 300.0).abs() <= 1.0, "G37: three notches back did not move 300 px (back {back:.1}, max {max:.1}); budget 1 px of rounding");
    assert!(red_bottom > red_max + 100.0, "G37 red arm: the unclamped wheel did not bank overscroll (bottom {red_bottom:.1}, max {red_max:.1}); the gate cannot see the bug otherwise");
}
