# Bugs

| Date | Bug | Status | Evidence | Next step |
|---|---|---|---|---|
| 2026-10-21 | Ground fog in the morning shade reads weak: G15 pooled change 0.053 luma, budget 0.08. Terrain-following works (clear 0.006, red arm `ignore_terrain` clear 0.117) and G16 passes (shaded floor 0.109 vs red arm `ignore_shadow` 0.056). | open | `cargo test --release --test fog -- --ignored`, run on a software Vulkan driver (lavapipe), not the reference GPU. | Re-run on the reference GPU. If it reproduces: decide whether the budget or the look is wrong. Candidates: shaded fog is lit only by zenith sky radiance (`ambient_scale`, `ambient_od`, untuned) with no light bounced from the sunlit fog top or the slopes. |
