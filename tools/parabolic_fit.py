# Step I1b (indoor shadows): tries layouts of Skyrim's parabolic (indoor) shadow maps against the
# dumps the plugin writes to logs/parabolic/map_<n>.bin (first 4 captures a run, see World.cpp
# DumpParabolic). For each axis / sign / half layout it looks up every dumped point and counts how
# often the stored depth is within 0.01 of distance / radius. Also saves small pictures of the maps.
#
# Usage: python tools\parabolic_fit.py [folder with map_*.bin] [--png out_folder]
import itertools
import struct
import sys
from pathlib import Path

import numpy as np


def load(path):
    """GMSKYPL1 (test 92) or GMSKYPL2 (adds the mask channel and descriptor 0's transform)."""
    b = path.read_bytes()
    lx, ly, lz, radius = struct.unpack_from("4f", b, 8)
    if b[:8] == b"GMSKYPL1":
        w, h, n = struct.unpack_from("3I", b, 24)
        at = 36
    elif b[:8] == b"GMSKYPL2":
        w, h, n, _mask = struct.unpack_from("4I", b, 24)
        at = 40 + 64  # the transform isn't needed: points are stored relative to the light
    else:
        raise ValueError(f"{path}: not a parabolic dump")
    pts = np.frombuffer(b, np.float32, n * 4, at).reshape(n, 4)
    depth = np.frombuffer(b, np.uint16, w * h, at + n * 16).reshape(h, w) / 65535.0
    return (lx, ly, lz, radius), pts[pts[:, 0] >= 0], depth


def cs_lookup(depth, d, radius):
    """Skyrim's lookup as Community Shaders reconstructs it (Utility.hlsl, RENDER_SHADOWMASKDPB, MIT):
    p = (dz, -dy, 1.111 dx - 0.111) (descriptor 0's transform, as logged in test 85); the -z side is
    the lower half with v flipped; depth = |p| / radius. Returns (stored, compared)."""
    h_, w_ = depth.shape
    p = np.stack([d[:, 2], -d[:, 1], d[:, 0] / 0.9 - 0.1 / 0.9], 1)
    lower = p[:, 2] * 0.5 + 0.5 < 0
    n = p / np.linalg.norm(p, axis=1, keepdims=True)
    dirv = n + np.where(lower, -1.0, 1.0)[:, None] * np.array([0.0, 0.0, 1.0])
    dirv /= np.linalg.norm(dirv, axis=1, keepdims=True)
    uv = dirv[:, :2] / dirv[:, 2:3] * 0.5 + 0.5
    uy = np.where(lower, 1 - 0.5 * uv[:, 1], 0.5 * uv[:, 1])
    col = np.clip((uv[:, 0] * w_).astype(int), 0, w_ - 1)
    row = np.clip((uy * h_).astype(int), 0, h_ - 1)
    return depth[row, col], np.clip(np.linalg.norm(p, axis=1) / radius, 0, 1)


def lookup(depth, d, perm, signs, second, top):
    """Paraboloid along axis perm[2]; u, v from perm[0], perm[1]; uv = (u, v) / (1 + |h|)."""
    h_, w_ = depth.shape
    n = d / np.linalg.norm(d, axis=1, keepdims=True)
    u, v, h = (signs[k] * n[:, perm[k]] for k in range(3))
    pos = h >= 0
    u = np.where(pos, u, second[0] * u) / (1 + np.abs(h))
    v = np.where(pos, v, second[1] * v) / (1 + np.abs(h))
    in_top = pos if top == 0 else ~pos
    col = np.clip(((u * 0.5 + 0.5) * w_).astype(int), 0, w_ - 1)
    row = np.clip(((v * 0.5 + 0.5) * (h_ // 2)).astype(int), 0, h_ // 2 - 1) + np.where(in_top, 0, h_ // 2)
    return depth[row, col]


def main():
    args = sys.argv[1:]
    png = None
    if "--png" in args:
        k = args.index("--png")
        png = Path(args[k + 1])
        del args[k:k + 2]
    folder = Path(args[0]) if args else Path(__file__).resolve().parent.parent / "logs" / "parabolic"
    dumps = [load(p) for p in sorted(folder.glob("map_*.bin"))]
    if not dumps:
        print(f"no dumps in {folder}")
        return
    for i, (light, pts, depth) in enumerate(dumps):
        print(f"map {i}: light {light[:3]} radius {light[3]:.0f}, {len(pts)} points, {np.mean(depth > 0.99998) * 100:.0f}% empty")
        if png:
            from PIL import Image
            png.mkdir(parents=True, exist_ok=True)
            Image.fromarray((np.clip(depth[::8, ::8], 0, 1) * 255).astype(np.uint8)).save(png / f"map_{i}.png")
    names = "xyz"
    results = []
    for perm in itertools.permutations(range(3)):
        for signs in itertools.product((1, -1), repeat=3):
            for second in itertools.product((1, -1), repeat=2):
                for top in (0, 1):
                    hit = tot = dark = 0
                    for _, pts, depth in dumps:
                        s = lookup(depth, pts[:, 1:4], perm, signs, second, top)
                        x = pts[:, 0]
                        hit += np.sum(np.abs(s - x) < 0.01)
                        dark += np.sum(s < x - 0.02)
                        tot += len(x)
                    results.append((hit / tot, dark / tot, perm, signs, second, top))
    results.sort(key=lambda r: -r[0])
    sign = lambda s: "+" if s > 0 else "-"
    for score, dark, perm, signs, second, top in results[:8]:
        print(f"{score * 100:5.1f}% match, {dark * 100:4.1f}% would be dark: u = {sign(signs[0])}{names[perm[0]]}, v = {sign(signs[1])}{names[perm[1]]}, "
              f"axis {sign(signs[2])}{names[perm[2]]}; second half u x{second[0]} v x{second[1]}; axis >= 0 in the {'top' if top == 0 else 'bottom'} half")
    print(f"median layout: {np.median([r[0] for r in results]) * 100:.1f}% match")
    hit = dark = tot = 0
    for light, pts, depth in dumps:
        s, cmp = cs_lookup(depth, pts[:, 1:4], light[3])
        hit += np.sum(np.abs(s - cmp) < 0.01)
        dark += np.sum(s < cmp - 0.02)
        tot += len(s)
    print(f"Community Shaders' formula: {hit / tot * 100:.1f}% match, {dark / tot * 100:.1f}% would be dark")


if __name__ == "__main__":
    main()
