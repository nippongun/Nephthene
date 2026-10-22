# GPU handover

Checks that need the owner's GPU (CLAUDE.md). Mark an item done or delete it once approved; otherwise reply with feedback and it stays open.

| Date | Change | Run | Look for | Status |
|---|---|---|---|---|
| 2026-10-22 | Air state; ground fog forms on the dew point spread (`docs/air.md`) | `cargo test --release --test fog -- --ignored --nocapture --test-threads=1` | G16 passes (lavapipe: shaded 0.0835 vs red arm 0.0318); G15, G18, G21 unchanged (forced fog) | open |
| 2026-10-22 | Same | `cargo run --release --example sky -- --terrain --shot fog_valley --hour 5.6 --out shots --set ground_fog.enabled=true --set clouds.enabled=false --set wind.speed_mps=1.0 --set map.default_coverage=0.1 --set map.default_coverage_swing=0.0`, then again with `--set air.dew_below_c=12` | First: a dawn fog pool on the valley floor, `AIR temp 20.0 C dew 20.0 C ... precip_kind 0.00 (ground 1 m)` (the air field, read at the ground; was 20.1 C from the camera-local air). Second: bare valley, `dew 12.4 C`. Look call: is the dawn amount right? | open |
| 2026-10-22 | Weather map advection (PR #4) | `cargo test --release --test weather -- --ignored --nocapture --test-threads=1` | G22 passes on Metal (lavapipe: shift vs offset 0.30, budget 0.95; moved 12.8, minimum 5) | open |
