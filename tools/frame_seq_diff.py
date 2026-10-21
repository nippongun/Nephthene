#!/usr/bin/env python3
"""Flicker metrics over a --seq frame sequence (shots/<pose>_NN.png).
usage: frame_seq_diff.py <glob-or-dir> [--ground-y0 FRAC] [--thr 0.05]
Luma 0..1 on the lower part of the frame (ground; default rows below 0.5 H). Prints:
 dluma  mean |f[t+1]-f[t]|; pct  % pixels with that > thr; flicker  mean |f[t+1]-2f[t]+f[t-1]| (steady drift cancels)."""
import sys, glob, os
import numpy as np
from PIL import Image
a = [x for x in sys.argv[1:] if not x.startswith('--')]
opt = dict(zip(sys.argv[1:], sys.argv[2:]))
y0 = float(opt.get('--ground-y0', 0.5)); thr = float(opt.get('--thr', 0.05))
pat = a[0]; files = sorted(glob.glob(os.path.join(pat, '*.png') if os.path.isdir(pat) else pat))
fr = []
for f in files:
    im = np.asarray(Image.open(f).convert('RGB'), dtype=np.float32) / 255
    l = 0.2126 * im[..., 0] + 0.7152 * im[..., 1] + 0.0722 * im[..., 2]
    fr.append(l[int(l.shape[0] * y0):])
f = np.stack(fr)
d = np.abs(np.diff(f, axis=0))
h = np.abs(f[2:] - 2 * f[1:-1] + f[:-2])
print(f"n={len(f)} dluma={d.mean():.5f} pct>{thr}={100*(d>thr).mean():.3f} flicker={h.mean():.5f} flicker_pct>{thr}={100*(h>thr).mean():.3f}")
