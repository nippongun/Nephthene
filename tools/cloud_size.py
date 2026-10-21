"""Offline measurement of the cloud size spectrum.

Ports the CPU noise_3d (base texture R channel) and the no-detail density of
src/shaders/clouds_march.wesl / clouds_shadow.wesl (mean-erosion footprint), then measures:
  1. connected cloud cells in a horizontal slice near the base: equivalent diameters
  2. column tops / thickness over a window
  3. radially averaged power spectrum peak of base_shape
Run: python3 tools/cloud_size.py [--config file.json] [--legacy]   (numpy + scipy, about 50 s; file.json has a "clouds" object like the core Config)
Check (`--legacy`): cov 0.2 slice line shows p99=3.26 and max=5.06; type 0.5 slice max thickness 1200 m.
Default run: the tier-2 tower report, flat-layer density (flat layer, tower=False) against the tower density (tower=True,
reads weather.json clouds.tower_depth_m / top_m; mirrors clouds_common.wesl density_at). Gates: type 0.8 thickness p90 >= 4 km,
type 0.3 p90 <= 1.5 km, Spearman rho(column top, cell diameter) >= 0.5. The flat layer must fail the last one (rho near 0).
"""
import numpy as np
from scipy import ndimage
import sys, time, json, os

N = 128
M32 = np.uint32


def hash_table(f, salt):
    """hash3(c, salt) for every wrapped cell c in 0..f^3, as an (f, f, f, 3) table indexed [z, y, x]."""
    out = np.zeros((f, f, f, 3), np.float32)
    for z in range(f):
        for y in range(f):
            for x in range(f):
                h = (x * 73856093) & 0xFFFFFFFF
                h ^= (y * 19349663) & 0xFFFFFFFF
                h ^= (z * 83492791) & 0xFFFFFFFF
                h ^= (salt * 2654435761) & 0xFFFFFFFF
                vals = []
                for _ in range(3):
                    h ^= (h << 13) & 0xFFFFFFFF
                    h ^= h >> 17
                    h ^= (h << 5) & 0xFFFFFFFF
                    vals.append(np.float32(h) / np.float32(4294967295.0))
                out[z, y, x] = vals
    return out


def grid():
    i = np.arange(N, dtype=np.float32) / N
    z, y, x = np.meshgrid(i, i, i, indexing="ij")
    return x, y, z


def worley(x, y, z, f, tab):
    qx, qy, qz = x * f, y * f, z * f
    cx, cy, cz = np.floor(qx).astype(np.int64), np.floor(qy).astype(np.int64), np.floor(qz).astype(np.int64)
    best = np.full(x.shape, np.inf, np.float32)
    for dz in (-1, 0, 1):
        for dy in (-1, 0, 1):
            for dx in (-1, 0, 1):
                ex, ey, ez = cx + dx, cy + dy, cz + dz
                fp = tab[ez % f, ey % f, ex % f]
                d = np.sqrt((ex + fp[..., 0] - qx) ** 2 + (ey + fp[..., 1] - qy) ** 2 + (ez + fp[..., 2] - qz) ** 2)
                best = np.minimum(best, d)
    return np.minimum(best, 1.0)


def perlin(x, y, z, f, tab):
    qx, qy, qz = x * f, y * f, z * f
    cx, cy, cz = np.floor(qx).astype(np.int64), np.floor(qy).astype(np.int64), np.floor(qz).astype(np.int64)
    tx, ty, tz = qx - cx, qy - cy, qz - cz
    sx, sy, sz = tx * tx * (3 - 2 * tx), ty * ty * (3 - 2 * ty), tz * tz * (3 - 2 * tz)
    g = tab * 2.0 - 1.0
    n = np.linalg.norm(g, axis=-1, keepdims=True)
    g = np.where(n > 0, g / np.maximum(n, 1e-30), 0.0)

    def grad(ox, oy, oz):
        gg = g[(cz + oz) % f, (cy + oy) % f, (cx + ox) % f]
        return gg[..., 0] * (tx - ox) + gg[..., 1] * (ty - oy) + gg[..., 2] * (tz - oz)

    l = lambda a, b, k: a + (b - a) * k
    x00 = l(grad(0, 0, 0), grad(1, 0, 0), sx)
    x10 = l(grad(0, 1, 0), grad(1, 1, 0), sx)
    x01 = l(grad(0, 0, 1), grad(1, 0, 1), sx)
    x11 = l(grad(0, 1, 1), grad(1, 1, 1), sx)
    return l(l(x00, x10, sy), l(x01, x11, sy), sz) * 1.4


