# Handoff (2026-10-06, S2j done; next: collisions with doors, interactables and Skyrim's physics objects)

Where things stand, for the next session. Read `AGENTS.md` first. Details:
- `MODLOG.md` lists every change and test, newest at the end.
- `docs/DESIGN.md` §12 is the plan: steps S2i (stairs), S2j (size), S3-S5 and S4's P1-P5.
- `STATUS.md` covers the stone-stairs problem, which is now resolved.

## Where we are

- **Done and committed:** stairs (S2i, tests 128-135), push out of walls after a load (S2g, test
  136), the player's size and the camera's near distance (S2j, tests 137-140).
  - Protocol **v16**: Skyrim sends its player capsule radius every frame; `gmod/lua/gmsky/size.lua`
    sets the box to half width = that radius (10.9) on both sides.
  - `skse/src/Camera.cpp` `ApplyNear`: while Garry's Mod drives the camera, the near distance
    shrinks so the near rectangle's corners stay inside the body (8.92 instead of 15), set on the
    frustum and on the world scene graph's custom near distance (otherwise Skyrim resets it and
    far shadows flicker, test 139).
- **Next step (the user's choice): collisions with doors, interactables and Skyrim's physics
  objects.** Plan it in `docs/DESIGN.md` first (a new step, S2k), check `reference/SkyCraft` first.

## Next step: what is known so far (not investigated yet)

- **Doors:** at test 136's door the player was blocked at x 71.5 (near (87, 552-586, 144) relative
  to the anchor, logs of tests 136-137) where Skyrim's sent triangles have nothing: an invisible
  wall. The user confirmed a door is there: "clearly doors don't work right now". Guess, not
  checked: the door is a moving body (layer `kAnimStatic`), sent with the region; the region is
  re-sent every second near the player (`kRefreshNear` in `skse/src/Collision.cpp`), and the
  chunk is only rebuilt when its fingerprint changes, so a door's old pose (closed) may stay in
  Garry's Mod's chunk, or the door's swing isn't sent at all. First: log what Skyrim sends for
  that door (its body, layer, pose, each refresh) and what the chunk holds (`C.MeshAt` in STUCK
  lines lists the chunk's own mesh at the contact point; test 138 onward).
- **Skyrim's physics objects (loose clutter: plates, baskets, etc.):** `Collision.cpp`
  `Included()` sends only kStatic, kAnimStatic, kTransparent, kTrees, kProps, kTerrain, kGround
  and kStairHelper. kClutter (moving loose objects) is not sent, so they aren't solid for Garry's
  Mod. They move, so they can't simply go into the static chunks (every move would rebuild a
  chunk); they likely need their own entities that follow Skyrim's pose, and the other way
  (Garry's Mod's player pushing them) is a separate question. Ask the user what they expect: block
  the player only, or also get pushed around by the player.
- Other interactables to look at: activators (levers, chains), containers, furniture, gates
  (also `kAnimStatic`, like doors).

## Open items (the user picks the order)

1. **Bottom ends of flights in the air:** up to 16 in one chunk in test 134, no dumps there and
   nothing noticed. Left alone.
2. **Client/server ramp fingerprint mismatch:** a harmless timing race (MODLOG test 134). Could
   be logged only when it lasts, if it gets in the way.
3. Bumps at the sides of flights (the real shape). Left alone on purpose.
4. **After S2:**
   - S3: props, ragdolls and grenades on Skyrim's ground.
   - S4: props saved in Skyrim's world and save (P1-P5 in DESIGN.md).
   - S5: remove the old traced ground. **The Rust sweep stays: the ramps use it.**

## How the ramps work (code map)

- `skse/src/Collision.cpp`
  - `FlagsFor`, `GuardedShapeMaterial`: the stair-material mark on each triangle.
- `gmod/module/src/shapes.rs`
  - `find_noses`: the front edges of treads ("noses").
  - `pair_noses`: the nearest higher parallel nose across.
  - `ramps_in_box`: the strips, the bend handling, the bottom strips down to the floor and the top
    landing check.
  - `RampSets`: each side's kept ramp triangles per ramp entity; `sweep` (S2i-4).
  - `ramp_check`: the explanation shown in STUCK lines.
  - `dump`: writes the triangles in a box to a file.
- `gmod/module/src/lib.rs`
  - `gmsky.RampsInBox` (with an entity number: also keeps them), `gmsky.RampSweep`,
    `gmsky.RampForget`, `gmsky.RampCheck`, `gmsky.DumpTris`.
- `gmod/lua/gmsky/chunks.lua`
  - The `gmsky_ramp` entity: one per chunk, replaced when its fingerprint changes. No physics
    object; `RAMP:TestCollision` answers player-clip traces from `gmsky.RampSweep`. The client
    builds its own copy.
  - STUCK lines, with an automatic dump to `logs/tris/stuck_N.txt`.
  - The console command `gmsky_dump_tris <name>`.

## Working on stairs offline (the method that works)

When a shape rule fails in game, get the real triangles first. Don't guess from summary counts:
that failed twice and frustrated the user.
- Dumps come from STUCK lines automatically, or from `gmsky_dump_tris <name>` in the Garry's Mod
  console.
- Run the dumps offline:
  `GMSKY_DUMPS="B:/Gmod x Skyrim/build/logs-ingame-test-N/tris" cargo test --release -p gmsky-module ramps_on_real_dumps -- --ignored --nocapture`
  - The path must be absolute.
  - Each dump prints its noses, strips, bottoms and tops, and its bumps (all, 6+ units, within 24
    units).
- Optional environment variables (`<part>` is part of a dump's file name):

  | Variable | What it adds |
  |---|---|
  | `GMSKY_DUMP_MAP=<part>` | letter map and jump map around the spot |
  | `GMSKY_DUMP_HEIGHTS=1` | heights around the spot (with the map) |
  | `GMSKY_DUMP_BUMPS=1` | every bump position |
  | `GMSKY_DUMP_DETAIL=<part>` | noses near the spot and their partners |
  | `GMSKY_PROBE=x,y` | ramp triangles over a point (with DETAIL) |

- To run one dump alone, copy it into its own folder in the scratchpad.
- Dumps are game data. Keep them in `build/` and `logs/`, never in the repo.

## How to work here

- Plan in `docs/DESIGN.md` before code. One step at a time, and one commit per step at step
  boundaries.
- Add a `MODLOG.md` entry after every change. Give the user the exact test and what to expect.
  Write "not tested" when something isn't tested.
- **Before handing over an in-game test:**
  - `cargo build --release` (`cargo test` doesn't build the DLL)
  - `target\release\gmod_harness.exe`
  - for Skyrim plugin changes: `cmake --build --preset release` in `skse/`
- After two failed real attempts at something, stop and write `STATUS.md`.
- Check `reference/SkyCraft` (MIT) before each step, credit what you adapt, and say where we
  differ from it.
- Bash heredocs with quotes break here. Write edit scripts with the Write tool, or use Python
  heredocs with plain strings. PowerShell's `Set-Content -Encoding utf8` adds a byte-order mark;
  don't use it on source files.
- `d3d::tests::knows_an_ex_device` fails now and then (it makes a real Direct3D device). Rerun
  before suspecting a change.
