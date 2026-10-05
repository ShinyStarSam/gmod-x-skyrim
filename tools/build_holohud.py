"""Builds our personal patched copy of HOLOHUD 2 (docs/DESIGN.md "HOLOHUD 2, patched").

Reads the user's Workshop download (read only), extracts it into build/holohud2_gmsky/ (never
tracked by git: HOLOHUD has no licence file, so none of its files go into the repo), checks that
lua/holohud2/render.lua still has everything our patch uses, and appends our patch
(gmod/holohud2_patch/render_append.lua) to it. Writes gmsky_patch.txt (what was built from what).
Installing into Garry's Mod is a separate step the user runs: tools/install_holohud.ps1.

Usage: python tools/build_holohud.py [path to holohud2.gma]
"""
import hashlib
import os
import shutil
import struct
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_GMA = r"B:\SteamLibrary\steamapps\workshop\content\4000\3459525275\holohud2.gma"
OUT = os.path.join(ROOT, "build", "holohud2_gmsky")
PATCH = os.path.join(ROOT, "gmod", "holohud2_patch", "render_append.lua")
RENDER = "lua/holohud2/render.lua"
# What the patch relies on in render.lua (its local variables and the two functions it replaces).
NEEDED = [
    "local RT_FRAME, MAT_FRAME", "local RT_COMPOSITE, MAT_COMPOSITE", "local polygons", "local computed",
    "local r_3d ", "local r_pp ", "local debug ", "local function draw_elements(",
    "function HOLOHUD2.render.RenderHUDBackground(", "function HOLOHUD2.render.RenderHUD(",
]


def read_gma(path):
    """Garry's Mod addon archive: header, file list, then the files' data in the same order."""
    b = open(path, "rb").read()
    if b[:4] != b"GMAD":
        raise SystemExit(f"not a .gma file: {path}")
    p = 5 + 8 + 8  # magic + version, Steam ID, timestamp

    def cstr():
        nonlocal p
        e = b.index(b"\0", p)
        s = b[p:e].decode("utf-8", "replace")
        p = e + 1
        return s

    while cstr():  # required content list, ends with an empty string
        pass
    name = cstr()
    cstr(), cstr()  # description, author
    p += 4  # addon version
    entries = []
    while True:
        (idx,) = struct.unpack_from("<I", b, p)
        p += 4
        if idx == 0:
            break
        fn = cstr()
        (size,) = struct.unpack_from("<q", b, p)
        p += 8 + 4  # size, CRC
        entries.append((fn, size))
    files = {}
    for fn, size in entries:
        files[fn] = b[p:p + size]
        p += size
    return name, files


def main():
    gma = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_GMA
    name, files = read_gma(gma)
    if RENDER not in files:
        raise SystemExit(f"{RENDER} is not in {gma}")
    render = files[RENDER].decode("utf-8")
    missing = [n for n in NEEDED if n not in render]
    if missing:
        raise SystemExit(f"HOLOHUD's {RENDER} changed; the patch needs: {missing}. Not building.")
    # Only ever delete our own earlier build.
    if os.path.exists(OUT):
        if not os.path.exists(os.path.join(OUT, "gmsky_patch.txt")):
            raise SystemExit(f"{OUT} exists but isn't one of our builds; not touching it")
        shutil.rmtree(OUT)
    for fn, data in files.items():
        parts = fn.replace("\\", "/").split("/")
        if fn.startswith("/") or ":" in fn or ".." in parts:
            raise SystemExit(f"unsafe path in the .gma: {fn!r}")
        dest = os.path.join(OUT, *parts)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        with open(dest, "wb") as f:
            f.write(data)
    patch = open(PATCH, "rb").read()
    with open(os.path.join(OUT, *RENDER.split("/")), "ab") as f:
        f.write(b"\n" + patch)
    st = os.stat(gma)
    info = (f"built {time.strftime('%Y-%m-%d %H:%M:%S')} by tools/build_holohud.py (Gmod x Skyrim)\n"
            f"from {gma} ({st.st_size} bytes, modified {time.strftime('%Y-%m-%d %H:%M:%S', time.localtime(st.st_mtime))}), addon \"{name}\"\n"
            f"patch {os.path.relpath(PATCH, ROOT)} sha256 {hashlib.sha256(patch).hexdigest()}\n"
            f"appended to {RENDER}\n")
    open(os.path.join(OUT, "gmsky_patch.txt"), "w", encoding="utf-8").write(info)
    print(f"{name}: {len(files)} files -> {OUT}; patch appended to {RENDER} ({len(patch)} bytes)")


if __name__ == "__main__":
    main()
