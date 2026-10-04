# STATUS

Written 2026-10-04 after in-game test 16, for handing off to a new session. Phase 1 step 3c
(Garry's Mod walks on Skyrim's ground) is done and committed together with its crash
investigation. "Verified" means checked with logged numbers from a real run; "not tested" means
not checked in game. Full history with numbers: `MODLOG.md`. Design: `docs/DESIGN.md` §12.

## Goal

Play Skyrim as a Garry's Mod player: Garry's Mod's movement, weapons, health and HUD inside the
real Skyrim world, against real Skyrim NPCs. Both games run at the same time and exchange state
through shared memory; neither game is rewritten (the SkyCraft approach,
https://github.com/chasmlol/SkyCraft). Single-player and offline only.

## Setup

- **Games and exact versions:**
  - Garry's Mod 64-bit (normal x86-64 branch, not a beta), `gmod_win64.exe` file version
    2026.09.16, `garrysmod.ver` 260917, Steam build 25375506.
  - Skyrim Anniversary Edition, `SkyrimSE.exe` **1.7.104.0**, Steam build 24914197.
- **Agent / model:** Claude Code, model Claude Opus 5.5 (`claude-opus-5-5`).
- **Loaders and tools:** SKSE64 for 1.7.104 and Address Library (`versionlib-1-7-104-0.bin`),
  installed by the user; Garry's Mod binary modules in `garrysmod/lua/bin` loaded by `require`;
  CommonLibSSE-NG 10.1.0 (git submodule, `ng` branch, `39f9d07a6`); vcpkg in `.tools/vcpkg`;
  Visual Studio 2022 + Build Tools (MSVC 14.44.35207, includes `dumpbin`); CMake 4.4.3;
  Rust 1.99.0 stable (MSVC); Python 3.12.10; Git 2.51. No mod manager; no other SKSE plugins.
  No debugger installed.
- **Settings:** Skyrim `bAlwaysActive=1`. Garry's Mod started by `tools/launch_gmod.ps1`
  (`-insecure -windowed +fps_max_nofocus 300`), stops at the main menu; the user picks
  **gm_flatmap** (Workshop item 3669476334), Single Player. Then the user starts Skyrim (SKSE)
  and loads a save; Skyrim publishes the origin and the first ground grid.
- **Where things are:**
  - Project `B:\Gmod x Skyrim`; Garry's Mod `B:\SteamLibrary\steamapps\common\GarrysMod`;
    Skyrim `B:\SteamLibrary\steamapps\common\Skyrim Special Edition`.
  - Our files inside the games (the only places we write):
    - `garrysmod\lua\bin\gmsv_gmsky_win64.dll`, `gmcl_gmsky_win64.dll`: loader stubs, written once.
      They load `target\release\gmsky_module.dll` from the project, so rebuilding never touches the game.
    - `garrysmod\lua\autorun\server\gmsky_autoload.lua`: auto-loader, written once.
    - `Data\SKSE\Plugins\gmsky.dll` + `.pdb`: copied by every plugin build.
  - Lua runs straight from `gmod\lua\gmsky\` in the project (no copy).
  - Logs: `logs\` (`skyrim.log`, `gmod_server.log`, `gmod_client.log`, `stub.log`,
    `terrain\grid_<id>.bin` = the last 20 ground grids Garry's Mod received; overwritten on every
    launch). Saved copies per in-game test: `build\logs-ingame-test-N\` (12–16 cover the crash
    work; 13 has the recorded crash grids; 14–15 have crash dumps). Garry's Mod crash dumps:
    `GarrysMod\crashes\`. The user's own copies of replay tests A/B/E: `TEMP LOGS TEST A|B|E\`
    (untracked).

## How to build and test

- Rust (module, stub, harness, protocol): `cargo build --release`, then
  `target\release\gmod_harness.exe` → `HARNESS PASSED 63/63`. It runs our Lua in Garry's Mod's
  real `lua_shared.dll` with fake game objects; it does not run `PhysicsFromMesh`.
- Skyrim plugin: `cmake --build --preset release` (in `skse\`; copies the DLL into Skyrim).
- In game: launch as in Setup, play, then copy `logs\` to `build\logs-ingame-test-N\` before
  the next launch.

## What works (verified in game, numbers in MODLOG.md)

- **The link** (shared memory, protocol version 5), disconnect detection, coordinate
  conventions, scale 64 / 120.5 = 0.5311.
- **Playing from the Skyrim window (step 1):** W/A/S/D, Shift, mouse look, E activates Skyrim
  objects, Esc/~/J/M/F9 reach Skyrim; input returns to Skyrim in menus and on alt-tab. Garry's
  Mod runs at ~240–300 fps in the background. Mouse look without round-trip delay.
- **Smooth walking (step 2):** Skyrim blends between Garry's Mod's ticks (step spread 1–6%).
- **gmsky loads by itself** in single-player once the player is in the map.
- **Step 3a:** walking on a `PhysicsFromMesh` test hill.
- **Step 3b:** Skyrim casts a 65×65 ray grid around the player (4,225 rays, ~27 ms of casting
  over ~18 frames, 0 misses, 60 fps kept) and publishes it; grid centres snap to a 768-unit
  lattice. **NPC/animal rays re-cast** (verified test 16): up to 11 rays per grid hit an actor,
  all re-cast through to the ground (`still on an actor after re-casting: 0` on every grid).
- **Step 3c: Garry's Mod walks on Skyrim's ground** (verified test 16, ~2 min, 65 grids, no
  crash). On a new origin the player is moved to the middle of gm_flatmap and held until grid 1
  is built (16 pieces of 512 triangles, ~23 ms, ~226 ms from arrival), then released on top.
  Each later grid builds only the 4 new pieces at the far edge (4.6–6.7 ms, 44–81 ms from
  arrival) and keeps the 12 it already has; server and client both built all 272 pieces.

## What doesn't work yet

- **Not handled (known crash case, never seen in normal play):** if the player runs off the
  built ground faster than new grids arrive, a new piece would be built under the moving player,
  which crashes Garry's Mod (see "Known engine bug"). Planned guard: hold the player ~40 units
  up for a few ticks before building a piece under it (grid 1 does this and never crashed).
  Suggested to do together with step 5.
- **Fast falls can tunnel through the ground:** once in test 16 (falling at ~493 units/s
  downhill) the player ended up 125.8 units below the ground; the LIFT rule put it back on top.
- **Client status timer error:** the user saw a client-side Lua error near the end of test 16.
  `gmod_client.log`'s 5-second status lines stopped ~25 s before quitting while Skyrim was still
  connected (Garry's Mod deletes a timer whose function errors). Cause not found. The timer now
  catches and logs `ERROR in client status: …` (**not tested in game**). Check the client log
  next session.
- **Step 3d not started:** Skyrim's player still takes its height from its own ground ray, so
  jumping and crouching don't show in Skyrim, and sprinting up steep slopes can put the Skyrim
  player inside the mountain (test 11).
- **Interiors:** rays from 4,000 units above would hit ceilings and roofs.
- **Not started:** camera height/bob/FOV (step 4), origin re-centring (step 5), Garry's Mod's
  hands/weapon/HUD in Skyrim, combat, save/load consistency.

## Which game owns the player

**Garry's Mod is authoritative** for position and view (and later health). Skyrim's player is a
puppet. Temporary exception until step 3d: the Skyrim player's **height** comes from Skyrim's own
ground ray. Skyrim owns the world, NPCs, quests and saves, and publishes the origin.

## The current problem

None blocking. Next planned step: **3d** (Skyrim's player uses Garry's Mod's height), per
`docs/DESIGN.md` §12. Keep the known engine bug below in mind for any code that creates
collision near the player.

## Known engine bug (the step 3c crash, solved by avoiding it)

**What happened:** Garry's Mod crashed (access violation in `vphysics.dll`) on grid 2 in tests
12, 13 and 15, and once in a replay (test 14): always on the first new piece built **under the
player while the player's body was moving** (walking, sprinting, jumping, or held after noclip).

**Evidence** (crash dumps read with scratchpad Python scripts and `dumpbin`, read-only; MODLOG
has all numbers):
- All 4 dumps: write at `vphysics.dll+0xaaff0`, registers `rbp=0xfffd`, `r12=0xffff`,
  `r13=0xfffe`. The function (`+0xaaed0`) adds to a sorted list (float key + pointer, 24 bytes
  per item) numbered with **16-bit** indexes, capped at 65,532 items; it was full and wrote one
  item past the end of its memory. Not a stack overflow (36 KB of 1 MB stack used).
- The overflowing step is the physics working out what the new object touches. It is reached
  from `CBaseEntity::CollisionRulesChanged` → `RecheckCollisionFilter` (`server.dll+0x2eb870`,
  identified by its warning string; runs when `SetSolid`/`EnableCustomCollisions` follow
  `PhysicsFromMesh`), and, when those run first, from object creation inside `PhysicsFromMesh`.
- Never crashed: pieces away from the player (hundreds, incl. 60 copies of one piece in one
  session, and test 13's crash piece built 4,000 units away); pieces under a player standing
  still or held still (grid 1 on every run, replay tests A and B).

**What we tried:**
1. Smaller pieces (16 × 512 triangles instead of one of 8,192): crashed on the first piece.
2. "Overlapping meshes" theory: disproved by experiment, but its fix (grid lattice + reuse the
   pieces a new grid shares with the old one) **is the working fix**: the piece under the player
   is never rebuilt during normal movement (test 16).
3. Making the entity solid before `PhysicsFromMesh`: the crash moved into `PhysicsFromMesh`.
   Reverted.
4. Ruled out: ground shape/steepness, mesh size, the piece's data alone, many builds piling up.

**Tools kept for this:** `gmsky_replay` (Garry's Mod alone, no Skyrim): `<id> [dir]` replays a
recorded grid (default dir `build/logs-ingame-test-13/terrain`), `order near|far`,
`piece <id> <k> here|away`, `repeat <id> <k> <count>`, `hold <x> <y> <z>` / `hold off`, `clear`.
`gmsky_reuse_pieces 0` (resets to 1 on every launch) rebuilds all 16 pieces each grid, which
brings the crash back. Every `building …` log line gives the build count and whether the
player is over the piece.

## Ideas not tried yet

- The guard for the unhandled case above (hold the player up before building under it).
- If the bug must be removed rather than avoided: build the ground from convex blocks
  (`PhysicsInitMultiConvex`) instead of `PhysicsFromMesh` (different collision path; edges may
  catch the player; not known to be free of the same bug).
- Search Garry's Mod issue reports for this `vphysics.dll` crash on the 64-bit branch.

## Files that matter

- `AGENTS.md`: project rules. `docs/DESIGN.md`: architecture and phases. `MODLOG.md`: every
  change with numbers and failures. `README.md`: what works / what doesn't.
  `STATUS-handoff.md`: the template for this file.
- `protocol/gmsky_protocol.h` (layout, conversion; Rust copy `protocol/src/lib.rs`, `shm.rs`).
- Skyrim plugin `skse/src/`: `Probe.cpp` (per frame), `Puppet.cpp` (moves the player),
  `Input.cpp` (input bridge), `Camera.cpp` (turns the camera), `Smooth.cpp` (tick blending),
  `Terrain.cpp` (ground grid rays, lattice, NPC re-cast), `Link.cpp` (shared memory).
- Garry's Mod: `gmod/module/src/lib.rs` (Lua functions `gmsky.*`, incl. `ReadTerrain`,
  `ReadGridFile`, grid recording), `gmod/lua/gmsky/server.lua` (per-tick state, input),
  `client.lua`, `terrain.lua` (ground pieces, hold/lift, replay tool),
  `gmod/autorun/gmsky_autoload.lua`.
- Tests: `gmod/harness/src/main.rs` (63 checks), `protocol/test/`.
- Tools: `tools/launch_gmod.ps1`, `tools/install_gmod_stubs.ps1`, `tools/install_gmod_autoload.ps1`.
