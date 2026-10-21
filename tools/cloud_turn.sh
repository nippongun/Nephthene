#!/bin/bash
# Headless turn-then-hold capture. usage: cloud_turn.sh <outdir> <div> <at-frames csv> [turn "deg,frames" default 40,20] [extra sky args ...]
# SKY overrides the command (e.g. SKY=target/release/examples/sky). Weather runs live unless NOLIVE=1 is set (frozen). Writes <outdir>/f00.png (before the turn) and f01.. at the given frames after the first shot. Run from anywhere.
out=$1; div=$2; at=$3; turn=${4:-40,20}; shift 4 2>/dev/null
cd "$(dirname "$0")/.." || exit 1
tmp=$(mktemp -d); mkdir -p "$out"
${SKY:-cargo run --release --example sky --} --shot golden_ne25_retina --out "$tmp" --hour 17.5 $([ -z "$NOLIVE" ] && echo --live) --settle 60 --set clouds.update_div=$div \
  --turn "$turn" --seq 1,1 --at "$at" "$@" >"$out/log" 2>&1 || { echo FAIL; tail -3 "$out/log"; exit 1; }
n=$(echo "$at" | tr ',' '\n' | wc -l)
for i in $(seq 0 $n); do cp "$tmp/golden_ne25_retina_0$i.png" "$out/f0$i.png"; done
rm -rf "$tmp"
