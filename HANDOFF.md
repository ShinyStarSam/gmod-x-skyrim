# Handoff (2026-10-06, after S2i-4 was built; next in-game test is 135)

Where things stand, for the next session. Read `AGENTS.md` first. Details:
- `MODLOG.md` lists every change and test, newest at the end.
- `docs/DESIGN.md` §12 is the plan: steps S2i (stairs), S3-S5 and S4's P1-P5.
- `STATUS.md` covers the stone-stairs problem, which is now resolved.

## Where we are

- **Current work: step S2i, invisible ramps over Skyrim's stairs.** Garry's Mod's player walks
  Skyrim's ground as Garry's Mod physics objects (`gmsky_ground physics`). Skyrim's stairs are
  steps that Source catches on, so smooth ramps are laid over every staircase.
- **S2i-1 (test 128):** the Skyrim plugin marks triangles with Skyrim's stair materials.
- **S2i-2 (test 133)** and **S2i-3 (test 134):** walking the stairs works, including the big stone
  stairs, curved flights and stacked-box steps. User after test 134: "good". Commit `20d28d4`.
- **S2i-4 (built, not committed, not tested in game):** ramps for the player only. The ramp has
  no physics object. It is an invisible box whose `TestCollision` answers only traces that
  include player clip (the player's movement). Bullets, NPCs, props and grenades pass through to
  the real steps. Unit tests (31) and the harness pass, including a new ramp check.
- Protocol is still **v15** (no protocol change).

## The test waiting for the user (test 135)

The user:
- walks the same stairs as test 134
- shoots the steps of a staircase
- throws a grenade onto a staircase
- drops a prop (spawn menu) onto a staircase

**Expect:**
- walking feels the same as test 134
- bullets, the grenade and the prop meet the real steps (no invisible slope above them)
- server log: `ramp SELF-TEST PASSED`, "bullets N (0 hit a ramp)" in the status lines, and
  "TestCollision this second: ... hits" above 0 while walking on stairs

**Afterwards, save the logs yourself:** `tools\save_logs.ps1 -Test 135`. Then read
`build\logs-ingame-test-135\`.

If it passes: commit S2i-4 as one step.

## Open items (the user picks the order)

1. **Bottom ends of flights in the air:** up to 16 in one chunk in test 134, no dumps there and
   nothing noticed. Left alone.
2. **Client/server ramp fingerprint mismatch:** a harmless timing race (MODLOG test 134). Could
   be logged only when it lasts, if it gets in the way.
3. **Player stuck inside the ground right after loading** (test 134 STUCK 1-3, at the anchor
   spot, no stairs). Not looked at.
4. Bumps at the sides of flights (the real shape). Left alone on purpose.
5. **After S2:**
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
