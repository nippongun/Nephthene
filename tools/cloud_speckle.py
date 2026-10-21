#!/usr/bin/env python3
"""Block/speckle metric for cloud frame sequences. usage: cloud_speckle.py <dir_test> [<dir_ref_div1>]
Sky rows only (top 80%). speckle = mean |I - median3x3(I)| (isolated dots; clean clouds ~0). With a ref dir also prints mean |test-ref|
and, per frame, speckle(test) - speckle(ref). Frames f*.png (cloud_turn.sh output)."""
import sys, glob, os
import numpy as np
from PIL import Image
from scipy.ndimage import median_filter
def load(d):
    out = []
    for f in sorted(glob.glob(os.path.join(d, 'f*.png'))):
        a = np.asarray(Image.open(f).convert('RGB'), dtype=np.float32)
        l = a.mean(axis=2)
        out.append(l[: int(l.shape[0] * 0.8)])
    return np.stack(out)
def speckle(x):
    # period-8/4 lattice energy would need a fixed phase; the dot rate (>3 levels off the 3x3 median, per 1000 px) is robust.
    return np.array([(np.abs(f - median_filter(f, size=3)) > 3).mean() * 1e3 for f in x])
t = load(sys.argv[1]); s = speckle(t)
print('speckle per frame', np.round(s, 3), 'mean %.3f max %.3f' % (s.mean(), s.max()))
if len(sys.argv) > 2:
    r = load(sys.argv[2]); sr = speckle(r)
    print('ref speckle mean %.3f' % sr.mean(), ' |test-ref| mean %.3f' % np.abs(t - r).mean())
    print('RESULT speckle_excess %.3f' % (s.mean() - sr.mean()))
