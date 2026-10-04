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
