# Gmod x Skyrim: Design

> Play Skyrim as a Garry's Mod player. Garry's Mod movement, weapons, health and HUD, inside the
> real Skyrim world, fighting real Skyrim NPCs.

Status: draft v0.1 · 2026-10-04 · nothing below is built or tested yet.

This copies the architecture of [SkyCraft](https://github.com/chasmlol/SkyCraft) (a local copy is in
`reference/SkyCraft`, read-only, not part of this project). Where we differ from SkyCraft, this doc
says so and why.

---

## 1. Core principle

**Neither game is rewritten.** Both games run at the same time, each with its own mod:

- **Garry's Mod** runs its own game logic: player movement, collision, weapons, damage, health, HUD.
- **Skyrim** runs its own world: terrain, buildings, NPCs, AI, quests, dialogue, saves.

The two mods only **translate**:

- Skyrim tells Garry's Mod *what the world is shaped like*, *where the NPCs are*, and *what keys you pressed*.
- Garry's Mod tells Skyrim *where the player is*, *what the player hit*, and *what to draw on top*.

If we ever re-implement a Garry's Mod mechanic in the Skyrim plugin (or the reverse), the design has
gone wrong.

**Garry's Mod is authoritative for the player** (it decides where the player is and how much health
they have). Skyrim's player becomes a puppet that follows it.

## 2. Target environment

| Thing | Value | Notes |
|---|---|---|
| Skyrim | Anniversary Edition **1.7.104** | Same version SkyCraft was built on |
| Skyrim loader | SKSE64 for 1.7.104 + Address Library (`versionlib-1-7-104-0.bin`) | Installed by the user, verified present 2026-10-04 |
| Garry's Mod | 64-bit (`gmod_win64.exe`), September 2026 update | Uses DirectX 9 (`shaderapidx9.dll`) |
| Garry's Mod loader | Built in: Lua addons + binary modules in `garrysmod/lua/bin` | No script extender needed |
| Skyrim plugin language | **C++** with CommonLibSSE-NG (approved by user 2026-10-04) | Same as SkyCraft |
| Garry's Mod module language | **Rust** (stable, MSVC) | Small hand-written bindings to `lua_shared.dll` instead of a big dependency |
| Toolchain found | Visual Studio 2022 Community + Build Tools, CMake 4.4, Git, Python 3.12 | **Rust is not installed yet.** vcpkg not found |

Glossary:
- **SKSE plugin**: a DLL that Skyrim's script extender loads into the game. It can call any game function.
- **Binary module**: a DLL that Garry's Mod loads when a Lua script calls `require("name")`.
- **Lua**: the scripting language Garry's Mod mods are written in.
- **Shared memory**: a block of RAM that two programs both open by name, so they can pass data
  without files or network.

## 3. Safety boundaries (from AGENTS.md)

- **Offline only.** Garry's Mod is always started with `-insecure`, its own launch option that
  turns VAC (Valve Anti-Cheat) off and blocks joining secure servers. We only use single-player.
  We never host, join, or touch the server browser, Workshop, or Skyrim's Creations menu.
