"""Looks into the overlay picture block (step 2b, DESIGN.md "Step 2b in detail") from outside.

Opens Local\\GmodSkyrim_overlay_v1 (protocol/gmsky_protocol.h "Overlay picture block"), prints its
numbers, measures how fast Garry's Mod publishes, and saves the newest picture as a PNG.

It does NOT attach as the reader (that is Skyrim's place): it copies the middle slot (the newest
finished picture) and keeps the copy only if Garry's Mod published nothing while it copied (after
one publish the writer may start writing into that slot). Retries up to 50 times.

Usage:  python tools\\overlay_read.py [seconds to measure, default 2]
Writes: logs/overlay/live_<frame>.png (colour un-premultiplied, so image viewers show it right),
        logs/overlay/live_<frame>_alpha.png (alpha as grey).
"""

import ctypes
import mmap
import os
import struct
import sys
import time

import numpy as np
from PIL import Image

NAME = "Local\\GmodSkyrim_overlay_v1"
MAGIC = 0x564F534D
VERSION = 1
MAX_W, MAX_H = 3840, 2160
PIXELS_OFF = 0x1000
MASK_OFF = MAX_W * MAX_H * 4
SLOT_BYTES = (MASK_OFF + (MAX_W // 4) * (MAX_H // 4) + 0xFFFF) & ~0xFFFF
MAP_SIZE = PIXELS_OFF + 3 * SLOT_BYTES
SLOT_HDR_OFF = 0x40
CTL = struct.Struct("<IIQIIQQIIq")         # GmskyOverlayCtl, 56 bytes
SLOT = struct.Struct("<IIIIQqIIII16x")     # GmskyOverlaySlotHdr, 64 bytes
assert CTL.size == 56 and SLOT.size == 64 and MAP_SIZE == 101257216

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def qpc():
    c, f = ctypes.c_int64(), ctypes.c_int64()
    ctypes.windll.kernel32.QueryPerformanceCounter(ctypes.byref(c))
    ctypes.windll.kernel32.QueryPerformanceFrequency(ctypes.byref(f))
    return c.value, f.value


def ctl(m):
    keys = ("magic", "version", "map_size", "state", "shot_request", "published", "taken",
            "skyrim_w", "skyrim_h", "skyrim_qpc")
    return dict(zip(keys, CTL.unpack_from(m, 0)))


def slot_hdr(m, i):
    keys = ("width", "height", "format", "flags", "frame", "capture_qpc", "row_bytes",
            "mask_w", "mask_h", "reserved0")
    return dict(zip(keys, SLOT.unpack_from(m, SLOT_HDR_OFF + i * SLOT.size)))


def describe_state(s):
    return (f"{s:#x}: middle {s & 3}{' (new picture)' if s & 4 else ''}, "
            f"writer slot {(s >> 4) & 3} {'attached' if s & 0x40 else 'not attached'}, "
            f"reader slot {(s >> 8) & 3} {'attached' if s & 0x400 else 'not attached'}")


def copy_newest(m):
    """(header, bytes) of the middle slot, copied while nothing was published; or (None, reason)."""
    for attempt in range(50):
        before = ctl(m)
        mid = before["state"] & 3
        h = slot_hdr(m, mid)
        if h["width"] == 0 or h["height"] == 0 or h["width"] > MAX_W or h["height"] > MAX_H:
            return None, f"middle slot {mid} holds no picture (size {h['width']}x{h['height']})"
        start = PIXELS_OFF + mid * SLOT_BYTES
        data = m[start:start + h["row_bytes"] * h["height"]]
        after = ctl(m)
        if after["published"] == before["published"] and slot_hdr(m, mid)["frame"] == h["frame"]:
            return (h, data, attempt), None
    return None, "Garry's Mod published during every one of 50 copies"


def main():
    seconds = float(sys.argv[1]) if len(sys.argv) > 1 else 2.0
    # Opens the block, or creates an empty one if Garry's Mod hasn't (then magic is 0).
    m = mmap.mmap(-1, MAP_SIZE, tagname=NAME)
    c = ctl(m)
    if c["magic"] != MAGIC:
        print(f"no overlay block yet (magic {c['magic']:#x}): is Garry's Mod running with the module, "
              "and has it published a picture?")
        return 1
    if c["version"] != VERSION or c["map_size"] != MAP_SIZE:
        print(f"version {c['version']} / size {c['map_size']} don't match this tool ({VERSION} / {MAP_SIZE})")
        return 1
    print("state", describe_state(c["state"]))
    size = f"{c['skyrim_w']}x{c['skyrim_h']}" if c["skyrim_w"] else "not known"
    print(f"frames published {c['published']}, taken by Skyrim {c['taken']}, Skyrim's size {size}")
    p0, t0 = c["published"], time.perf_counter()
    time.sleep(seconds)
    p1, t1 = ctl(m)["published"], time.perf_counter()
    print(f"published per second: {(p1 - p0) / (t1 - t0):.1f} over {t1 - t0:.1f} s")

    got, why = copy_newest(m)
    if got is None:
        print("no picture saved:", why)
        return 1
    h, data, retries = got
    now, freq = qpc()
    age_ms = (now - h["capture_qpc"]) * 1000.0 / freq if h["capture_qpc"] else float("nan")
    flags = [n for b, n in ((1, "premultiplied"), (2, "sRGB")) if h["flags"] & b]
    print(f"newest picture: frame {h['frame']}, {h['width']}x{h['height']}, format {h['format']} "
          f"({'BGRA8' if h['format'] == 1 else '?'}), flags {', '.join(flags) or 'none'}, "
          f"{age_ms:.1f} ms old when copied, {retries} retries, mask {h['mask_w']}x{h['mask_h']}")

    px = np.frombuffer(data, np.uint8).reshape(h["height"], h["row_bytes"])[:, :h["width"] * 4]
    px = px.reshape(h["height"], h["width"], 4)
    rgba = px[:, :, [2, 1, 0, 3]].astype(np.float32)
    a = rgba[:, :, 3]
    print(f"alpha: {np.count_nonzero(a == 0)} px 0, {np.count_nonzero((a > 0) & (a < 255))} px between, "
          f"{np.count_nonzero(a == 255)} px 255; colour where alpha is 0: "
          f"{np.count_nonzero((a == 0) & (rgba[:, :, :3].max(axis=2) > 0))} px (additive glow)")
    if h["flags"] & 1:  # un-premultiply for viewing; additive-only pixels keep their colour over alpha 0
        safe = np.where(a > 0, a, 255.0)[:, :, None]
        rgba[:, :, :3] = np.clip(rgba[:, :, :3] * 255.0 / safe, 0, 255)
    out = os.path.join(ROOT, "logs", "overlay")
    os.makedirs(out, exist_ok=True)
    name = os.path.join(out, f"live_{h['frame']}")
    Image.fromarray(rgba.astype(np.uint8), "RGBA").save(name + ".png")
    Image.fromarray(px[:, :, 3].copy(), "L").save(name + "_alpha.png")
    print("saved", name + ".png", "and", name + "_alpha.png")
    return 0


if __name__ == "__main__":
    sys.exit(main())
