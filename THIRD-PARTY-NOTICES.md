# Third-party notices

## SkyCraft

Gmod x Skyrim copies SkyCraft's architecture (two games running at once, linked through shared
memory) and adapts parts of its Skyrim plugin. Thank you to chasmlol.

- Source: https://github.com/chasmlol/SkyCraft
- License: MIT (full text below)
- Adapted in this project (each file says what it took):
  - `skse/src/Input.cpp`: input bridge hooks (PlayerControls / MenuControls, E activates).
  - `skse/src/Camera.cpp`: turning Skyrim's camera directly after its camera update.
  - `skse/src/Smooth.cpp`: blending between the other game's ticks.
  - `skse/src/Puppet.cpp`, `skse/src/Probe.cpp`: the player puppet and its character
    controller handling (no momentum or fall damage, on-ground / in-air state, gravity kept on).
  - `skse/src/Terrain.cpp`: ground rays around the player ("stage A").
  - `skse/src/Body.cpp`: dropping the puppet body's contacts above its feet, skipping Skyrim's
    land rescue (from SkyCraft's `DigPhysics.cpp`).
  - `skse/src/Collision.cpp`: streaming Skyrim's Havok collision around the player region by
    region (layer list, shape decoding and the offsets SkyCraft found, guarded reads, triangles
    from boxes / capsules / convex hulls, refresh near the player, per-frame limits); and each
    triangle's Skyrim material at its key (SkyCraft's `Dig.cpp` `ShapeMaterial` /
    `GuardedMaterial` and `Collision.cpp` `FlagsFor`), used here to mark stair triangles (S2i-1).
  - `skse/src/Link.cpp` `RingWrite`, `protocol/src/shm.rs` `ring_write` / `ring_read`,
    `protocol/gmsky_protocol.h` "Collision ring": the byte ring with pad messages.
  - `gmod/module/src/shapes.rs`, `gmod/lua/gmsky/field.lua`: the idea of feeding Skyrim's exact
    triangles to the other game's collision query (SkyCraft's `TriCollider.java`); the swept test
    itself is our own.
  - `gmod/lua/gmsky/stairs.lua`: ground judged under a small footprint and steps below step
    height not blocking a grounded player (SkyCraft's `TriCollider.java`), as two rules on top of
    Source's own movement.
  - `docs/DESIGN.md`: architecture.

```
MIT License

Copyright (c) 2026 chasmlol

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## CommonLibSSE-NG

`skse/extern/CommonLibSSE-NG` (git submodule, not copied in): GPL-3.0-or-later with a modding
exception. See its own `LICENSE` and `docs/DESIGN.md` §14 question 4.

## Community Shaders (shader reconstructions)

Gmod x Skyrim reads Skyrim's indoor (parabolic) and sun shadow maps the way Skyrim's own shaders do,
as reconstructed by Community Shaders. Thank you to its authors.

- Source: https://github.com/doodlum/skyrim-community-shaders (read at commit `dd2677fc`),
  `package/Shaders/Utility.hlsl` (`RENDER_SHADOWMAP_PB` and `RENDER_SHADOWMASKDPB`)
- License of the shader files used: MIT (`package/Shaders/LICENSE`, full text below). The rest of
  that repository is under other terms; nothing else from it is used.
- Adapted in this project: `skse/src/World.cpp`: `ParabolicLookup` / `ParaboloidLight` (the
  dual-paraboloid lookup: light-local position, hemisphere choice, uv, depth = length / radius, and
  its soft 8-sample filter), `IndoorShadowVS` (drawing a dual-paraboloid map), the 8 spiral
  sample offsets from `package/Shaders/Common/Random.hlsli` (Jimenez's, as Community Shaders lists them),
  and for the sun `SkyrimCascade` / `SkyrimSunShadow` / `SunShadow` (`SampleShadowPCF`, the same
  filter on the sun's cascades, and the blend from cascade 0 to 1 in `RENDER_SHADOWMASK`).

```
MIT License

Copyright (c) 2022 Ilya Perapechka

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