- **No writing to game installs**, except:
  - `GarrysMod/garrysmod/lua/bin/`: the user allowed **one** write, so we must use it well (§10).
  - `Skyrim Special Edition/Data/SKSE/Plugins/`: the user allowed writing here at any time
    (2026-10-04). The build copies `gmsky.dll` there.
  - Anything else (for example Garry's Mod `addons`) needs the user's go-ahead first.
- **Logs go into this project's `logs/` folder**, not into `Documents/My Games`, so we never write
  outside the project.

## 4. Components

```
┌────────────── SkyrimSE.exe ───────────────┐          ┌──────────── gmod_win64.exe (single-player) ───────────┐
│  gmsky.dll (SKSE plugin, C++)              │          │  gmsky module (Rust) + Lua scripts                     │
│                                            │          │                                                        │
│  WorldExporter ─ ground shape near player ─┼────────▶ │  Collision chunks (invisible static physics meshes)    │
│  ActorMirror   ─ nearby NPCs ──────────────┼────────▶ │  Proxy entities (invisible, hittable)                  │
│  InputBridge   ─ keyboard / mouse ─────────┼────────▶ │  StartCommand hook writes the player's button/aim cmd  │
│  HitBridge     ─ "NPC hit you for X" ──────┼────────▶ │  player:TakeDamageInfo(...)                            │
│                                            │          │                                                        │
│  PlayerPuppet  ◀─ player pos / aim / pose ─┼───────── │  Garry's Mod player movement                           │
│  CameraDriver  ◀─ camera position / FOV ───┼───────── │  CalcView                                              │
│  DamageApplier ◀─ "you hit NPC for X" ─────┼───────── │  EntityTakeDamage on a proxy                           │
│  Compositor    ◀─ hands + HUD picture ─────┼───────── │  offscreen render target                               │
└────────────────────────────────────────────┘          └────────────────────────────────────────────────────────┘
                              shared memory  Local\GmodSkyrim_v1
```

## 5. Coordinates

Both games use **Z = up**, so unlike SkyCraft (Minecraft is Y-up) there is no axis swap. The only
differences are scale and where the origin is.

- **Scale:** match player heights. The Garry's Mod standing player hull is 72 units tall; the Skyrim
  player is roughly 128 units. So `gmod = skyrim × 72/128 = skyrim × 0.5625`. The exact Skyrim height
  is **not measured yet**. Phase 0 logs it and pins the number.
- **Yaw (left/right aim):** Skyrim stores it in radians, Garry's Mod in degrees, and the zero
  direction and sign may differ. Phase 0 pins this with a logged test, the same way SkyCraft did.
- **Floating origin (new, not in SkyCraft):** a Garry's Mod map can only be about 32,768 units
  wide. Skyrim's main worldspace is far bigger than that, even scaled down. So Garry's Mod works in a
  local frame centred near the player: `gmod = (skyrim − origin) × scale`. When the player gets
  more than about 8,000 units from the centre, we move the origin, teleport the Garry's Mod player
  by the same amount, and rebuild the collision. Both sides log every re-centre.

## 6. The Garry's Mod side

- Runs a normal **single-player** game on an empty map (start with `gm_flatgrass`, playing in its
  large empty air space above the floor; a purpose-built empty box map is a later option).
- NPC spawning off; time of day and weather come from Skyrim (later phases).
- **Collision:** the Skyrim plugin sends small blocks of ground shape (triangle meshes, per 16×16
  chunk around the player). Lua turns each into an invisible, frozen physics object
  (`PhysicsFromMesh`). Garry's Mod's own movement code then walks, jumps, crouches and slides on them
  unchanged. This replaces SkyCraft's "inject boxes into Minecraft's collision query" step.
  - Whether player movement collides smoothly with many mesh chunks is **not tested**. That is the
    first thing Phase 1 measures.
- **Frame rate matters even though Garry's Mod is hidden:** in single-player the server ticks run
  inside the game's frame loop. In the first in-game test, when Garry's Mod dropped to 20 fps
  (window not focused), its 66 ticks per second arrived in bursts of about 3, and a 60 fps reader
  saw only ~20 changes per second. So Garry's Mod must not be throttled in the background: this
  64-bit build has no `engine_no_focus_sleep`; the launch script sets `+fps_max_nofocus 300`
  (test 5: ~300 fps). Skyrim blends between ticks itself (Phase 1 step 2).
- **Input:** Skyrim keeps the keyboard focus. Garry's Mod receives key and mouse data through shared
  memory and writes it into the player's command each tick with the `StartCommand` hook, so the
  Garry's Mod window does not need focus.

## 7. The Skyrim side

Copied from SkyCraft's design (`reference/SkyCraft/docs/DESIGN.md` §6–8):

- **PlayerPuppet:** turns off Skyrim's own player movement and moves the Skyrim player and its
  collision capsule to Garry's Mod's position every frame. NPCs still see, chase and hit the player
  normally.
- **CameraDriver:** forces first person and sets Skyrim's camera from Garry's Mod's view.
- **WorldExporter:** collision rays around the player at first (SkyCraft's "stage A"), real Havok
  shapes later (Havok is Skyrim's physics engine).
- **ActorMirror / DamageApplier / HitBridge:** NPC positions out, hits in, using Skyrim's own hit
  system so NPCs react, bleed and get angry.

## 8. Combat

- Each Skyrim NPC near the player gets an invisible **proxy entity** in Garry's Mod with the same
  size. Crowbar swings, bullets and explosions hit it using normal Garry's Mod code.
- `EntityTakeDamage` on a proxy cancels the damage there and sends `HitActor {formId, damage, ...}`
  to Skyrim. (A **FormID** is Skyrim's ID number for a character or object.)
- When an NPC hits the Skyrim puppet, Skyrim cancels it and sends `PlayerHurt`. Garry's Mod applies it
  to the real player. **Garry's Mod health decides death.**
- Damage scaling between the two games is an open question (§14). It will be a config value.

## 9. Drawing

Same plan as SkyCraft: Skyrim draws the world, and Garry's Mod draws **only its own things**
(hands, weapon, HUD, later props) into an offscreen picture that Skyrim draws on top.

- Garry's Mod uses **DirectX 9** and Skyrim uses **DirectX 11**. Sharing the picture directly on the
  graphics card only works if Garry's Mod's DirectX 9 device is the "9Ex" kind. **Unknown; Phase 2
  checks it.** The fallback is copying the picture through normal memory (slower, roughly one frame
  of delay).
- Until Phase 2, the Garry's Mod window simply stays visible next to Skyrim for debugging.

## 10. Installing into Garry's Mod with one write

We may write to `garrysmod/lua/bin` **once**, but the module will be rebuilt many times. So the one
write installs two tiny, stable **loader stubs**:

- `gmsv_gmsky_win64.dll` (server side) and `gmcl_gmsky_win64.dll` (client side)

When Garry's Mod loads a stub, it copies the real module (`target/release/gmsky_module.dll`) to
`build/gmod/loaded/gmsky_module_<process id>.dll`, loads that copy and passes all calls through.
The copy means the real module can be rebuilt while Garry's Mod runs (the new build is picked up on
the next game start). The stub learns which side it is from its own file name (`gmcl_` = client).
The project folder is fixed into the stub when it's built; the `GMSKY_ROOT` environment variable
overrides it if the project ever moves. Installed 2026-10-04 by `tools/install_gmod_stubs.ps1`. The real module then runs this project's Lua files from `gmod/lua/`.
After the one write, every rebuild happens inside the project and the game folder is never touched again.

**Starting it without an addon file:** after the map loads, type `lua_run require("gmsky")` in the
console. That loads the server stub; the server side then tells the client side to load its stub
(`SendLua`), which works (in-game test 2026-10-04). `+lua_run` on the command line does nothing,
and `-hijack` (sending it to the running game) passes only one word; both dropped.

**Auto-loading (user's decision 2026-10-04):** a second one-time write, `gmsky_autoload.lua` in
`garrysmod/lua/autorun/server/` (master copy `gmod/autorun/`, installed by
`tools/install_gmod_autoload.ps1`). Single-player only, it requires gmsky once the player's game is
fully loaded (first real `SetupMove` command). Like the stubs it never needs changing.

(The stubs can't be swapped while Garry's Mod is running, so rebuilds need a Garry's Mod restart.)

## 11. Shared memory protocol

- One header file `protocol/gmsky_protocol.h` (C++) is the source of truth. A hand-written Rust copy
  (`#[repr(C)]` structs) mirrors it. Both sides have a test that checks every struct's size and
  field offsets, so they can't drift apart without a test failing.
- Layout: a **header** (magic number, protocol version, both process IDs, heartbeats), **latest-value
  slots** for per-frame data, and **two one-way queues** for events. Same as SkyCraft §10.
- If either side stops sending heartbeats, the other goes back to a safe state (Skyrim gives
  movement back to the normal player).

| Direction | Message | How often |
|---|---|---|
| S→G | `Hello / Heartbeat` | 1 per second |
| S→G | `Origin {worldspace/cell, origin xyz}` | On change |
| S→G | `CollisionChunk {chunk, triangles[]}` / `CollisionEvict` | Streamed |
| S→G | `ActorUpsert {formId, pos, yaw, box, hpFrac, flags}` / `ActorRemove` | 20 per second |
| S→G | `Input {keys, mouse dx/dy, wheel, buttons}` | Every frame |
| S→G | `PlayerHurt {...}` | Event |
| G→S | `PlayerState {pos, aim, pose, onGround, eye pos, fov}` | Every tick |
| G→S | `HitActor {...}` | Event |

## 12. Phases

Each phase ends with something you can test, and each step inside a phase is one revertable change.

| # | Phase | Done when |
|---|---|---|
| 0 | **Link** | Both mods connect over shared memory. Walking in Garry's Mod (on a temporary flat floor) moves the Skyrim player, and the logs show matching positions |
| 1 | **Walk Skyrim with Garry's Mod movement** | Collision chunks, camera, input bridge. You can run and jump around Whiterun with Garry's Mod movement |
| 2 | **Overlay** | Garry's Mod hands, weapon and HUD drawn inside Skyrim; Garry's Mod window hidden |
| 3 | **Combat** | Proxies, HitActor, PlayerHurt, death. You can fight a bandit camp with a crowbar |
| 4 | **Full world** | Real Havok shapes, interiors, load doors, floating origin across the whole map |
| 5 | **Polish** | Auto-launch Garry's Mod from Skyrim, time/weather sync, save/load consistency |

### Phase 0 steps

1. **Repo setup:** whitelist `.gitignore`, `README.md`, `MODLOG.md`, folder layout (§13).
2. **Protocol:** the C++ header, the Rust copy, layout tests on both sides, and the coordinate
   conversion with unit tests. Test: `cargo test` and the C++ test print sizes and pass.
   Coordinate conversion lives **only** in the C++ header, because only the Skyrim side converts.
   Garry's Mod always works in its own units.
3. **Fake Skyrim:** a small Rust program (`tools/fake_skyrim`) that opens the shared memory and
   prints what Garry's Mod writes, so we can test the Garry's Mod side alone. Rust instead of
   SkyCraft's Python so it reuses the protocol crate instead of a third copy of the layout.
4. **Garry's Mod module + stubs:** the module opens shared memory and writes `PlayerState` every
   tick, and logs to `logs/gmod.log`. Then the **one** write of the two stubs to `lua/bin`.
   Test: start Garry's Mod, run `lua_run require("gmsky")`, walk around, and the fake Skyrim
   prints positions that change as you move.
5. **Skyrim plugin, read-only:** loads, opens shared memory, and logs Skyrim's player position,
   height and yaw next to Garry's Mod's to `logs/skyrim.log`. Pins the scale and yaw conversion.
6. **Skyrim plugin, puppet** (`skse/src/Puppet.cpp`): Skyrim's player follows Garry's Mod's
   left/right/forward position and view direction. **Height comes from Skyrim's own ground** (a
   downward ray cast at that spot, falling back to terrain height), because Garry's Mod is still
   on a flat floor (user's choice 2026-10-04); full 3D comes with Phase 1 collision. It moves the
   player only while Garry's Mod is alive and anchored to the current origin; otherwise Skyrim keeps
   normal control. When Skyrim moves the player itself (load door, fast travel, more than 300
   units away from where the puppet put it) or the location changes, a new origin is published and
   the puppet waits for Garry's Mod to re-anchor. Testing needs Skyrim's own `bAlwaysActive=1`
   (set by the user in their Skyrim.ini) so Skyrim keeps running while Garry's Mod has focus.
   Phase 0 done when this works in game.

### Phase 1 steps (planned 2026-10-04)

Goal: play from the Skyrim window with Garry's Mod movement on Skyrim's real ground.

1. **Input bridge.** Skyrim keeps keyboard focus. The Skyrim plugin reads Skyrim's keyboard and
   mouse events (as SkyCraft does) and, while the puppet is on, (a) swallows them from Skyrim's own
   player controls (`PlayerControls` hook) and (b) writes a new **`GmskyInput`** latest-value slot:
   held actions as bits (forward, back, left, right, jump, duck, sprint, walk, attack, attack2,
   reload) and *cumulative* mouse x/y and wheel counts. Garry's Mod (server `StartCommand` hook)
   turns that into the player's command: buttons, forward/side move, and view angles. **Garry's
   Mod integrates the mouse itself** (Source formula: degrees = counts × 0.022 × sensitivity, with
   a `gmsky_sensitivity` setting), so Garry's Mod stays in charge of where the player looks.
   Skyrim keeps: Esc (menu), ~ (console), J (journal), M (map), F9 (quickload), and **E = Skyrim
   activate** (doors, talking, looting) via the crosshair target, as SkyCraft does with G.
   Garry's Mod must not slow down in the background: `client.lua` sets `engine_no_focus_sleep 0`.
   Protocol version 2 (new slot). Test: from the Skyrim window, walk, sprint, look, open a door.

   **1b. Mouse look without the round trip (planned 2026-10-04).** In-game test 5: looking is
   smooth but "floaty": it keeps going a little after the mouse stops. Cause: the mouse goes
   Skyrim → Garry's Mod (waits for the next tick, up to 15 ms) → back to Skyrim (waits for
   Skyrim's next frame, ~17 ms), so Skyrim shows a view that is 1–3 frames old (estimate).
   Fix, the usual "client-side prediction": Garry's Mod reports, with each player state, the
   Skyrim mouse totals it has already turned into its view (and the Skyrim input frame they came
   from, and its degrees per mouse count). Skyrim draws Garry's Mod's view **plus only the mouse
   movement Garry's Mod hasn't applied yet**, with the same formula and pitch limit. Garry's Mod
   still decides the view; Skyrim only shows the part already on its way there, so the two
   cannot drift apart (once Garry's Mod catches up, the extra is zero). `gmsky_predict 0` in
   Garry's Mod's console turns it off for comparison. Skyrim logs once a second how many frames
   old Garry's Mod's view is, and how far ahead the prediction drew it. Protocol version 3.
   Test: the view stops when the mouse stops; the log shows the lag in frames and the prediction
   gap back at ~0 when the mouse is still.

   **1c. Turn Skyrim's camera directly (planned 2026-10-04).** Test 6: prediction works (Skyrim's
   angle ran up to 22° ahead of Garry's Mod's), but with both windows overlaid Garry's Mod still
   turns first. So the remaining delay is after we set the angle, inside Skyrim. SkyCraft found
   two causes there: Skyrim's camera update runs *before* the player update in each frame (an
   angle set in the player update is shown one frame later), and Skyrim's first-person camera
   *eases* its rotation (~50 ms). Fix, as SkyCraft does: hook Skyrim's camera update (it is
   called directly, so its call sites are found by scanning the game's code) and, after it, set
   the camera's rotation from our view ourselves; set it again after the player update with the
   newest numbers. Skyrim's camera axis order is learned at run time first (240 frames of votes,
   as SkyCraft) and nothing is overridden until it is certain. First person only, not while a
   Skyrim menu is open. `gmsky_camera 0` in Garry's Mod's console turns it off. The log shows how
   far Skyrim's own camera lags our view (degrees), which is the easing measured.
   Not done here: the camera *position* (still one frame late when walking); Phase 1 step 4.
2. **Smooth position (revised 2026-10-04).** Garry's Mod moves the player 66 times a second,
   Skyrim draws 60 times a second, and the two aren't in step, so taking the newest tick each frame
   gives uneven steps (most frames one tick, some none, some two). The first plan, sending the
   position Garry's Mod's client draws each frame, judders the same way (SkyCraft tried it: the
   two games' frames aren't phase-locked). Instead, as SkyCraft does with Minecraft's ticks:
   Garry's Mod stamps each tick with the PC's precise clock (`QueryPerformanceCounter`, the same
   clock in both processes) and its tick length; Skyrim keeps the last few ticks, locks their
   times to the exact tick rhythm (tick numbers say how many ticks passed; the stamps' noise is
   filtered out), and draws the player at "now minus a small delay", blending between the two
   ticks around that moment. The delay is one tick plus how late ticks really arrive (last 2 s; ~17–25 ms),
   so the next tick has normally arrived when needed. Only the position; the view keeps step 1b/1c
   (no added delay). `gmsky_smooth 0` in Garry's Mod's console = newest tick as before.
   Protocol version 4. Test: walking, the per-frame steps in the log are even (low spread) with
   smoothing on and uneven with it off; "late frames" near 0.
3. **Collision, stage A** (planned in detail 2026-10-04). Skyrim casts a grid of downward rays
   around the player (SkyCraft's stage A) and sends the heights; Garry's Mod builds an invisible,
   frozen physics object from them (`PhysicsFromMesh`), so Garry's Mod's own movement walks, jumps
   and crouches on Skyrim's ground, hills and rocks. Then the puppet uses Garry's Mod's height and
   the Phase 0 ground ray goes. Stage A limits (as SkyCraft's): only the top surface of each
   column (no walking under arches or into buildings with roofs), thin things (fences) may be
   missed. Sub-steps, each tested on its own:
   - **3a. Garry's Mod-only test (no Skyrim).** Unknowns first: how much room gm_flatgrass has
     above its floor and to its walls (logged with traces from the player), whether the player
     stands and walks smoothly on a `PhysicsFromMesh` object, and how long building one takes.
     Garry's Mod predicts the player's movement on the client too, so the client must build the
     same object or the Garry's Mod window would stutter; our module is loaded on both sides, so
     later both read the same heights from shared memory. Console command `gmsky_test_terrain`
     builds a test hill (a known formula) under the player on both sides; the log compares the
     player's height with the formula once a second, and says what the player stands on.
   - **3b. Skyrim height grid.** The plugin casts 65×65 downward rays, 48 Skyrim units apart
     (about 3,100 units square), centred on the player, spread over several frames, and publishes
     the grid in a new latest-value slot (~17 KB; the map is 64 KB). A new grid when the player is
     more than 768 units from the last centre. Logs ray time and misses.
   - **3c. Garry's Mod builds it.** Both sides turn the newest grid into the physics object
     (converted to Garry's Mod units with the origin), replacing the old one only once the new one
     exists. The Garry's Mod player is placed high in the map's air space when it anchors, so
     Skyrim ground below the start stays reachable.
     **3a measured (test 10):** gm_flatgrass's open space is x −16,000…14,720, y −15,744…14,976,
     floor z −12,288, ceiling ~15,360 (Garry's Mod units; ≈58,000 × 58,000 × 52,000 Skyrim units).
     So the anchor goes near the middle, about (0, 0, 1,500): ±13,800 units of height either way
     (±26,000 Skyrim units). A 65×65 `PhysicsFromMesh` (8,192 triangles) builds in 18–25 ms on
     each side; the player stands and walks on it. Placement rule: above the highest point under
     the whole 32×32 footprint (the centre alone left the player stuck in a slope). Safety rule
     for 3c: with no Skyrim ground under the player yet, hold the Garry's Mod player in place
     rather than let it fall ~13,000 units to the map floor.
   - **3d. The puppet uses Garry's Mod's height**; the ground ray is removed. Test: jump, crouch,
     walk up and down hills; Skyrim z follows Garry's Mod z.
4. **Camera.** Skyrim's camera from Garry's Mod's eye (crouch height, view bob) and FOV.
5. **Floating origin re-centre** (§5): move the origin when the player is far from it.

## 13. Repo layout

```
docs/DESIGN.md
protocol/            gmsky_protocol.h + layout test
skse/                Skyrim plugin (C++, CMake, CommonLibSSE-NG)
gmod/module/         Garry's Mod module (Rust)
gmod/stub/           the two loader stubs (Rust, change rarely)
gmod/lua/            Lua scripts, run by the module
tools/               fake_skyrim.py, launch scripts
logs/                (ignored) runtime logs
build/               (ignored) build output
reference/           (ignored) SkyCraft clone, read-only
```

## 14. Open questions

1. ~~Installing the Skyrim plugin~~: answered, the build may write to `Data/SKSE/Plugins` (§3).
2. ~~Rust install~~: done by the user. Rust 1.99.0 stable, MSVC toolchain, verified 2026-10-04.
3. **CommonLibSSE-NG with Visual Studio 2022:** SkyCraft used Visual Studio 2026. Whether 2022 builds
   it is **not tested**. vcpkg (a C++ package manager CommonLib uses) is also needed, and would live
   inside this project in an ignored `.tools/` folder like SkyCraft does.
4. **License (only matters if this is ever published):** CommonLibSSE-NG (a git submodule in
   `skse/extern/`) is GPL-3.0-or-later with a "modding exception". The exception covers linking to
   the *game's* code, not ours, so a published `gmsky.dll` probably has to be GPL-compatible.
   Not a lawyer's opinion; check before any release. Personal use is unaffected.
5. **Damage scaling:** a Garry's Mod crowbar does 10 damage, but Skyrim bandits have 50–300 HP.
   This gets decided in Phase 3.

## 15. Risks

| Risk | What we do about it |
|---|---|
| Player movement on many mesh chunks is jittery or slow | Measure in Phase 1 with logged frame times; fall back to fewer, larger chunks or box shapes |
| DirectX 9 → 11 picture sharing doesn't work | Copy through normal memory first, measure the cost |
| Floating origin re-centres cause visible jumps | Re-centre only while the player is on the ground; log each jump |
| `+lua_run` on the command line doesn't work | Type it in the console once per session |
| Garry's Mod updates break the module | It only uses the stable Lua C functions; stubs never change |
