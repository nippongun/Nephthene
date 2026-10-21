#!/bin/bash
# Turn-rate sweep, div 4 (or $DIV) vs div 1 at the same frames. usage: cloud_turn_eval.sh <workdir> [rates deg/s csv, default 45,90]
# Dirs <workdir>/r<rate>_d<div>; div-1 refs are cached in <workdir>/ref/r<rate> (same camera and clock, reusable until the sky or shot options change).
work=$1; rates=${2:-45,90}; AT=20,40,60,80,100,120,140,160; T="$(dirname "$0")/cloud_turn.sh"; div=${DIV:-4}
for r in ${rates//,/ }; do
  deg=$(python3 -c "print($r/60*1000)")
  [ -d "$work/ref/r$r" ] || "$T" "$work/ref/r$r" 1 $AT $deg,1000 || exit 1
  "$T" "$work/r${r}_d$div" $div $AT $deg,1000 || exit 1
  echo "-- $r deg/s"; python3 "$(dirname "$0")/cloud_turn_err.py" "$work/r${r}_d$div" "$work/ref/r$r" | sed -n '3,4p;6p'
done
