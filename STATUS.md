# STATUS

**2026-10-06, after in-game tests 101-103 and 107: BLOCKED on Garry's Mod's physics crash again** (physics
props on Skyrim's ground, plan `docs/DESIGN.md` "One collision system", step S1). See "The current
problem". Waiting for the user's choice of the next experiment.

**2026-10-06: the indoor shadow problem below is solved (test 93: Community Shaders' formula agrees
with Skyrim's own shadow mask on 99.86% of points). Kept below as a record.**
Written 2026-10-04 after in-game test 31; rewritten 2026-10-05 after in-game test 37 (blocked on
HOLOHUD 2's alpha); **unblocked after test 39** (see "The current problem"). **Phase 1 is complete and committed.** Phase 2 (the
overlay) is in step 2a, an experiment in Garry's Mod alone. "Verified" means checked with logged
numbers from a real run; "not tested" means not checked in game. Full history with numbers:
`MODLOG.md`. Design and plan: `docs/DESIGN.md` §12 (Phase 2 steps, "HUD decision after test 32",
"HOLOHUD 2, patched").

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
  (`-insecure -windowed +fps_max_nofocus 300`) loads **gm_flatmap** straight into single-player
  when `garrysmod\maps\gm_flatmap.bsp` exists (the user is moving it there from the Workshop
  addon, item 3669476334); otherwise it stops at the main menu and the user picks gm_flatmap,
  Single Player. `-Menu` always stops at the menu; `-Map <name>` loads another map. Then the user starts Skyrim (SKSE)
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
  `target\release\gmod_harness.exe` → `HARNESS PASSED` (112 checks); `cargo test --release` (ring and sweep unit tests). It runs our Lua in Garry's
  Mod's real `lua_shared.dll` with fake game objects (incl. the collision math and the ground
  entity's `TestCollision`); it does not run Garry's Mod's movement.
- C++ layout test: `cmake --build build/protocol --config Release`, then
  `build\protocol\Release\protocol_test.exe` → `PROTOCOL TEST PASSED`.
- Skyrim plugin: `cmake --build --preset release` (in `skse\`; copies the DLL into Skyrim). After
  adding a new `.cpp`, run `cmake --preset default` first (sources are globbed).
- In game: launch as in Setup, play. **Claude saves the logs** after each test
  (`tools\save_logs.ps1` → `build\logs-ingame-test-N\`); `tools\launch_gmod.ps1` also saves
  unsaved logs before a launch. Garry's Mod-only collision test: `gmsky_test_field` in its console.

## What works (verified in game, numbers in MODLOG.md)

- **The link** (shared memory `Local\GmodSkyrim_v2`, protocol version 13 (3b: actors; 3c-2: hitboxes; 3c-3: hit test; 3d: hit events): grids relative to the anchor, 16 MB collision ring, view punch), disconnect detection, coordinate
  conventions, scale 64 / 120.5 = 0.5311.
- **Playing from the Skyrim window:** W/A/S/D, Shift, Space, Ctrl, mouse look, **V = Garry's
  Mod noclip** (17 launches in test 21), E activates Skyrim objects, Esc/~/J/M/F9/T/Tab/I/P/F5 reach Skyrim.
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
- Garry's Mod physics props/ragdolls fall through Skyrim's ground (traces only; measured in test
  100; fix blocked, see "The current problem").
- Not started: Garry's Mod's hands/weapon/HUD in Skyrim (Phase 2),
  combat, save/load consistency.

## Which game owns the player

**Garry's Mod is authoritative** for position (height too, since step 3d) and view (and later
health). Skyrim's player is a puppet. Skyrim owns the world, NPCs, quests and saves, and
publishes the origin.

## The current problem

### Physics ground crashes Garry's Mod (2026-10-06, tests 101-103 and 107; blocked)

**What we did:** to make Garry's Mod's physics props, ragdolls and grenades land on Skyrim's ground
(today they fall ~16,300 units to gm_flatmap's floor: test 100, 67 floor crossings by 25 objects
in 70 s), the plan is one collision system like InfMap2's (a Garry's Mod addon for huge maps,
`reference/infmap2`): Skyrim's triangles as Garry's Mod physics objects (`PhysicsFromMesh`) that the
player, props and bullets all use. Step S1 tests the old engine crash first:
`gmsky_test_physground old|infmap` builds a ~3,200-triangle physics mesh of Skyrim's triangles
around the player every 0.5 s and removes the previous one after the new one exists.
**What we expected:** `old` (tests 12-17's entity setup) crashes; `infmap` (InfMap2's chunk setup:
mass 50000, `FVPHYSICS_CONSTRAINT_STATIC`, `FVPHYSICS_NO_SELF_COLLISIONS`, `FL_STATICPROP`,
`FSOLID_FORCE_WORLD_ALIGNED`, custom collision checks) survives 10 minutes.
**What happened:** both crashed within seconds (old: build 5; infmap: build 6), in exactly tests
12-17's place.

**Evidence:**
- Dumps `GarrysMod/crashes/gmod_win64_20261006_022401_1_accessviolation.mdmp` (old) and
  `..._022524_...` (infmap): `0xc0000005` write past a heap block at `vphysics.dll+0xaaff0`;
  `rbp=0xfffd r12=0xffff r13=0xfffe rcx=0x2fff7` (a sorted list with 16-bit numbering, full at
  65,532 items); inner chain `0x97c9b ← 0x99523 ← 0x97912 ← 0x977b8 ← 0x96cea … 0xa0728 ← 0x98cfc`,
  reached from `server.dll+0x2eb909` (`CollisionRulesChanged` → `RecheckCollisionFilter`). Identical
  in both dumps and to tests 12-17.
- Logs (`build/logs-ingame-test-101/` = run 1, `-103/` = run 2): run 2's player stood **still**
  (speed 0) at the same spot for all 6 builds, so each new mesh was the same triangles in the same
  place as the previous one, which still existed (Garry's Mod deletes removed entities at the end
  of the tick). The player touched the test mesh in 35 of 157 ticks. `ShouldCollide` ran (2,578
  calls in 2 s).
- A Lua error the user saw: almost certainly our log line after each build (`PhysObj:GetGameFlags`
  doesn't exist; fixed). It ran after every setup call and didn't cause the crash.

**What this rules out:** the player moving is **not** needed (run 2). InfMap2's settings don't help.
**Pattern that fits every run so far (not proven):** a new mesh created **overlapping an existing
mesh**, where **something touches both** (the player). Test 16 (pieces never overlapped, 272 builds)
never crashed; the 2026-10-04 overlap experiment (overlap, nobody touching) survived; tests 12-15
rebuilt pieces where old pieces still were; InfMap2 never puts two meshes of one chunk in the same
place.

**Test 107 (idea 1, "no overlap", the user's rule "never two meshes in the same place"):** the old
mesh's physics destroyed before the new one was built ("meshes with physics now: 0" on both
sides), player standing still on the spot: **crashed on build 2**, same dump signature. So the
overlap is not the trigger. What every crash shares: a mesh created where the player's body touches
it. Every mesh created where nothing touches it survived (hundreds, tests 12-16). But the *first*
build of every run also touched the player and survived, so it isn't that simple either.

**What differs from InfMap2, which builds chunks under players without crashing:** its triangles
come from a smooth height function (2 per square, about 600 units across in `gm_inf_bliss`);
ours are Skyrim's real collision: thousands of small triangles, stacked layers (test 100: props
crossed 2-3 surfaces in one fall), and probably slivers, near-duplicates and tiny edges. Nobody
has measured the quality of what we feed `PhysicsFromMesh`.

**Ideas not tried yet (in the order I'd suggest; one variable each):**
1. **Clean the mesh before building it.** Weld vertices closer than 1/16 unit, drop degenerate and
   sliver triangles (tiny area or a near-zero angle), drop duplicates in any vertex order. Log the
   mesh's quality every build (counts of each kind, shortest edge, smallest angle) so we see what
   Skyrim's data looks like. Same test as 107 (player standing on it). If it survives, the trigger
   is bad geometry and cleaning is part of the real design.
2. **Not touching:** the same test but the mesh built 2,000 units from the player. Confirms that
   something touching the new mesh is needed.
3. **Player's body off for the moment of creation:** turn the player's physics collisions off,
   build, turn them back on the next tick. Shows whether the crash is in working out the pairs
   between the player and the new mesh (then it would crash on turning back on instead).
4. **A static physics object** (like the map's own ground) made by our Rust module through the
   engine's physics interface. More work; only if 1-3 don't lead anywhere.
5. If none works: a different route for props (see `docs/DESIGN.md` fallbacks), decided with the
   user.

### Solved (2026-10-06, test 93): how Skyrim reads its indoor shadow maps

**Goal of the step:** indoor shadows (plan `docs/DESIGN.md`, "Next: indoor shadows, steps I1b-I3").
Indoors Skyrim's shadows come from point lights with "parabolic" shadow maps. Before our objects
can receive (I2) or cast (I3) those shadows, we need the exact formula from "point in the world"
to "texel and stored depth" in those maps. Step I1b was to find it by numbers.
**Stopped after two real attempts (AGENTS.md rule).** Nothing is drawn differently; the probe only logs.

**What is known (measured):**
- Every indoor parabolic light renders into **slice 0** of a shared 4096 x 4096 x 8 depth array
  (R16 / D16), one light after another; we copy the slice right after its Render (vtable 0x0A) returns.
- **The layout, seen in the dumped maps (test 92):** two stretched round images, one in the top
  half (rows 0-2047), one in the bottom half, each filling the full width; the two are 180° turns of
  each other. Empty texels store 1.0. So a dual paraboloid, one hemisphere per half, as the ports say.
- **Depth is probably distance / radius:** stored ≈ 1.0 x d/R in the runs that matched (slopes
  0.99-1.01, test 91), and in test 92 stored is mostly equal to or a few % above d/R (median ratio
  1.04-1.07), almost never below.
- **Best layout found (offline, 2,231 points, 4 maps):** paraboloid axis = world x, u = +z, v = -y,
  uv = (u, v) / (1 + |axis|), the -x hemisphere with u mirrored, +x in the top half. This agrees with
  the light's own descriptor transforms (light-local x = world z, y = -/+ world y, forward world x).
  **But only 22% of points match within 0.01** (wrong layouts: median 2.4%, so it's not chance),
  and 9% of points would come out shadowed where they probably aren't.

**What's wrong:** close, not exact. A layout that's this close but this inaccurate would give
blotchy, misplaced shadows. Not good enough to build I2 on.

**Evidence:** `build/logs-ingame-test-91/skyrim.log` and `-92/skyrim.log`, lines `indoor shadow
probe I1b`; dumps `build/logs-ingame-test-92/parabolic/map_0..3.bin` (untracked, 32 MB each);
`python tools\parabolic_fit.py build\logs-ingame-test-92\parabolic [--png <folder>]` reproduces the
numbers above and saves pictures of the maps.

**What we've already tried:**
1. In-game probe, 96 candidate layouts x 3 depth formulas (test 91): no consistent winner; most
   high scores were flat-line fits to uniform texels (a flaw in that score). Two runs showed depth =
   d/R with the axis on world x.
2. Offline fit on the raw maps (test 92): all 192 axis / sign / half layouts; then the paraboloid's
   strength k in uv / (1 + k |axis|) from 0.6 to 1.5 and a uv scale 0.9-1.1 (best: k 0.9, 25%); then a
   light position offset up to ±80 units per axis (each map wants a different offset, at most +10%).
   None explains it.

**Ideas not tried yet (my order):**
1. **Read how Skyrim's own shaders do it, instead of guessing.** Skyrim's shadow-map vertex shader
   and its lighting pixel shader contain the exact formula. Community Shaders (open source) has
   readable reconstructions of Skyrim's vanilla shaders; its parabolic / "omni" shadow code would
   settle it. Needs reading their source online (read only, credit if used).
2. **Read the numbers Skyrim gives its shaders.** Hook the shadow render's draw calls and log the
   vertex shader's constant buffer for one parabolic light (the real matrix and any extra numbers
   such as radius or a near plane). The descriptor transform we log has a 1.111 scale and -0.111
   offset that we don't use; the real formula probably does.
3. **Check whether the map changes between our copy and Skyrim's use** (several lights share slice
   0). A second copy at the end of the frame, compared texel by texel with the first.
4. A frame capture with RenderDoc would show the shaders directly, but that means a download and
   attaching a tool to Skyrim: only with your go-ahead.

### Earlier (2026-10-05, after test 74; superseded: Skyrim now draws Garry's Mod's objects itself, steps G3-G6): Garry's Mod's world objects are never hidden by Skyrim's geometry

**What we did:** Garry's Mod's world objects (props, rockets, grenades) and effects are drawn into
a "world layer" that Skyrim draws over its frame. To hide them behind Skyrim's walls and trees,
Garry's Mod must also send how far away each pixel is (a depth picture); Skyrim compares it with
its own depth buffer (SkyCraft's method).
**Expected:** objects behind Skyrim's geometry hidden.
**What happened:** the depth picture is always empty, so every object shows on top. Three ways
of drawing it from Lua failed:
1. A second view with `render.RenderView` + DepthWrite override (test 71): RenderView ignores
   `render.MaterialOverride`; the props came out in their normal materials.
2. `ent:DrawModel()` under a DepthWrite override in the main view (tests 72-73): draws nothing at
   all, not even a test square, even when copied after all of Source's drawing.
3. Fog depth (models black under white fog; test 74): worked with nothing on screen (squares
   0.0006 / 0.0399, but squared, not linear: Source gamma-corrects render targets), then turned
   everything pure white once in game (0.00% pixels with depth for the rest of the test).

**Evidence:** `build/logs-ingame-test-74/gmod_client.log`, lines `present hook: ... depth: ...
fog depth ... squares A ... pixels ...% with depth ... agreement`; `skyrim.log` lines
`re-projection matrix check` (correct) and `world layer drawn ... re-projected`.

**The way out (not built, not tested):** the engine's own depth pass. Return `true` from the
`NeedsDepthPass` hook; Source then draws the scene again itself with DepthWrite into
`_rt_ResolvedFullFrameDepth` (`render.GetResolvedFullFrameDepth()`), upgraded to R32F. This is
what the Garry's Mod wiki documents and what the GShader Library and other depth addons use. Plan
W5 in `docs/DESIGN.md` (after "W4. The fixes").

**Also open:** the RPG's laser glow sprite gets stamped solid (a pulsing black patch). Plan: stamp
through a bone-merged `ClientsideModel` stand-in (DESIGN.md, same place).

### Earlier (resolved): the Direct3D 9 device

**None. Resolved 2026-10-05 in test 41.** The module finds Garry's Mod's Direct3D 9 device and
hooks its `Present` (step 2b-1). Tests 32 and 40 failed because this `d3d9.dll` gives each device
its own function table in ordinary memory, and the search only accepted tables inside `d3d9.dll`.
Test 41: Ex device, pure + multithreaded; `Present` counted on every frame (= client frames);
drawing thread = Lua's thread even with `mat_queue_mode -1`; back buffer `A8R8G8B8` 1280×720, no
multisampling; hook 0.001 ms per frame; no crash.
- **2b-2 done (no game needed):** the picture block `Local\GmodSkyrim_overlay_v1` with a triple
  buffer whose slots can never overlap, even across restarts; unit-tested (20,000 pictures, all
  whole; the test was shown to fail on a broken rule). `fake_overlay` + `tools\overlay_read.py`.
- **2b-3 works (test 42):** `gmsky_overlay 1` prepares every frame and the `Present` hook copies
  it into the block (`gmod/module/src/capture.rs`): every frame published, 0 failures, ~1.1 ms per
  frame; picture right (hands solid, HOLOHUD plates 94, world gone).
- **2b-4 works (test 43, both games):** Skyrim writes its back buffer size (1280×720, = Garry's
  Mod's) into the block and sees Garry's Mod's newest picture, 6-9 ms old, ~180 per second
  (`skse/src/Overlay.cpp`). **Step 2b is done.**
- **Now: step 2c** (Skyrim draws the picture; plan `docs/DESIGN.md` "Step 2c in detail"). **2c-1
  works (test 44):** Skyrim draws Garry's Mod's picture over its frame in linear light (snapshot:
  ≤ 1 level from the prediction), 7-9 ms old, hidden in menus. Holding a prop with the physgun
  blacked out Skyrim's screen (Garry's Mod's halo makes the frame opaque): **fixed**
  (`liveoverlay.lua` keeps the alpha around halos; test 45: ≤ 10% opaque with halos, 100% without
  the fix). Garry's Mod's menus and console show in Skyrim too: **kept, user's choice**.
  **2c-2 works (tests 46-48)** (`skse/src/Arms.cpp`): Skyrim's arms hidden while the overlay is
  live; single-frame flashes around Skyrim menus fixed (0 in test 48). **Step 2c is done.**
  **Blur behind HOLOHUD's plates works (test 49):** done in Skyrim (`skse/src/Draw.cpp`), weight
  from the picture's alpha, every drawn frame, 0.007 ms CPU; `gmsky_overlay_blur 0/1`.
  **Step 2d works (test 50)** (user's choices): Skyrim's crosshair, bars, compass, charge meters
  and arrows hidden while the overlay is live (`skse/src/Hud.cpp`); Garry's Mod's window moved
  off-screen and back (`gmod/module/src/window.rs`, `gmsky_overlay_hidewindow`); Source keeps
  drawing off-screen. **Phase 2's planned steps are done.**
  **Phase 2 closed; Phase 3 (combat) planned** (`docs/DESIGN.md` "Phase 3 steps"). **Step 3a
  rebuilt after the user's feedback, not tested in game:** wheel / 1-6 / clicks go through Garry's
  Mod's own binds (HOLOHUD's weapon menu shows), Q = spawn menu with a cursor driven from Skyrim.
  **3a works (tests 53-54).** **Step 3b works (test 55):** Skyrim's nearby actors reach Garry's Mod
  (protocol v10, log only); distances agree exactly in both logs. **3c-1 works (test 56):**
  people have 19 per-bone capsules, wolves 22, following the animation. **3c-2 works (tests 57-58):**
  stand-ins in Garry's Mod with Skyrim's hitboxes; bullets, crowbar and explosions hit the body
  parts; flat hull for bumping. **NPCs shove the player; 3c-3 works (test 59):**
  the copy matches Skyrim's parts on 99.3-100% of rays. **3d-1 works (test 60):** hits hurt
  Skyrim's NPCs through Skyrim's own hit function (× 3, head × 2; protocol v13). Not working
  (for later, DESIGN.md "Found in test 60"): grenades, rockets, AR2 balls, explosions and the
  physgun beam are invisible; grenades and AR2 balls fall through and don't hurt; physgun can't
  grab NPCs. **3e works (test 61):** Skyrim's NPCs hurt the Garry's Mod
  player (protocol v14; Skyrim player essential and topped up while Garry's Mod drives).
  **3f works (test 62):** dying respawns you at the last calm spot (the user's
  choice), no save reload; input stays with Garry's Mod while dead. F1 = Garry's Mod's console in
  its own window (works, test 64).
  **3d-2 works (tests 63-64):** shooting corpses pushes the ragdoll with Garry's Mod's force × 3;
  blasts throw limbs apart. F1 console works (test 64). **Phase 3 is done.**
  **Phase 4:** 4a world audit done (test 66: cities, interiors, fast travel, dungeon fine). Skyrim's
  T / Tab / I / P / F5 now reach Skyrim. **4b swimming works (tests 67-68),** W + Space too; `gmsky_overlay`
  on by default works; menu keys work (test 68). **Phase 4 is done.**
  **Now: Garry's Mod's world objects and effects in Skyrim** (DESIGN.md, W1-W3, SkyCraft's depth
  test). Test 70: W1 never ran (2D-sky quirk), fixed. Test 71: W2/W3 showed black boxes, jitter and
  no depth (override ignored by `render.RenderView`); W4 built (depth drawn in the main view with a
  calibration square, camera re-projection in Skyrim, alpha off for effects). Test 72: black boxes
  fixed; depth still empty (copied before Source's held-back drawing), re-projection aimed at the
  wrong camera, see-through models. Test 73: stamp recursion (stack overflow, ~12 frames/s), depth
  still empty (DepthWrite draws nothing from Lua: dead end). Fog depth + fixes built (test 74).
  Future (user): stamina / magicka bars and quest markers, locations, enemies in HOLOHUD.

## Evidence (tests 33-37)

Pictures and logs: `build/logs-ingame-test-33/` … `-37/` (`overlay/`). Reports:
`python tools\alpha2d_report.py build\logs-ingame-test-N\overlay <name>`.

| Test | What | Alpha result |
|---|---|---|
| 33 | HOLOHUD as installed, 3D visor on | 862,905 px (93.6%) alpha 255 in every frame, forced or not |
| 34 | visor off | 921,599 of 921,600 px alpha 255 in every frame; black/grey matte right (plates 0.37, glow ~0) |
| 35 | patched copy | unforced: plates 255 (104,312 px), rest 0; forced: 921,599 px 255 |
| 36 | step-by-step captures | unforced: glow layer adds no alpha; forced: glow layer makes 921,599 px 255. `RT_FRAME` (0,0,0,255) on plates; `RT_COMPOSITE` alpha 255 on 921,599 px |
| 37 | glow drawn with the override off; plate experiment | glow layer makes 921,599 px 255 in **both** frames; plates fully opaque in our own picture in all 3 set-ups (implied alpha 1.000, plate colour 0 over grey 124) |

Plate see-through on the real screen (test 35 grey frame): 0.368 on every plate pixel (1st-99th
percentile), = 94/255.

## What we've already tried

- **Finding Garry's Mod's Direct3D 9 device (tests 32, 40; solved in 41: tables outside d3d9.dll):** scanning shaderapidx9.dll's data for a
  direct pointer to it. Test 32: matched on `QueryInterface` + `GetRenderState`, none. Test 40:
  matched on `QueryInterface` only, plain and Ex references: none (10 other Direct3D objects).
- **Phase 2a, getting the overlay out of Garry's Mod (tests 25-37, MODLOG):**
  - `vm:DrawModel()` / `hands:DrawModel()` / `render.RenderHUD` into a render target from
    `PostRender`: nothing drawn (test 25).
  - `render.RenderView` into a render target from `PostRender`: only the sky (test 26).
  - Own frame, clear at the view model, capture in `DrawOverlay`: hands, weapon, HUD, panels
    (tests 27-29).
  - Source's default alpha: depth, unusable (test 27). `render.SetWriteDepthToDestAlpha(false)`: 3D
    right (test 30).
  - Forcing alpha writes from `PreDrawHUD`: exact for Lua 2D and the engine HUD; VGUI panels 0;
    `OverrideBlend` changes nothing (test 32; test 31's "no fix" was a measuring error).
  - Reading Direct3D 9's settings natively (`gmsky.D3DState`, `gmod/module/src/d3d.rs`): device not
    found (10 objects with a function table in d3d9.dll, none with a plain null device's functions;
    probably a 9Ex device, not verified) (test 32).
  - Our own Lua HUD: started, cancelled by the user (2026-10-05).
  - HOLOHUD 2 as installed: alpha 1 everywhere (tests 33, 34).
  - **HOLOHUD patched, attempt 1** (plates "over" with alpha, glow `OverrideAlphaWriteEnable(true,
    false)`): plates 1, glow fine unforced / alpha 1 everywhere forced (tests 35, 36).
  - **HOLOHUD patched, attempt 2** (glow with the override off): glow alpha 1 everywhere in both
    frames; plates still 1 (test 37).
- **Ground as Garry's Mod physics objects (`PhysicsFromMesh`), steps 3a–3e:** worked for walking
  but crashed Garry's Mod 5 times (tests 12, 13, 14, 15, 17): access violation in `vphysics.dll`
  `+0xaaff0`, a 16-bit list overflowing while working out what a **new** physics object
  touches, reached from `CollisionRulesChanged` → `RecheckCollisionFilter`
  (`server.dll+0x2eb909`), whenever a piece was created near the moving player. **Replaced** by
  Lua triangle collision (steps A–B), which creates no physics objects. Don't go back to physics
  objects for Skyrim's world near the player.
- **InfMap2's chunk setup for the physics mesh (S1, test 103, 2026-10-06):** crashed the same way
  as our old setup (test 101), within 6 builds, with the player standing still. The settings make
  no difference.
- **Never two meshes at once (S1, test 107):** old mesh's physics destroyed before building the
  new one: crashed on build 2, player standing still. Overlap is not the trigger.
- **Lift rule using the highest point under the footprint:** popped the player onto object
  spikes up to 23×/s (test 17). Now judged at the player's centre (test 18: 0 lifts).
- **Grids centred where the player was:** fast players outran them (test 20). Now centred ahead.

## Ideas not tried yet

For the HUD's alpha (in the order I'd suggest):
1. **Alpha from two backgrounds, alternating frames (recommended).** Proven right on HOLOHUD (test
   34), needs no knowledge of how any addon draws, and covers VGUI panels (hints, chat, menus) too.
   Each Garry's Mod frame: keep the hands/weapon picture right after they're drawn (their own alpha
   works, test 30), then clear to black on even frames and grey on odd ones before the HUD; Skyrim
   combines the newest black/grey pair in linear light. The flicker doesn't matter (the Garry's Mod
   window will be out of sight, step 2d). Cost: HUD at half Garry's Mod's frame rate; a one-frame
   wrong edge where the HUD changes between the two frames. Differs from SkyCraft (one picture).
2. **Both backgrounds in one frame, for HOLOHUD only:** draw HOLOHUD's finished output twice into
   two pictures of our own (black, grey) in the same frame. No motion error. But test 37 showed
   HOLOHUD's plates come out differently in our own picture than on screen, so first find out why.
3. **See Direct3D 9's real states per draw:** a call trace (apitrace's d3d9 wrapper) would show
   exactly which blend / alpha-write states each draw uses, and explain both mysteries. Needs a
   download and a `d3d9.dll` placed next to `gmod_win64.exe` (a write into the game folder: only
   with the user's go-ahead).
4. **Fix the device search** (a Direct3D 9Ex reference device) to read those states from our module.
- (Collision) Faster long view rays on the client: walk cells along the ray (DDA).
- (Collision) Physics props on Skyrim's ground: `PhysicsFromMesh` pieces only *far* from the player.

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
  `gmod/autorun/gmsky_autoload.lua`. Phase 2a: `overlay.lua` (`gmsky_test_overlay`), `overlay2d.lua`
  (`gmsky_test_alpha2d`), `gmod/module/src/d3d.rs` (`gmsky.D3DState`, read only, and the device
  search). Phase 2b: `liveoverlay.lua` (`gmsky_overlay_probe`), `gmod/module/src/present.rs`
  (the `Present` hook).
- Tests: `gmod/harness/src/main.rs` (78 checks), `protocol/test/`.
- Tools: `tools/overlay_read.py` (newest picture in the overlay block), `fake_overlay` (fake picture writer), `tools/overlay_preview.py`, `tools/alpha2d_report.py`, `tools/launch_gmod.ps1`, `tools/install_gmod_stubs.ps1`, `tools/install_gmod_autoload.ps1`.
