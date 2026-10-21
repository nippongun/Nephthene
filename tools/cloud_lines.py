#!/usr/bin/env python3
"""Line metric. usage: cloud_lines.py <dir> [<ref_dir>]   (frames f*.png, e.g. turn-then-hold, --at)
Horizontal streak lines in cloud bodies = band energy (DoG sigma 2..8 px, full res) that varies with y but not x. Per frame (sky rows, top 80%):
row = mean over the sky of (d/dy DoG)^2 / (d/dx DoG)^2 (1 = isotropic grain; >1 horizontal streaks), and band = rms of the DoG. Prints per frame."""
import sys, glob, os
import numpy as np
from PIL import Image
from scipy.ndimage import gaussian_filter
def frames(d):
    return [np.asarray(Image.open(f).convert('L'), dtype=np.float32) for f in sorted(glob.glob(os.path.join(d, 'f*.png')))]
def metric(l):
    l = l[: int(l.shape[0] * 0.8)]
    b = gaussian_filter(l, 2) - gaussian_filter(l, 8)
    gy, gx = np.diff(b, axis=0), np.diff(b, axis=1)
    return float((gy ** 2).mean() / (gx ** 2).mean()), float(b.std())
def run(d):
    return [metric(f) for f in frames(d)]
if __name__ == '__main__':
    t = run(sys.argv[1]); print('row/col ratio', [round(a, 3) for a, _ in t], 'band rms', [round(b, 3) for _, b in t])
    if len(sys.argv) > 2:
        r = run(sys.argv[2]); print('ref row/col', [round(a, 3) for a, _ in r], 'ref band', [round(b, 3) for _, b in r])
