#!/usr/bin/env python3
"""Coarse-block convergence. usage: cloud_converge.py <dir_test> <dir_div1_ref>   (f01.. = frames after a cut/turn, from cloud_turn.sh)
Per frame (sky rows, top 80%): blk = 8 px grid excess (blockiness-1: worst phase of x%8 / y%8 step-edge energy over the median phase, as G11),
err = mean |luma - luma of the SAME frame at div 1| (same clouds, so shapes cancel; div 1 re-marches every pixel so it is the reference),
dots = isolated dots per 1000 px (>3 levels off 3x3 median, as G10). Prints blk/err/dots per frame."""
import sys, glob, os
import numpy as np
from PIL import Image
from scipy.ndimage import median_filter
def load(d): return [np.asarray(Image.open(f).convert('L'), dtype=np.float32) for f in sorted(glob.glob(os.path.join(d, 'f*.png')))[1:]]
def blk(l):
    l = l[: int(l.shape[0] * 0.8)]; r = []
    for g in (l, l.T):
        d = np.abs(np.diff(g, axis=1)); n = d.shape[1] // 8 * 8
        ph = d[:, :n].reshape(d.shape[0], -1, 8).mean(axis=(0, 1)); r.append(ph.max() / np.median(ph))
    return float(np.mean(r)) - 1
def dots(l):
    l = l[: int(l.shape[0] * 0.8)]; return float((np.abs(l - median_filter(l, size=3)) > 3).mean() * 1e3)
from scipy.ndimage import gaussian_filter
def coarse(l, truth):
    """rms of the 8 px-scale error vs the converged truth (div 1, last frame) and rms of the rest (grain). Coarse chunks = the first number."""
    h = int(l.shape[0] * 0.8); e = (l - truth)[:h]; lo = gaussian_filter(e, 4)
    return float(np.sqrt((lo ** 2).mean())), float(np.sqrt(((e - lo) ** 2).mean()))
def stats(t, r):
    return [(blk(a), float(np.abs(a[:int(a.shape[0]*.8)] - b[:int(b.shape[0]*.8)]).mean()), dots(a)) for a, b in zip(t, r)]
if __name__ == '__main__':
    s = stats(load(sys.argv[1]), load(sys.argv[2]))
    tr = load(sys.argv[2])[-1]; cs = [coarse(a, tr) for a in load(sys.argv[1])]
    print('coarse', [round(c[0], 3) for c in cs]); print('grain ', [round(c[1], 3) for c in cs])
    print('blk ', [round(x[0], 3) for x in s]); print('err ', [round(x[1], 2) for x in s]); print('dots', [round(x[2], 3) for x in s])
