# Metric tools

Python needs numpy, scipy, Pillow. Shell tools call `cargo run --release --example sky` (override with `SKY=target/release/examples/sky`). Frame dirs hold `f00.png`, `f01.png`, ...

| Tool | Measures | Run |
|---|---|---|
| `cloud_turn.sh` | Capture: turn-then-hold frames at a given `clouds.update_div` (live weather unless `NOLIVE=1`) | `tools/cloud_turn.sh <outdir> <div> <at csv> [deg,frames] [extra sky args]` |
| `cloud_cut_series.sh` | Coarse error after a 120 degree camera cut, div 4 vs div 1 (prints series) | `tools/cloud_cut_series.sh [tag] [deg,frames]` |
| `cloud_blocks.py` | Blockiness of the march-block grid (max/median phase of step-edge energy), ratio to a reference | `python3 tools/cloud_blocks.py <dir> [ref_dir] [--period 8]` |
| `cloud_converge.py` | Per-frame 8 px grid excess, error vs div 1, isolated dots, coarse error | `python3 tools/cloud_converge.py <dir> <div1_ref_dir>` |
| `cloud_lines.py` | Horizontal streak lines in cloud bodies (DoG row/column energy ratio) | `python3 tools/cloud_lines.py <dir> [ref_dir]` |
| `cloud_speckle.py` | Isolated dots (> 3 levels off the 3x3 median per 1000 px), excess over a reference | `python3 tools/cloud_speckle.py <dir> [ref_dir]` |
| `frame_seq_diff.py` | Ground flicker over a frame sequence (frame diff, second difference) | `python3 tools/frame_seq_diff.py <dir-or-glob> [--ground-y0 0.5] [--thr 0.05]` |
| `cloud_size.py` | Offline cloud cell size spectrum and tower thickness from a CPU port of the density (about 50 s) | `python3 tools/cloud_size.py [--config file.json] [--legacy]` |