def base_texture():
    x, y, z = grid()
    wy = [1.0 - worley(x, y, z, f, hash_table(f, f)) for f in (4, 8, 16)]
    pf = sum(perlin(x, y, z, 4 << o, hash_table(4 << o, 977 + (4 << o))) * 0.5 ** o for o in range(4)) / 1.875 * 0.5 + 0.5
    wf = wy[0] * 0.625 + wy[1] * 0.25 + wy[2] * 0.125
    r = (pf + 0.6 * wf).ravel()
    order = np.argsort(r, kind="stable")
    rank = np.empty_like(order)
    rank[order] = np.arange(r.size)
    r = rank.astype(np.float32) / (r.size - 1)
    q = np.floor(np.clip(r, 0, 1) * 255 + 0.5) / 255.0
    return q.reshape(N, N, N).astype(np.float32)  # [z, y, x]


def sample(tex, u, v, w):
    """Trilinear, repeat; u -> x, v -> y, w -> z (texture coords in tiles)."""
    fx, fy, fz = u * N - 0.5, v * N - 0.5, w * N - 0.5
    x0, y0, z0 = np.floor(fx).astype(np.int64), np.floor(fy).astype(np.int64), np.floor(fz).astype(np.int64)
    ax, ay, az = fx - x0, fy - y0, fz - z0
    out = 0.0
    for dz in (0, 1):
        for dy in (0, 1):
            for dx in (0, 1):
                wgt = (ax if dx else 1 - ax) * (ay if dy else 1 - ay) * (az if dz else 1 - az)
                out = out + wgt * tex[(z0 + dz) % N, (y0 + dy) % N, (x0 + dx) % N]
    return out


def smoothstep(a, b, x):
    t = np.clip((x - a) / (b - a), 0, 1)
    return t * t * (3 - 2 * t)


def gradient(h, kind):
    top = 0.22 + (1.0 - 0.22) * kind
    soft = np.sin(kind * np.pi)
    rise = 0.03 + (0.07 - 0.03) * soft
    return smoothstep(0.0, rise, h) * (1.0 - smoothstep(top * (0.75 + (0.25 - 0.75) * soft), top, h))


BASE_TILE, BASE_TILE2, COV_VAR, DETAIL_STRENGTH = 11000.0, 4300.0, 0.5, 0.6


CFG = sys.argv[sys.argv.index("--config") + 1] if "--config" in sys.argv else None
WX = json.load(open(CFG))["clouds"] if CFG else {"top_m": 6000.0, "tower_depth_m": []}
TOWER_CURVE = WX.get("tower_depth_m", [])
TOWER_VSQ = 0.3  # clouds.look.tower_vsq (untuned)
TOWER_BILLOW = 0.5  # clouds.look.tower_billow (untuned)


def curve(c, x):
    xs, ys = [a for a, _ in c], [b for _, b in c]
    return np.interp(x, xs, ys)


def density(tex, x, z, alt, h, cov, kind, tower=False, bottom=1500.0):
    # Tower tier: the base shape is sampled with the altitude squashed toward the shell bottom, so a blob is one coherent column; the
    # column's depth is curve(type) * sqrt(blob strength), and the height fraction fed to gradient/erosion is relative to that column.
    def shape(ys):
        a = sample(tex, x / BASE_TILE, ys / BASE_TILE, z / BASE_TILE)
        qx, qy, qz = 0.7986 * x - 0.6018 * z + 3170.0, ys + 910.0, 0.6018 * x + 0.7986 * z + 5230.0
        b = sample(tex, qx / BASE_TILE2, qy / BASE_TILE2, qz / BASE_TILE2)
        m = 0.5 * (a + b)
        return np.where(m > 0.5, 1 - 2 * (1 - m) ** 2, 2 * m * m)
    bs_col = shape(bottom + (alt - bottom) * TOWER_VSQ if tower else alt)
    bs = (1 - TOWER_BILLOW) * bs_col + TOWER_BILLOW * shape(alt) if tower else bs_col
    vary = sample(tex, x / (BASE_TILE * 2.3) + 0.31, 0.0 * x + 0.5, z / (BASE_TILE * 2.3) + 0.77)
    c_ = np.clip(cov * (1 + COV_VAR * (2 * vary - 1)), 0, 1)
    lo = 1 - np.minimum(c_ * 1.5, 1.0)
    if tower:
        st = np.clip((bs_col - lo) / np.maximum(1 - lo, 1e-6), 0, 1)
        depth = np.maximum(curve(TOWER_CURVE, kind) * np.sqrt(st), 50.0)
        h = (alt - bottom) / depth
    c = np.clip((bs * gradient(h, kind) - lo) / np.maximum(1 - lo, 1e-6), 0, 1)
    # Mean detail erosion (clouds_shadow.wesl): footprint of what the march draws.
    e = 0.5 * DETAIL_STRENGTH * (0.8 + 0.8 * np.clip(h, 0, 1))
    return np.clip((c - e) / (1 - e), 0, 1), bs


