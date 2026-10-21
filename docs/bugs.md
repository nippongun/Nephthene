# Bugs

| Date | Bug | Status | Evidence | Next step |
|---|---|---|---|---|
| 2026-10-21 | Ground fog in the morning shade reads weak: G15 pooled change 0.053 luma, budget 0.08. Terrain-following works (clear 0.006, red arm `ignore_terrain` clear 0.117) and G16 passes (shaded floor 0.109 vs red arm `ignore_shadow` 0.056). | fixed (lavapipe; Metal re-check pending) | Reproduced on Metal: pooled 0.0535, clear 0.0058 (matches lavapipe within 1%). Cause: sky light was the zenith radiance only (the darkest part of a low-sun sky) and nothing came from below. Fix: mean sky-view radiance over the hemisphere, half from above and half reflected from below (two-stream fog layer over `ground_albedo` ground). After, lavapipe: G15 pooled 0.1273, clear 0.0106, red arm clear 0.1963 (pass); G16 shaded 0.0532 vs red arm 0.0098 (pass). | Re-run `cargo test --release --test fog -- --ignored` on Metal. Bounce from sunlit slopes into shaded fog is still not modelled. |
