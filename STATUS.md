# STATUS

Written 2026-10-04 after in-game test 21, for handing off to a new session. Phase 1 steps 3d–3f
and re-plan steps A and B are done and committed. "Verified" means checked with logged numbers
from a real run; "not tested" means not checked in game. Full history with numbers: `MODLOG.md`.
Design and plan: `docs/DESIGN.md` §12 ("Phase 1 re-plan after test 17").

## Goal

Play Skyrim as a Garry's Mod player: Garry's Mod's movement, weapons, health and HUD inside the
real Skyrim world, against real Skyrim NPCs. Both games run at the same time and exchange state
through shared memory; neither game is rewritten (the SkyCraft approach,
https://github.com/chasmlol/SkyCraft, MIT, credited in `THIRD-PARTY-NOTICES.md`).
Single-player and offline only.

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
  - Lua runs straight from `gmod\lua\gmsky\` in the project (no copy; restart Garry's Mod or
    `lua_run gmsky.Reload()` to pick up changes).
  - Logs: `logs\` (`skyrim.log`, `gmod_server.log`, `gmod_client.log`, `stub.log`,
    `terrain\grid_<id>.bin` = the last 20 ground grids; overwritten on every launch). Saved
    copies per in-game test: `build\logs-ingame-test-N\` (17 has the last crash dump; 19 is the
    Garry's Mod-only collision test; 20–21 the current ground). Garry's Mod crash dumps:
    `GarrysMod\crashes\`.
  - SkyCraft's source, read-only, for reference: `reference\SkyCraft` (not tracked).

## How to build and test

- Rust (module, stub, harness, protocol): `cargo build --release`, then
  `target\release\gmod_harness.exe` → `HARNESS PASSED` (78 checks). It runs our Lua in Garry's
  Mod's real `lua_shared.dll` with fake game objects (incl. the collision math and the ground
  entity's `TestCollision`); it does not run Garry's Mod's movement.
- C++ layout test: `cmake --build build/protocol --config Release`, then
  `build\protocol\Release\protocol_test.exe` → `PROTOCOL TEST PASSED`.
- Skyrim plugin: `cmake --build --preset release` (in `skse\`; copies the DLL into Skyrim). After
  adding a new `.cpp`, run `cmake --preset default` first (sources are globbed).
- In game: launch as in Setup, play, then copy `logs\` to `build\logs-ingame-test-N\` before
  the next launch. Garry's Mod-only collision test: `gmsky_test_field` in its console.

## What works (verified in game, numbers in MODLOG.md)

- **The link** (shared memory, protocol version 6), disconnect detection, coordinate
  conventions, scale 64 / 120.5 = 0.5311.
- **Playing from the Skyrim window:** W/A/S/D, Shift, Space, Ctrl, mouse look, **V = Garry's
  Mod noclip** (17 launches in test 21), E activates Skyrim objects, Esc/~/J/M/F9 reach Skyrim.
  Mouse look without round-trip delay; smooth position (Skyrim blends Garry's Mod's ticks).
- **Skyrim's player follows Garry's Mod in 3D (step 3d)**, including jumps and noclip flight.
  Skyrim's own push on the player stays under 3 units (test 17 `max-drift`).
- **Skyrim's player body, as SkyCraft (step 3f, `skse/src/Body.cpp`):** on-ground / in-air state
  from Garry's Mod, gravity kept on (Skyrim AI won't melee a target "in the air"); the body's
  contacts above its feet dropped (all of them in noclip); Skyrim's "land rescue" skipped for our
  player (hook at `REL::ID(37507) + 0xCEA`, checked before patching). Test 17: no origin reset.
- **Ground = Lua triangle collision (re-plan steps A and B):** Skyrim casts a 65×65 ray grid
  (48 Skyrim units apart) around the player; Garry's Mod's `gmsky_field` entity (custom
  collisions, **no physics object**) answers `ENT:TestCollision` with a swept box-vs-triangle
  test. Test 19 (Garry's Mod alone): `startpos` is the bottom of the moving box; the player
  stands 1/32 unit above the surface; 0.03–0.3 ms of Lua per tick. Test 21: 164 grids, no crash,
  grid build 0.1 ms (triangles made only when a trace touches a square), slowest trace 1.26 ms.
- **Skyrim measures ahead of a fast player** (test 21): grids centred at position + velocity ×
  recent cast time (up to 1,205 units ahead), 4 ms of rays a frame above 1,000 units/s:
  **0 freezes** in 17 noclip launches (test 20 before the fix: 14).

## What doesn't work yet

- **Map edge:** ~30,000 Skyrim units from where you loaded you reach the edge of gm_flatmap
  (test 17: x −16,929). Needs step C (moving the origin).
- **Objects are bumpy, no buildings/interiors:** the ray grid only sees the top surface every 48
  units; posts and rocks become steep spikes; no arches, roofs or interiors. Needs step D.
- Ending noclip > ~2,100 Garry's Mod units under Skyrim's ground holds the player; press V again.
- Garry's Mod physics props/ragdolls fall through Skyrim's ground (traces only).
- Not started: camera height / view bob / FOV (step 4), Garry's Mod's hands/weapon/HUD in Skyrim,
  combat, save/load consistency.

## Which game owns the player

**Garry's Mod is authoritative** for position (height too, since step 3d) and view (and later
health). Skyrim's player is a puppet. Skyrim owns the world, NPCs, quests and saves, and
publishes the origin.

## The current problem

None blocking. Next, in order (user's direction 2026-10-04: get to Skyrim's real collision
shapes as soon as possible, triple-checked against SkyCraft):

- **Step C: moving the origin (step 5, `docs/DESIGN.md` §5).** When the player is far from the
  centre of the Garry's Mod map, publish a new origin and move the Garry's Mod player by the same
  amount. With Lua collision this only shifts numbers; nothing is rebuilt. Today a new origin
  epoch sends the player "home" and holds it until grid 1 (`terrain.lua` `T.BeforeAnchor`,
  `Probe.cpp` `PublishOrigin`); a re-centre must instead keep the player's place and speed.
  Check SkyCraft's teleport/resync handling first (`skse/src/Game.cpp`, "teleportPending",
  `kSkyrimTeleportThreshold`; `fabric/.../SkyClient.java` "Freeze the player until Skyrim's
  collision around them has arrived").
- **Step D: Skyrim's real collision shapes (SkyCraft's stage C), ported.** SkyCraft
  `skse/src/Collision.cpp/.h`: walk the loaded Havok world under its read lock; layers static,
  anim static, transparent, trees, props, terrain, ground, invisible walls, stair helpers; decode
  MOPP/BV trees, lists, compressed/extended meshes, convex shapes, boxes, capsules; regions of
  8×8×8 blocks within radius 5, nearest first, continuously; worker thread; stair helpers as
  ramps. We want **triangles** (test 19: they feel right; SkyCraft's own player collision,
  `fabric/src/main/java/dev/skycraft/world/TriCollider.java`, also uses triangles), streamed
  into `field.lua`'s cell store. Needs a larger shared-memory ring (the map is 64 KB) and a
  protocol bump.

## Evidence (latest test, 21)

`build/logs-ingame-test-21/`: `gmod_server.log` 164 × `terrain: grid N is the ground now (4095
squares, 0.1 ms)`, 0 × `HOLD at`, 17 × `noclip ON (V in Skyrim)`; slowest `TestCollision` 1.264
ms; `skyrim.log` `TERRAIN published grid … centred 1205 units ahead of the player`, slowest grid
36 frames / 584 ms from start; no new file in `GarrysMod\crashes\`.

## What we've already tried

- **Ground as Garry's Mod physics objects (`PhysicsFromMesh`), steps 3a–3e:** worked for walking
  but crashed Garry's Mod 5 times (tests 12, 13, 14, 15, 17): access violation in `vphysics.dll`
  `+0xaaff0`, a 16-bit list overflowing while working out what a **new** physics object
  touches, reached from `CollisionRulesChanged` → `RecheckCollisionFilter`
  (`server.dll+0x2eb909`), whenever a piece was created near the moving player. Tried: smaller
  pieces (crashed), lattice + reusing pieces (worked while walking), solid-before-mesh (crash
  moved into `PhysicsFromMesh`), a guard that held the player before building under it (test
  17: crashed with a fast player *beside* a piece). **Replaced** by Lua triangle collision
  (steps A–B), which creates no physics objects. That code is gone (the replay tool,
  `gmsky_reuse_pieces`, `gmsky_test_terrain` with it). Don't go back to physics objects for
  Skyrim's world near the player.
- **Lift rule using the highest point under the footprint:** popped the player onto object
  spikes up to 23×/s (test 17). Now judged at the player's centre (test 18: 0 lifts).
- **Grids centred where the player was:** fast players outran them (test 20). Now centred ahead.

## Ideas not tried yet

- If step D's Lua cost is too high: move the swept test into the Rust module (it already runs
  on both sides) and call it from `TestCollision`.
- Physics props on Skyrim's ground, if ever needed: a few `PhysicsFromMesh` pieces *far* from
  the player only (never near it).
- Faster long view rays on the client: walk cells along the ray (DDA) instead of sections.

## Files that matter

- `AGENTS.md`: project rules. `docs/DESIGN.md`: architecture, phases, re-plan. `MODLOG.md`:
  every change with numbers and failures. `README.md`: what works / what doesn't.
  `STATUS-handoff.md`: the template for this file. `THIRD-PARTY-NOTICES.md`: SkyCraft credit.
- `protocol/gmsky_protocol.h` (layout v6, conversion; Rust copy `protocol/src/lib.rs`, `shm.rs`).
- Skyrim plugin `skse/src/`: `Probe.cpp` (per frame, origin), `Puppet.cpp` (moves the player,
  controller state), `Body.cpp` (contact filter, land rescue), `Input.cpp` (input bridge, V),
  `Camera.cpp`, `Smooth.cpp`, `Terrain.cpp` (ray grid, lattice, look-ahead), `Link.cpp`.
- Garry's Mod: `gmod/module/src/lib.rs` (Lua functions `gmsky.*`), `gmod/lua/gmsky/server.lua`
  (per-tick state, input, noclip), `client.lua`, `terrain.lua` (grids → ground, holds, lift),
  `field.lua` (triangle store, swept test, `gmsky_field` entity, `gmsky_test_field`),
  `gmod/autorun/gmsky_autoload.lua`.
- Tests: `gmod/harness/src/main.rs` (78 checks), `protocol/test/`.
- Tools: `tools/launch_gmod.ps1`, `tools/install_gmod_stubs.ps1`, `tools/install_gmod_autoload.ps1`.
