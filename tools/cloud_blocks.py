#!/usr/bin/env python3
"""Block metric for cloud frame dirs (f*.png). usage: cloud_blocks.py <dir> [<ref_dir>] [--period 8]
Mosaic of flat squares on the march-block grid (4 half-res px = 8 screen px at render_scale 0.5) shows as step-edge energy that peaks at
one phase of x % period (and y % period). Per frame: sky rows (top 80%), e = mean |I[x+1]-I[x]| per phase; blockiness = max phase / median
phase (1.0 = smooth, no grid); also mean abs luma difference to ref frames (same index). Prints per-frame values for x and y averaged."""
import sys, glob, os
import numpy as np
from PIL import Image
a = [x for x in sys.argv[1:] if not x.startswith('--')]
period = int(sys.argv[sys.argv.index('--period') + 1]) if '--period' in sys.argv else 8
if '--period' in sys.argv: a = [x for x in a if x != str(period)]
def load(d):
    out = []
    for f in sorted(glob.glob(os.path.join(d, 'f*.png'))):
        l = np.asarray(Image.open(f).convert('RGB'), dtype=np.float32).mean(axis=2)
        out.append(l[: int(l.shape[0] * 0.8)])
    return out
def block(f):
    r = []
    for g in (f, f.T):
        d = np.abs(np.diff(g, axis=1))
        n = (d.shape[1] // period) * period
        ph = d[:, :n].reshape(d.shape[0], -1, period).mean(axis=(0, 1))
        r.append(ph.max() / np.median(ph))
    return float(np.mean(r))
t = load(a[0]); b = np.array([block(f) for f in t])
print('blockiness per frame', np.round(b, 3), 'mean %.3f max %.3f' % (b.mean(), b.max()))
if len(a) > 1:
    r = load(a[1]); br = np.array([block(f) for f in r])
    print('ref blockiness mean %.3f' % br.mean(), ' |test-ref| mean %.3f' % np.mean([np.abs(x - y).mean() for x, y in zip(t, r)]))
    print('RESULT block_ratio %.3f' % (b.mean() / br.mean()))
