"""Phase 2a after test 31: reads what gmsky_test_alpha2d saved and prints, per variant and test
patch, the alpha (transparency) Garry's Mod actually wrote against the alpha it should have
written.

Each patch is white with a known alpha a (255, 128 or 46 of 255) drawn on a cleared, fully
transparent black frame, so:
  - right:     alpha = a                (what Skyrim needs)
  - a x a:     alpha = a * a            (alpha written, but blended like the colour: no separate
                                         alpha rule; e.g. 0.50 -> 0.25)
  - none:      alpha = 0, colour > 0    (drawn, but no alpha written)
  - not drawn: colour = 0
Measured in the middle 20 x 20 of each 40 x 40 patch (median), away from rounded corners.
Also: the engine HUD's health corner and the crosshair (median alpha where something is drawn).

Test 33 (a HUD with an additive glow, HOLOHUD 2): the "grey" frame is the same 2D drawing over
mid-grey. In linear light Garry's Mod's real result there is G, and the "write" frame (colour B
and alpha A over black) predicts it as B + (1 - A) x grey: what Skyrim will show over a grey
world. The report prints how far the prediction is off (sRGB levels 0-255), where (3 x 3 screen
grid), and whether the alpha is too high (Skyrim darkened behind the glow: dark halos) or too low
(see-through where it should cover). It writes <name>_grey_check.png: left the real grey frame,
right the prediction, bottom the error x 8 (red).

Usage: python tools/alpha2d_report.py [folder] [name]   (default: logs/overlay, a2d)
Needs Pillow and NumPy.
"""
import os
import sys

import numpy as np
from PIL import Image


def verdict(alpha, colour, a):
    want = a / 255.0
    if colour < 2:
        return "not drawn"
    if abs(alpha - want) < 0.05:
        return "right"
    if alpha < 0.02:
        return "none"  # for a = 46, "a x a" (0.03) looks the same; the 128 patches tell them apart
    if abs(alpha - want * want) < 0.05:
        return "a x a"
    return "other"


def lin(v):
    """sRGB levels 0-255 -> linear light 0-1."""
    x = v / 255.0
    return np.where(x <= 0.04045, x / 12.92, ((x + 0.055) / 1.055) ** 2.4)


def enc(x):
    """Linear light 0-1 -> sRGB levels 0-255."""
    x = np.clip(x, 0.0, 1.0)
    return 255.0 * np.where(x <= 0.0031308, x * 12.92, 1.055 * x ** (1 / 2.4) - 0.055)


