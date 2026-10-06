# Handoff (2026-10-06, after S2i-3; next in-game test is 134)

Where things stand, for the next session. Read `AGENTS.md` first. Details:
- `MODLOG.md` lists every change and test, newest at the end.
- `docs/DESIGN.md` §12 is the plan: steps S2i (stairs), S3-S5 and S4's P1-P5.
- `STATUS.md` covers the stone-stairs problem, which is now resolved.

## Where we are

- **Current work: step S2i, invisible ramps over Skyrim's stairs.** Garry's Mod's player walks
  Skyrim's ground as Garry's Mod physics objects (`gmsky_ground physics`). Skyrim's stairs are
  steps that Source catches on, so smooth ramps are laid over every staircase.
- **S2i-1 (test 128):** the Skyrim plugin marks triangles with Skyrim's stair materials (flag
  `GMSKY_TRI_STAIRS` 0x2). All 7 stair materials are checked, using SkyCraft's material read.
- **S2i-2 (test 133):** the ramps work on the big stone stairs. The user said "Much, much, much
  better!" Commit `9e9659b`.
- **S2i-3:** the rough spots from test 133 are fixed offline. Commit `c3cebcb`, and the working
  tree is clean. **Not tested in game.** The fixes:
  - Strips now meet at the bends of curved flights.
  - A middle step piece is no longer taken for the bottom step.
  - The bottom faces of stacked boxes are no longer taken for step edges.
  - Steps just too steep to walk get a 45° strip with a lip of at most 3 units.
  - Offline result: bumps within 24 units of each of the 15 dumped spots went from 40-120 to 0.
- The module is built (`cargo build --release`), the unit tests pass (30, plus 1 ignored) and the
  harness passes (196 checks). Protocol is **v15**.

## The test waiting for the user (test 134)

The user walks these:
- the curved stone stairs ("very bumpy")
- the box-step stairs ("small lump at end")
- "missing ramp" and "small bump"
- a few others

**Expect:**
- smooth walking with no slots and no lump at the top
- more than 73 status seconds "standing on gmsky_ramp" (test 133's figure)
- fewer than 10 STUCK lines

**Afterwards, save the logs yourself:** `tools\save_logs.ps1 -Test 134`. Then read
`build\logs-ingame-test-134\`, including `tris\`.

## Open items (the user picks the order)

1. **Ramps are not player-only.** Props, grenades and bullets hit them. The user parked this
   behind the stairs. Already tried, both failed:
   - a `ShouldCollide` hook during movement
   - `PhysObj:SetContents(CONTENTS_PLAYERCLIP)`

   The self-test showed traces hit the ramp either way.
2. **One client/server ramp fingerprint mismatch** in test 133. Not looked at yet. The client logs
   these mismatches.
3. **Bottom ends of flights in the air:** 35 in test 133. In the dumps, most are a test artifact,
   because the floor lies below the dump box. Check the real count in test 134.
4. Bumps that remain at the sides and ends of flights: steps of different lengths and a
   fan-shaped bottom step. They are away from the middle where the player walks. Left alone on
   purpose.
5. **After S2:**
   - S3: props, ragdolls and grenades on Skyrim's ground.
   - S4: props saved in Skyrim's world and save (P1-P5 in DESIGN.md).
   - S5: remove the old traced ground.

## How the ramps work (code map)

- `skse/src/Collision.cpp`
  - `FlagsFor`, `GuardedShapeMaterial`: the stair-material mark on each triangle.
- `gmod/module/src/shapes.rs`
  - `find_noses`: the front edges of treads ("noses").
  - `pair_noses`: the nearest higher parallel nose across.
  - `ramps_in_box`: the strips, the bend handling, the bottom strips down to the floor and the top
    landing check.
  - `ramp_check`: the explanation shown in STUCK lines.
  - `dump`: writes the triangles in a box to a file.
- `gmod/module/src/lib.rs`
  - `gmsky.RampsInBox`, `gmsky.RampCheck`, `gmsky.DumpTris`.
- `gmod/lua/gmsky/chunks.lua`
  - The `gmsky_ramp` entity: one per chunk, rebuilt when its fingerprint changes. The client
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
  heredocs with plain strings.
