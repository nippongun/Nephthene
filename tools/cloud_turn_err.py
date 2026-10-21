#!/usr/bin/env python3
"""Per-frame error of a div-N turn/move series vs the div-1 series at the SAME frames (cloud_turn.sh dirs, f01..),
split by screen region. usage: cloud_turn_err.py <dir_test> <dir_div1> [--lead left|right|none]
Per frame: coarse = rms of the >=8 px-scale error (gauss sigma 4 of test-ref), grain = rms of the rest, blk = 8 px grid excess of the TEST
(as G11), all on sky rows (top 80%). coarse is also given for the leading 15% columns (revealed side; yaw>0 reveals the left) and the rest."""
import sys, glob, os
import numpy as np
from PIL import Image
from scipy.ndimage import gaussian_filter
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from cloud_converge import blk
def load(d): return [np.asarray(Image.open(f).convert('L'), dtype=np.float32) for f in sorted(glob.glob(os.path.join(d, 'f*.png')))[1:]]
def clouds(d):
    o = []
    for f in sorted(glob.glob(os.path.join(d, 'f*.png')))[1:]:
        a = np.asarray(Image.open(f).convert('RGB'), dtype=np.float32); o.append((a[..., 0] - a[..., 2]) > -25)
    return o
def rms(x): return float(np.sqrt((x ** 2).mean()))
def frame(a, b, lead='left', cloud=None):
    h = int(a.shape[0] * 0.8); e = (a - b)[:h]; lo = gaussian_filter(e, 4); w = int(e.shape[1] * 0.15)
    m = np.ones_like(lo, bool) if cloud is None else gaussian_filter(cloud[:h].astype(np.float32), 8) > 0.1  # cloud pixels (red - blue > -25) of the ref, dilated
    sl = {'left': (slice(None), slice(0, w)), 'right': (slice(None), slice(-w, None)), 'none': (slice(None), slice(None))}[lead]
    ldm = np.zeros_like(m); ldm[sl] = True
    r = lambda mm: rms(lo[mm]) if mm.any() else 0.0
    return r(m), rms(e - lo), r(m & ldm), r(m & ~ldm), blk(a)
if __name__ == '__main__':
    lead = sys.argv[sys.argv.index('--lead') + 1] if '--lead' in sys.argv else 'left'
    s = np.array([frame(a, b, lead, c) for a, b, c in zip(load(sys.argv[1]), load(sys.argv[2]), clouds(sys.argv[2]))])
    for n, c in zip(('coarse', 'grain', 'lead', 'rest', 'blk'), s.T): print('%-6s' % n, np.round(c, 3).tolist())
    print('RESULT lead/rest %.2f coarse_mean %.3f grain_mean %.3f lead_mean %.3f rest_mean %.3f blk_mean %.3f' % ((s[:, 2].mean() / s[:, 3].mean(),) + tuple(s.mean(axis=0))))