def grey_check(folder, name):
    wp, gp = (os.path.join(folder, f"{name}_{v}.png") for v in ("write", "grey"))
    if not (os.path.exists(wp) and os.path.exists(gp)):
        print("grey check: needs <name>_write.png and <name>_grey.png")
        return
    w = np.asarray(Image.open(wp).convert("RGBA")).astype(float)
    g = np.asarray(Image.open(gp).convert("RGB")).astype(float)
    a = w[..., 3] / 255.0
    b_lin, g_lin = lin(w[..., :3]), lin(g)
    empty = (w[..., :3].max(axis=2) == 0) & (w[..., 3] == 0)
    if empty.sum() < 100:
        print("grey check: no empty pixels to measure the grey")
        return
    grey = np.median(g_lin[empty], axis=0)
    pred = b_lin + (1.0 - a)[..., None] * grey
    err = np.abs(enc(pred) - enc(g_lin)).max(axis=2)
    drawn = (w[..., :3].max(axis=2) > 2) | (a > 0.004) | (np.abs(g_lin - grey).max(axis=2) > 0.004)
    # The alpha the grey frame implies, and what kind of pixel it is.
    a_ref = np.clip(1.0 - ((g_lin - b_lin) / grey).mean(axis=2), 0.0, 1.0)
    additive_only = drawn & (w[..., :3].max(axis=2) > 2) & (a < 0.02)
    h, wd = a.shape
    print(f"grey check (write vs grey): grey = {enc(grey).mean():.0f} levels ({grey.mean():.3f} linear); "
          f"{int(drawn.sum())} px drawn, {int(additive_only.sum())} of them colour with alpha 0 (additive only)")
    if drawn.sum() == 0:
        return
    e = err[drawn]
    print(f"   prediction error over grey (levels): mean {e.mean():.2f}, 95% {np.percentile(e, 95):.1f}, "
          f"max {e.max():.0f}; px off by > 8: {int((e > 8).sum())}, > 24: {int((e > 24).sum())}")
    d = (a - a_ref)[drawn]
    print(f"   alpha vs what the grey frame implies: too high by > 0.1 (dark halo in Skyrim): {int((d > 0.1).sum())} px, "
          f"too low by > 0.1 (see-through): {int((d < -0.1).sum())} px")
    for cls, m in (("additive only", additive_only), ("with alpha", drawn & ~additive_only)):
        if m.sum():
            print(f"   {cls}: {int(m.sum())} px, error mean {err[m].mean():.2f}, > 8: {int((err[m] > 8).sum())}")
    bad = drawn & (err > 8)
    rows = []
    for gy, ylab in enumerate(("top", "middle", "bottom")):
        cells = []
        for gx, xlab in enumerate(("left", "centre", "right")):
            sl = (slice(gy * h // 3, (gy + 1) * h // 3), slice(gx * wd // 3, (gx + 1) * wd // 3))
            cells.append(f"{xlab} {int(drawn[sl].sum())}/{int(bad[sl].sum())}")
        rows.append(f"{ylab}: " + ", ".join(cells))
    print("   where (drawn px / px off by > 8): " + " | ".join(rows))
    pic = np.zeros((h * 2, wd * 2, 3))
    pic[:h, :wd] = enc(g_lin)
    pic[:h, wd:] = enc(pred)
    heat = np.clip(err * 8, 0, 255)
    pic[h:, :wd, 0] = heat
    pic[h:, wd:] = np.stack([a * 255] * 3, axis=2)  # bottom right: the write frame's alpha as grey
    out = os.path.join(folder, f"{name}_grey_check.png")
    Image.fromarray(pic.astype(np.uint8)).save(out)
    print(f"   -> {out} (top: real | predicted; bottom: error x 8 | alpha)")


def main():
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "..", "logs", "overlay")
    name = sys.argv[2] if len(sys.argv) > 2 else "a2d"
    layout_path = os.path.join(folder, f"{name}_patches.txt")
    if not os.path.exists(layout_path):
        print(f"no {layout_path}")
        return
    patches = []
    for line in open(layout_path, encoding="utf-8"):
        p = line.split()
        if len(p) == 7:
            patches.append((p[0], p[1], *map(int, p[2:])))
    for variant in ("plain", "write", "over", "rt"):
        path = os.path.join(folder, f"{name}_{variant}.png")
        if not os.path.exists(path):
            print(f"{variant}: no picture")
            continue
        img = np.asarray(Image.open(path).convert("RGBA")).astype(float)
        h, w, _ = img.shape
        print(f"{variant} ({w}x{h}):")
        where = "rt" if variant == "rt" else "screen"
        for pname, pwhere, x, y, pw, ph, a in patches:
            if pwhere != where:
                continue
            cell = img[y + 10:y + ph - 10, x + 10:x + pw - 10]
            alpha = float(np.median(cell[..., 3])) / 255.0
            colour = float(np.median(cell[..., :3]))
            print(f"   {pname:9s} want {a / 255:.2f} (a x a {(a / 255) ** 2:.2f}): alpha {alpha:.2f}, colour {colour:5.1f} -> "
                  f"{verdict(alpha, colour, a)}")
        if where == "screen":
            # The engine HUD's health corner (bottom left) and the crosshair (centre 25 x 25).
            for label, region in (("health corner", img[int(h * 0.85):, :int(w * 0.3)]),
                                  ("crosshair", img[h // 2 - 12:h // 2 + 13, w // 2 - 12:w // 2 + 13])):
                drawn = region[..., :3].max(axis=2) > 2
                if drawn.sum() == 0:
                    print(f"   {label}: nothing drawn")
                    continue
                al = region[..., 3][drawn] / 255.0
                print(f"   {label}: {int(drawn.sum())} px drawn, alpha median {np.median(al):.2f}, "
                      f"share with alpha 0: {(al < 0.02).mean():.2f}")
    grey_check(folder, name)


if __name__ == "__main__":
    main()
