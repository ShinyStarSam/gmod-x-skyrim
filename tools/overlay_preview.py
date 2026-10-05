"""Phase 2a: turns each overlay test's black/white pair into something a person can look at, and
prints what it contains.

The PNGs that gmsky_test_overlay saves carry Source's own alpha channel, which is meaningless
(13-90 on solid pixels, test 27), so an image viewer shows them as mostly see-through or black.
The real transparency comes from the pair: alpha = 1 - (white - black) / 255. This writes
<name>_preview.png (the overlay over a grey checkerboard, composited the way Skyrim will:
out = black + (1 - alpha) * background) next to the pictures, and prints per picture how many
pixels are drawn and where (health / ammo corners, crosshair).

Usage: python tools/overlay_preview.py [folder]   (default: logs/overlay)
Needs Pillow and NumPy.
"""
import glob
import os
import sys

import numpy as np
from PIL import Image


def load(path):
    return np.asarray(Image.open(path).convert("RGB")).astype(float)


def main():
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "..", "logs", "overlay")
    blacks = sorted(glob.glob(os.path.join(folder, "*_black*.png")))
    if not blacks:
        print(f"no *_black*.png in {folder}")
        return
    for black_path in blacks:
        white_path = black_path.replace("_black", "_white")
        if not os.path.exists(white_path):
            continue
        b, w = load(black_path), load(white_path)
        h, wd, _ = b.shape
        alpha = np.clip(1.0 - (w - b).mean(axis=2) / 255.0, 0.0, 1.0)
        drawn = alpha > 0.004

        def share(y0, y1, x0, x1):
            return int(drawn[int(h * y0):int(h * y1), int(wd * x0):int(wd * x1)].sum())

        yy, xx = np.mgrid[0:h, 0:wd]
        checker = np.where(((yy // 16 + xx // 16) % 2)[..., None] == 0, 200.0, 120.0) * np.ones(3)
        out = b + (1.0 - alpha)[..., None] * checker
        preview = black_path.replace("_black", "_preview")
        Image.fromarray(np.clip(out, 0, 255).astype(np.uint8)).save(preview)
        name = os.path.basename(black_path).replace("_black", "")
        print(f"{name}: drawn {int(drawn.sum())} px ({100 * drawn.mean():.1f}%), solid {int((alpha > 0.996).sum())}, "
              f"partly see-through {int((drawn & (alpha <= 0.996)).sum())}; health corner {share(0.85, 1, 0, 0.3)} px, "
              f"ammo corner {share(0.85, 1, 0.7, 1)} px, crosshair (centre 25x25) "
              f"{int(drawn[h // 2 - 12:h // 2 + 13, wd // 2 - 12:wd // 2 + 13].sum())} px -> {os.path.basename(preview)}")
        # One-frame alpha (tests 30, 31): a picture's own alpha channel, drawn with Source's
        # depth-in-alpha switched off ("alpha"), plus alpha writes forced for the 2D parts
        # ("alpha2d") or for everything ("alphaall"), compared with the matte from the black/white
        # pair (the reference). Per area, so it shows which parts write a right alpha.
        h4, w4 = int(h * 0.85), int(wd * 0.7)
        areas = {"health": (slice(h4, h), slice(0, int(wd * 0.3))), "ammo": (slice(h4, h), slice(w4, wd)),
                 "right panels": (slice(int(h * 0.68), h4), slice(w4, wd)),
                 "view model": (slice(int(h * 0.45), h4), slice(int(wd * 0.45), w4)),
                 "crosshair": (slice(h // 2 - 12, h // 2 + 13), slice(wd // 2 - 12, wd // 2 + 13))}
        for variant in ("alpha", "alpha2d", "alphaall"):
            alpha_path = black_path.replace("_black", "_" + variant)
            if not os.path.exists(alpha_path):
                continue
            single = np.asarray(Image.open(alpha_path).convert("RGBA")).astype(float)
            a1 = single[..., 3] / 255.0
            diff = np.abs(a1 - alpha)
            off = diff > 0.1
            per_area = ", ".join(f"{k} {int(off[r].sum())}/{int(drawn[r].sum())}" for k, r in areas.items())
            print(f"   {variant}: alpha vs matte mean |difference| {diff.mean():.4f}; pixels off by > 0.1 (of drawn): {per_area}; "
                  f"colour difference to the black picture {np.abs(single[..., :3] - b).mean():.2f}")
            out1 = single[..., :3] + (1.0 - a1)[..., None] * checker
            Image.fromarray(np.clip(out1, 0, 255).astype(np.uint8)).save(black_path.replace("_black", "_preview_" + variant))


if __name__ == "__main__":
    main()
