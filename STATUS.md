# STATUS

Written 2026-10-04 after in-game test 23. Phase 1 steps 3d–3f and re-plan steps A–D are done
and committed. "Verified" means checked with logged numbers
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
  `target\release\gmod_harness.exe` → `HARNESS PASSED` (97 checks); `cargo test --release` (ring and sweep unit tests). It runs our Lua in Garry's
  Mod's real `lua_shared.dll` with fake game objects (incl. the collision math and the ground
  entity's `TestCollision`); it does not run Garry's Mod's movement.
- C++ layout test: `cmake --build build/protocol --config Release`, then
  `build\protocol\Release\protocol_test.exe` → `PROTOCOL TEST PASSED`.
- Skyrim plugin: `cmake --build --preset release` (in `skse\`; copies the DLL into Skyrim). After
  adding a new `.cpp`, run `cmake --preset default` first (sources are globbed).
- In game: launch as in Setup, play, then copy `logs\` to `build\logs-ingame-test-N\` before
  the next launch. Garry's Mod-only collision test: `gmsky_test_field` in its console.

## What works (verified in game, numbers in MODLOG.md)

- **The link** (shared memory `Local\GmodSkyrim_v2`, protocol version 8: grids relative to the anchor, 16 MB collision ring), disconnect detection, coordinate
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

- **Skyrim's real collision shapes (step D), test 23:** walls, stairs, arches, interiors, objects;
  0 faults, 0 dropped, up to 90,116 triangles, slowest sweep 0.1 ms.
- **Re-centring (step C), test 22:** no map edge any more; Garry's Mod moves the player and the
  anchor back to the middle of gm_flatmap together, 42 times in test 22, ~109,000 Skyrim units
  travelled one way.

## What doesn't work yet

- (By choice, not a bug) ending noclip under Skyrim's ground: you fall "under the earth" like in
  Source until Skyrim moves you back up; the user likes it, keep it.
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

- **Step C: re-centring the origin: done (test 22, 42 re-centres, "works flawlessly").**
  Garry's Mod moves the player and the anchor back to the middle of the map together
  (`terrain.lua` `T.Recentre`, `gmsky.ShiftAnchor`); grids are anchor-relative (protocol 7).
  Open question from test 22: a few single-frame jumps of 100–260 Garry's Mod units, mostly
  near window switches, not at re-centres (MODLOG test 22). `gmod_server.log` now logs
  `JUMP tick …` lines (and `jumps N` in the status line) to find out; read them after the next test.
- **Step D: Skyrim's real collision shapes: done (test 23, "perfect and superior to grid").**
  Next in the plan: Phase 1 step 4 (camera height, crouch, view bob, FOV from Garry's Mod), then
  Phase 2 (Garry's Mod's hands, weapon and HUD drawn in Skyrim).

## Evidence (latest test, 22)

`build/logs-ingame-test-22/`: `gmod_server.log` 42 × `origin: RE-CENTRE #n` (player minus anchor
identical before/after in all), 42 × `2 ticks after re-centre` with the speed kept, 0 × `HOLD at`,
furthest player minus anchor y 58,024 (≈ 109,000 Skyrim units); `skyrim.log` origin epoch 1 all
session; 6 × `PUPPET big step` (one = a `LIFT`, the rest mostly near window switches; MODLOG test
22); no new file in `GarrysMod\crashes\`. Test 21 (ground, noclip): `build/logs-ingame-test-21/`.

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
