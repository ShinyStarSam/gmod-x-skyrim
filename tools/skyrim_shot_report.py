"""Checks Skyrim's drawing of Garry's Mod's picture by numbers (step 2c-1, DESIGN.md "Step 2c in detail").

`gmsky_overlay_shot` (Garry's Mod console) makes Skyrim save, on its next drawn frame:
  logs/overlay/skyrim_shot_<n>_before.bin  Skyrim's frame before our picture is drawn
  logs/overlay/skyrim_shot_<n>_gmod.bin    the Garry's Mod picture drawn (premultiplied, sRGB-encoded)
  logs/overlay/skyrim_shot_<n>_after.bin   Skyrim's frame after
(raw: "GSHT", width, height, DXGI format as u32, then rows of width * 4 bytes).

From `before` and `gmod` this predicts `after` two ways and says which one Skyrim matches:
  linear:       after = sRGB( lin(gmod) + (1 - a) * lin(before) )   (what Draw.cpp aims for)
  approximate:  after = gmod + (1 - a) * before, in sRGB numbers    (SkyCraft's way)
Errors in sRGB levels (0-255), where the picture is opaque, see-through, glow (colour with alpha 0),
or empty; also over the whole frame. PNGs: before, after, picture over white, error x 8.

Usage:  python tools\\skyrim_shot_report.py [<n> | <folder> <n>]   (default: newest in logs/overlay)
"""

import glob
import os
import re
import struct
import sys

import numpy as np
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BGRA = {87, 91, 90}   # DXGI B8G8R8A8 UNORM / SRGB / TYPELESS
RGBA = {28, 29, 27}


def load(path):
    with open(path, "rb") as f:
        magic, w, h, fmt = struct.unpack("<4I", f.read(16))
        if magic != 0x54485347:
            raise ValueError(f"{path}: not a snapshot")
        px = np.frombuffer(f.read(w * h * 4), np.uint8).reshape(h, w, 4).astype(np.float64)
    if fmt in BGRA:
        px = px[:, :, [2, 1, 0, 3]]
    elif fmt not in RGBA:
        raise ValueError(f"{path}: DXGI format {fmt} not handled")
    return px, fmt


def lin(c):  # sRGB 0-255 -> linear 0-1
    c = c / 255.0
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def enc(l):  # linear 0-1 -> sRGB 0-255
    l = np.clip(l, 0, 1)
    return np.where(l <= 0.0031308, l * 12.92, 1.055 * l ** (1 / 2.4) - 0.055) * 255.0


def stats(err, mask):
    e = err[mask]
    if e.size == 0:
        return "no pixels"
    return f"{e.size:>8} px  mean {e.mean():6.2f}  95% {np.percentile(e, 95):6.2f}  max {e.max():6.1f}"


def main():
    args = sys.argv[1:]
    folder = os.path.join(ROOT, "logs", "overlay")
    if len(args) == 2:
        folder, n = args[0], args[1]
    elif len(args) == 1:
        n = args[0]
    else:
        found = [int(m.group(1)) for p in glob.glob(os.path.join(folder, "skyrim_shot_*_after.bin"))
                 if (m := re.search(r"skyrim_shot_(\d+)_after\.bin$", p))]
        if not found:
            print("no snapshots in", folder)
            return 1
        n = str(max(found))
    base = os.path.join(folder, f"skyrim_shot_{n}_")
    before, fb = load(base + "before.bin")
    after, fa = load(base + "after.bin")
    gmod, fg = load(base + "gmod.bin")
    print(f"snapshot {n}: Skyrim {before.shape[1]}x{before.shape[0]} (format {fb}), "
          f"Garry's Mod picture {gmod.shape[1]}x{gmod.shape[0]}")
    if gmod.shape != before.shape:
        print("sizes differ: Skyrim scaled the picture; this report compares only equal sizes")
        return 1

    a = gmod[:, :, 3:4] / 255.0
    linear = enc(lin(gmod[:, :, :3]) + (1 - a) * lin(before[:, :, :3]))
    approx = np.clip(gmod[:, :, :3] + (1 - a) * before[:, :, :3], 0, 255)
    real = after[:, :, :3]
    alpha = gmod[:, :, 3]
    colour = gmod[:, :, :3].max(axis=2) > 0
    regions = {
        "opaque (a=255)": alpha == 255,
        "see-through (0<a<255)": (alpha > 0) & (alpha < 255),
        "glow (a=0, colour)": (alpha == 0) & colour,
        "empty (a=0, black)": (alpha == 0) & ~colour,
        "whole frame": np.ones(alpha.shape, bool),
    }
    for name, pred in (("linear", linear), ("approximate", approx)):
        err = np.abs(np.rint(pred) - real).max(axis=2)
        print(f"\npredicted {name}: error in sRGB levels")
        for rname, mask in regions.items():
            print(f"  {rname:24} {stats(err, mask)}")
    # Blur behind see-through parts (gmsky_overlay_blur 1): there, `after` is not the plain linear
    # prediction but should be smoother than it. Fine detail = mean |pixel - mean of its 4 neighbours|.
    def detail(img, mask):
        g = img.mean(axis=2)
        lap = np.abs(g[1:-1, 1:-1] - (g[:-2, 1:-1] + g[2:, 1:-1] + g[1:-1, :-2] + g[1:-1, 2:]) / 4)
        m = mask[1:-1, 1:-1]
        return lap[m].mean() if m.any() else float("nan")
    see = regions["see-through (0<a<255)"]
    if see.any():
        d_pred, d_real = detail(linear, see), detail(real, see)
        print(f"\nsee-through parts, fine detail: Skyrim {d_real:.2f}, unblurred prediction {d_pred:.2f} "
              f"(ratio {d_real / d_pred:.2f}: about 1 = no blur, well below 1 = blurred)")
    changed = np.abs(after - before)[:, :, :3].max(axis=2) > 0
    print(f"\npixels Skyrim's frame changed: {changed.sum()}; where the picture is empty: "
          f"{(changed & regions['empty (a=0, black)']).sum()} (should be 0)")

    out = os.path.join(folder, f"skyrim_shot_{n}")
    Image.fromarray(before[:, :, :3].astype(np.uint8)).save(out + "_before.png")
    Image.fromarray(real.astype(np.uint8)).save(out + "_after.png")
    over_white = enc(lin(gmod[:, :, :3]) + (1 - a) * 1.0)
    Image.fromarray(np.rint(over_white).astype(np.uint8)).save(out + "_gmod_over_white.png")
    err = np.abs(np.rint(linear) - real).max(axis=2)
    Image.fromarray(np.clip(err * 8, 0, 255).astype(np.uint8)).save(out + "_error_linear_x8.png")
    print("PNGs:", out + "_{before,after,gmod_over_white,error_linear_x8}.png")
    return 0


if __name__ == "__main__":
    sys.exit(main())