def cells(tex, cov, kind, bottom, top, span_m=100_000.0, px=50.0, hf=0.1, tower=False):
    n = int(span_m / px)
    xs = (np.arange(n) + 0.5) * px - span_m / 2
    X, Z = np.meshgrid(xs, xs, indexing="xy")
    alt = bottom + hf * (top - bottom)
    d, bs = density(tex, X, Z, np.full_like(X, alt), hf, cov, kind, tower, bottom)
    mask = d > 0
    lab, k = ndimage.label(mask, structure=np.ones((3, 3)))
    areas = ndimage.sum(np.ones_like(lab), lab, index=np.arange(1, k + 1)) * px * px
    dia = 2 * np.sqrt(areas / np.pi) / 1000.0
    return mask.mean(), dia, bs


def spectrum_peak(field, px):
    f = field - field.mean()
    F = np.abs(np.fft.rfft2(f)) ** 2
    ny, nx = f.shape
    ky = np.fft.fftfreq(ny, d=px)[:, None]
    kx = np.fft.rfftfreq(nx, d=px)[None, :]
    k = np.sqrt(kx ** 2 + ky ** 2)
    bins = np.logspace(np.log10(1 / 60000), np.log10(1 / 200), 40)
    idx = np.digitize(k.ravel(), bins)
    p = np.bincount(idx, weights=F.ravel(), minlength=len(bins) + 1)
    c = np.bincount(idx, minlength=len(bins) + 1)
    pk = p[1:len(bins)] / np.maximum(c[1:len(bins)], 1)
    kc = np.sqrt(bins[:-1] * bins[1:])
    # k * P(k): energy per log-k band
    e = kc * pk
    j = int(np.argmax(e))
    return 1.0 / kc[j], list(zip((1 / kc).round(0).tolist(), (e / e.max()).round(3).tolist()))


def tops(tex, cov, kind, bottom, top, span_m=25_000.0, px=100.0, dz=50.0, tower=False):
    n = int(span_m / px)
    xs = (np.arange(n) + 0.5) * px - span_m / 2
    X, Z = np.meshgrid(xs, xs, indexing="xy")
    alts = np.arange(bottom, top + 1, dz)
    any_ = np.zeros(X.shape, bool)
    lo = np.full(X.shape, np.nan)
    hi = np.full(X.shape, np.nan)
    for a in alts:
        h = (a - bottom) / (top - bottom)
        d, _ = density(tex, X, Z, np.full_like(X, a), h, cov, kind, tower, bottom)
        m = d > 0
        lo = np.where(m & ~any_, a, lo)
        hi = np.where(m, a, hi)
        any_ |= m
    thick = (hi - lo)[any_]
    return any_.mean(), hi[any_] - 0.0, thick


def q(v, ps=(10, 50, 90, 99)):
    return " ".join(f"p{p}={np.percentile(v, p):.2f}" for p in ps) if len(v) else "none"


