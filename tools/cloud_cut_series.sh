#!/bin/bash
# Coarse-error series after a camera cut (120 deg in one frame), div 4 vs div 1 at the same frames, frozen weather, Retina.
# usage: cloud_cut_series.sh <tag> [turn "deg,frames"]  -> /tmp/<tag>/d4, d1 and a printed series. Frames after the cut: 1 5 10 15 20 30 45 60 150.
tag=${1:-cs}; turn=${2:-120,1}; at=2,6,11,16,21,31,46,61,150
here="$(cd "$(dirname "$0")" && pwd)"
export NOLIVE=1  # frozen weather
for d in 4 1; do "$here/cloud_turn.sh" /tmp/$tag/d$d $d $at $turn  || exit 1; done
python3 - /tmp/$tag "$here" <<'PY'
import sys; sys.path.insert(0, sys.argv[2])
import cloud_converge as c
d = sys.argv[1]; t = c.load(d + '/d4'); r = c.load(d + '/d1')
s = [c.coarse(a, b) for a, b in zip(t, r)]
print('frames after cut  ', [1, 5, 10, 15, 20, 30, 45, 60, 150])
print('coarse (8px err)  ', [round(x[0], 2) for x in s]); print('grain             ', [round(x[1], 2) for x in s])
PY
