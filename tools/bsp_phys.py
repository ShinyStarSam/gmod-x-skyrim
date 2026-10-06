# Read-only: list the convex pieces ("ledges") of a BSP's world collision (PHYSCOLLIDE lump 29)
# with their bounding spheres in Source units, and count those whose sphere reaches a given point.
import struct, sys, math
path = sys.argv[1]; px, py, pz, rad = map(float, sys.argv[2:6])
d = open(path, 'rb').read()
assert d[:4] == b'VBSP'
ofs, ln = struct.unpack_from('<ii', d, 8 + 29 * 16)
p = ofs; IN = 1 / 0.0254
while p < ofs + ln:
    model, size, keysize, nsolid = struct.unpack_from('<iiii', d, p); p += 16
    if model == -1: break
    q = p
    for s in range(nsolid):
        ssize, = struct.unpack_from('<i', d, q)
        sp = q + 4
        vid, ver, mtype = struct.unpack_from('<ihh', d, sp)
        surf = sp + 8 + 4 + 12 + 4 if vid == 0x59485056 else sp   # 'VPHY' + compactsurfaceheader
        mc = struct.unpack_from('<3f', d, surf); ulr, = struct.unpack_from('<f', d, surf + 24)
        root = surf + struct.unpack_from('<i', d, surf + 32)[0]
        if model == 0:
            leaves = []
            stack = [root]
            while stack:
                n = stack.pop()
                right, cl = struct.unpack_from('<ii', d, n)
                cx, cy, cz, r = struct.unpack_from('<4f', d, n + 8)
                if right == 0: leaves.append((cx, cy, cz, r))
                else: stack += [n + 28, n + right]
            # IVP axes -> Source: x = x, y = z, z = -y ; metres -> inches
            src = [(x * IN, z * IN, -y * IN, r * IN) for x, y, z, r in leaves]
            near = [l for l in src if math.dist(l[:3], (px, py, pz)) <= l[3] + rad]
            print(f'model 0 solid {s}: {len(src)} ledges, object radius {ulr*IN:.0f}; '
                  f'ledge spheres reaching within {rad:.0f} of ({px},{py},{pz}): {len(near)}')
            for l in sorted(src, key=lambda l: -l[3])[:15]:
                print('   centre (%.0f, %.0f, %.0f) radius %.0f%s' % (*l, '  <- reaches' if l in near else ''))
        q = sp + ssize
    p += size + keysize