def legacy(tex):
    t0 = time.time()
    for (name, bottom, top) in (("low", 1500.0, 6000.0), ("thin", 700.0, 3500.0)):
        for cov in (0.12, 0.2, 0.45, 0.6):
            frac, dia, bs = cells(tex, cov, 0.5, bottom, top)
            big = dia[dia > 0]
            area = (np.pi * (big / 2) ** 2)
            share5 = area[big > 5].sum() / max(area.sum(), 1e-9)
            print(f"{name} cov {cov:.2f} kind 0.5 base slice: cloud frac {frac:.3f}, cells {len(big)}, diameter km {q(big)} max={big.max() if len(big) else 0:.2f}, area share in cells >5 km {share5:.2f}")
    # spectrum of base_shape (independent of coverage) at the slice base altitude
    _, _, bs = cells(tex, 0.2, 0.5, 1500.0, 6000.0)
    peak, table = spectrum_peak(bs, 50.0)
    print(f"base_shape energy-per-log-k peak wavelength {peak/1000:.2f} km")
    print("wavelength_m, relative k*P(k):", [(int(w), e) for w, e in table if e > 0.05])
    for (name, bottom, top) in (("low", 1500.0, 6000.0), ("thin", 700.0, 3500.0)):
        for kind in (0.5, 1.0):
            for cov in (0.2, 0.6):
                frac, hi, thick = tops(tex, cov, kind, bottom, top)
                print(f"{name} kind {kind} cov {cov}: cloudy columns {frac:.3f}; top AGL m {q(hi)}; thickness m {q(thick)} max={thick.max() if len(thick) else 0:.0f}")
    print(f"done {time.time()-t0:.1f}s")


def spearman(a, b):
    ra, rb = np.argsort(np.argsort(a)), np.argsort(np.argsort(b))
    return float(np.corrcoef(ra, rb)[0, 1])


def tower_report(tex, tower, cov=0.45, bottom=1500.0, top=None, span_m=40_000.0, px=100.0, dz=100.0):
    """Thickness p90 per type, and Spearman rho(column top, cell diameter) over connected cells of a slice 300 m above the base."""
    top = top or (WX["top_m"] if tower else 6000.0)
    out = {}
    for kind in (0.3, 0.5, 0.8):
        _, _, thick = tops(tex, cov, kind, bottom, top, span_m=25_000.0, px=px, dz=dz, tower=tower)
        out[kind] = (float(np.percentile(thick, 90)) if len(thick) else 0.0, float(thick.max()) if len(thick) else 0.0)
    n = int(span_m / px)
    xs = (np.arange(n) + 0.5) * px - span_m / 2
    X, Z = np.meshgrid(xs, xs, indexing="xy")
    d0, _ = density(tex, X, Z, np.full_like(X, bottom + 300.0), 300.0 / (top - bottom), cov, 0.8, tower, bottom)
    hi = np.full(X.shape, np.nan)
    for a in np.arange(bottom, top + 1, dz):
        d, _ = density(tex, X, Z, np.full_like(X, a), (a - bottom) / (top - bottom), cov, 0.8, tower, bottom)
        hi = np.where(d > 0, a, hi)
    lab, k = ndimage.label(d0 > 0, structure=np.ones((3, 3)))
    idx = np.arange(1, k + 1)
    area = ndimage.sum(np.ones_like(lab), lab, index=idx) * px * px
    ctop = ndimage.maximum(np.nan_to_num(hi, nan=bottom), lab, index=idx) - bottom
    big = area > 4 * px * px
    rho = spearman(2 * np.sqrt(area[big] / np.pi), ctop[big]) if big.sum() > 5 else float("nan")
    return out, rho, int(big.sum())


if __name__ == "__main__":
    t0 = time.time()
    tex = base_texture()
    print(f"base texture built {time.time()-t0:.1f}s mean {tex.mean():.3f}")
    if "--legacy" in sys.argv:
        legacy(tex)
        sys.exit()
    for name, tower in (("flat layer", False), ("towers", True)):
        th, rho, n = tower_report(tex, tower)
        print(f"{name}: thickness p90/max m type0.3 {th[0.3][0]:.0f}/{th[0.3][1]:.0f}  type0.5 {th[0.5][0]:.0f}/{th[0.5][1]:.0f}  type0.8 {th[0.8][0]:.0f}/{th[0.8][1]:.0f}  rho(top, cell diameter) {rho:.2f} over {n} cells")
        ok = th[0.8][0] >= 4000 and th[0.3][0] <= 1500 and rho >= 0.5
        print(f"  gates (0.8 p90 >= 4000, 0.3 p90 <= 1500, rho >= 0.5): {'GREEN' if ok else 'RED'}")
    print(f"done {time.time()-t0:.1f}s")
