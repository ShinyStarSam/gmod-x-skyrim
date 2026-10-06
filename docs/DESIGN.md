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
  by the same amount (built as re-plan step C, §12: Garry's Mod moves the player and the anchor
  together; nothing is rebuilt). Every re-centre is logged.

## 6. The Garry's Mod side

- Runs a normal **single-player** game on an empty map: **`gm_flatmap`** (Workshop item
  3669476334, user's find 2026-10-04), started from the main menu (a Workshop map can't be started
  from the command line: "No such map"). Measured room (Garry's Mod units): x −16,320…16,320,
  y −16,320…16,320, floor −16,288, ceiling ~16,320: the engine's whole ±16,384 box, ≈61,000
  Skyrim units each way. Compared: gm_flatgrass ~30,700 wide × 27,600 tall; gm_largeemptyplane
  ~30,200 wide but only ~3,776 tall (≈7,100 Skyrim units; too low: one walk in test 11 covered
  19,000 Skyrim units of height). The player lives in the empty air in the middle of the box. Its size is
  logged on every load (`terrain: map … room …`); gm_flatgrass's was measured in test 10.
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

Skyrim draws the world. Garry's Mod's **hands, weapon and HUD** come over as a picture that
Skyrim draws on top (as SkyCraft's hand and GUI layers, `Overlay.cpp`).

**Garry's Mod's world objects (props, rockets, grenades, ragdolls, NPCs) are drawn by Skyrim
itself, as 3D geometry, in Skyrim's own frame** (decided by the user 2026-10-05, after tests 70-74).
This is what SkyCraft really does (`WorldRender.cpp`, `AvatarExporter.java`, `WorldExporter.java`):
Minecraft ships its triangles and textures, and Skyrim draws them with its own camera, depth
buffer and light, so they are locked to the world and hidden by Skyrim's walls. Correction: until
2026-10-05 this file said the picture + depth-picture "world layer" (W1-W5) was SkyCraft's way. It
was not; only its reading of Skyrim's depth buffer came from SkyCraft. Plan: §12, "Garry's Mod's
things drawn inside Skyrim (geometry route)".

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
| 5 | **Polish** | Auto-launch Garry's Mod from Skyrim, save/load consistency (Garry's Mod has no time of day or weather, so there is nothing to sync there; Skyrim's own sky and weather simply stay) |

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
     **3c as built (done 2026-10-04, test 16):** the ground is 4×4 pieces of 16×16 squares
     (512 triangles each), one `PhysicsFromMesh` entity per piece, one piece per tick, nearest
     the player first. Skyrim snaps grid centres to a lattice one piece wide, so a new grid
     shares 12 pieces with the old one; only the 4 new ones are built, and pieces outside the new
     grid are removed. **Rule: never build a piece under a moving player.** Garry's Mod's physics
     crashes then (a 16-bit list overflows while it works out what the new object touches;
     STATUS.md "Known engine bug"). Reuse keeps the piece under the player during normal
     movement; outrunning the grid is not handled yet.
   - **3d. The puppet uses Garry's Mod's height** (planned 2026-10-04). Skyrim's player z comes
     from Garry's Mod (blended like x/y, step 2) instead of Skyrim's own ground ray. The ray stays
     only as a measuring tool: once a second the log gives "Garry's Mod feet minus Skyrim's ground"
     (smallest/largest), which should be about 0 when walking and positive in a jump. Test: jump,
     walk up and down hills; Skyrim z follows Garry's Mod z. Crouching lowers only the eye, not the
     feet, so it shows with step 4 (camera), not here.
   - **3e. Noclip (V)** (planned 2026-10-04, user request). Garry's Mod's own noclip, started
     from the Skyrim window. Needs 3d (otherwise Skyrim would keep the player on its ground).
     - **Key:** V in Skyrim's window adds 1 to a running count of presses in `GmskyInput`
       (the unused `reserved0` becomes `noclip_presses`; protocol version 6). A count, not a
       held bit, so a quick tap between two Garry's Mod ticks is never lost. Garry's Mod toggles
       noclip once per new press the way its own `noclip` command does: it asks the
       `PlayerNoClip` hook (the gamemode's permission check; always yes in single-player
       Sandbox) and switches the move type between noclip and walking. Logged on both sides.
     - **Crash guard (also closes the "outrunning the grid" gap):** noclip flies far faster than
       walking, so the player will often be over a ground piece that isn't built yet. Rule from
       the crash investigation: never build a piece under a moving player. Before building a
       piece whose square (plus 48 units) contains the player: in noclip, skip it for now (a
       flying player needs no ground; it is built once the player is no longer over it, or once
       noclip ends); walking, hold the player still with its feet at least 40 units above the
       ground near it for 3 ticks, then build, then let go (grid 1 is built like this on every
       run and never crashed; test 14 crashed with the player held with its feet *on* the new
       surface, hence the 40 units). Holds end at once when noclip starts.
     - **Ground while flying:** Skyrim's rays run from 4,000 units above the player's feet to
       4,000 below, so a player flying high would get a grid of misses. Rays now reach 20,000
       below (they stop at the first surface anyway), and a new grid is cast when the player's
       height moved more than 2,000 units from the last grid's. In Garry's Mod a piece that had
       misses is rebuilt (under the same guard) when a newer grid has more ground there.
     - **Skyrim side:** the puppet already puts the player wherever Garry's Mod says each frame.
       Not known yet: whether Skyrim's own physics pushes the player out of rocks and walls
       while it flies through them. The log measures it ("drift" in noclip); turning Skyrim's
       player collision off is the next step only if the numbers show it is needed.
     - Known limit: ending noclip more than ~2,100 Garry's Mod units under Skyrim's ground leaves
       the player held in place (no ground found above it within reach); press V again.
     Test: press V, fly up, through a hill and far away, press V again over open ground: the
     player lands and walks; Garry's Mod does not crash; logs show the toggles, skipped/guarded
     pieces and Skyrim z following Garry's Mod z.
   - **3f. Skyrim's player body, as SkyCraft does it** (planned 2026-10-04, from reading
     SkyCraft's `Game.cpp` puppet and `DigPhysics.cpp`; adapted from SkyCraft, MIT). With 3d,
     Skyrim's body must not fight Garry's Mod's height:
     - **On ground / in air:** each frame Skyrim's character controller is told what Garry's Mod
       says (`GMSKY_PF_ON_GROUND`): on the ground (with "support") or in the air. Skyrim's
       gravity stays on, because SkyCraft found Skyrim's AI won't melee a target it thinks is
       in the air (they tried zero gravity first and undid it).
     - **Contacts:** Skyrim's body collects contact points with the world and pushes itself out
       of them. Contacts above the player's feet (a wall at the head, a rock it was put inside)
       are dropped, so only what's under its feet holds it, as in SkyCraft. In noclip all its
       contacts are dropped (it flies through everything). Without this Skyrim would shove the
       body away from where Garry's Mod put it.
     - **Land rescue:** Skyrim puts any actor that is more than 30 units under the land back on
       top. For our player (only while the puppet is on) that is switched off: a noclip player
       under the ground would otherwise be lifted, and a lift of more than 300 units looks like
       "Skyrim moved the player" (a load door) and resets the origin. Same hook spot as SkyCraft
       (checked on 1.7.104: the land-height call in the actor movement update).
     Logs per second: contacts dropped, rescues skipped. Test: as 3e; `max-drift` near 0 in
     noclip inside a hill; no "Skyrim moved the player" while flying under ground.
4. **Camera** (planned 2026-10-04). Skyrim's camera from Garry's Mod's eye and FOV.
   **Read in SkyCraft first** (`skse/src/Game.cpp`: `PinCameraAndSky`, `ApplyMcFov` /
   `RestoreFov`, the eye height blended between ticks, the walk bob): every frame the camera root
   (and the sky) are put at feet + eye height, after Skyrim's camera update and again after the
   player update, together with the rotation (our step 1c already turns it there); Skyrim's FOV is
   saved, set from the other game's, and restored when it stops driving.
   - **Eye position:** the puppet's feet this frame (already blended, step 2) plus Garry's Mod's
     eye height (`EyePos().z - GetPos().z`: 64 standing, 28 crouched, with Source's smooth duck
     transition), blended between ticks the same way, converted to Skyrim units (64 ↔ 120.5,
     Skyrim's own standing eye). Placed in the same two spots as the rotation, only while we turn
     the camera (first person, no Skyrim menu, `gmsky_camera 1`).
   - **No walk bob:** unlike Minecraft, Garry's Mod (Sandbox) doesn't bob the camera, only the
     weapon model (Phase 2). What it does have is **view punch** (`GetViewPunchAngles`: the kick
     from a hard landing, later weapon recoil); it is sent and added to the camera's pitch, yaw
     and roll (newest tick, not blended: it is small and decays by itself).
   - **FOV:** Garry's Mod's `GetFOV()` (its `fov_desired` when 0) is, like Skyrim's
     `worldFOV`, the horizontal field of view of a 4:3 view, so it is copied as is. Skyrim's own
     value is saved first and restored when the puppet stops.
   - **Protocol 9:** `GmskyPlayerState` gets `punch_deg[3]` (pitch, yaw, roll).
   - **Not here:** hiding Skyrim's first-person arms is Phase 2.
   **4b. Crouch = Skyrim's sneak** (planned 2026-10-04, user request; adapted from SkyCraft's
   `SyncSneak`, MIT). Garry's Mod's crouch (`GMSKY_PF_CROUCHING`, already sent every tick) makes
   Skyrim's player sneak, so NPC detection, the stealth eye, sneak attacks and the Sneak skill all
   follow it. Skyrim sneaks through the same action its Sneak key uses
   (`PlayerControls::DoAction(ActionSneak)`, `REL::ID(42350)` on 1.7.104 as SkyCraft found); it
   toggles, so it is sent whenever the two games disagree, at most every 0.3 s (if Skyrim's
   animation didn't take it, e.g. mid-jump, it is tried again). Not while dead, mounted or
   swimming. Logged: each toggle (taken / refused) for the first 20, and per second the state of
   both games. Test: crouch near an NPC: the stealth eye shows; stand: it goes.
   - **Logs** (each second): eye height Garry's Mod / Skyrim units, camera placed counts, the
     camera's height above the feet after placing (must equal the eye height), largest view punch,
     FOV in use and the saved one.
   Test: stand, crouch, jump, land from a height (punch), change `fov_desired`; the log's camera
   height follows Garry's Mod's eye height.
5. **Floating origin re-centre** (§5): move the origin when the player is far from it.

### Phase 1 re-plan after test 17 (proposed 2026-10-04, waiting for the user's go-ahead)

User's direction: skip fixing the ray-grid ground, do step 5 next, get to Skyrim's real collision
shapes as soon as possible, checked against SkyCraft.

**What SkyCraft does that we don't** (read in `reference/SkyCraft`: `docs/DESIGN.md` §5.1,
`skse/src/Collision.cpp/.h`): Minecraft never gets physics *objects* for Skyrim's world. A Mixin
adds Skyrim's shapes (boxes) to Minecraft's collision *query*, so Minecraft's own movement code
runs unchanged against them. Our crash (STATUS.md "Known engine bug", 5 crashes) is in Garry's
Mod's physics engine creating a new physics object near the moving player. Real Havok shapes
built with `PhysicsFromMesh` would hit the same crash, and step 5 (moving the origin) means
rebuilding all ground, which would hit it too. So the crash is not something we'd "replace
anyway": it comes with how Garry's Mod gets the collision, not with where the data comes from.

**Garry's Mod's version of SkyCraft's trick:** an entity with custom collision
(`EnableCustomCollisions(true)`, a box solid type, no physics object) has its Lua
`ENT:TestCollision(start, delta, isbox, extents, mask)` asked whenever a trace (player movement,
bullets) crosses its bounds; it answers with the hit position, fraction and normal. Garry's Mod's
player movement is made of box traces, so it walks, jumps, steps up and slides on whatever Lua
answers. No physics object is created, so the crash's code is never reached. **Not known yet**
(the first step measures it): whether player movement traces really reach `TestCollision` for
this setup, the client's prediction (the client must answer the same), and the Lua cost per tick.
Known limit: physics props and ragdolls in Garry's Mod won't collide with it (traces only).

Steps, each tested on its own:
- **A. Collision field experiment (Garry's Mod only, like 3a; go-ahead 2026-10-04).** Real
  triangles, not boxes. User's concern: boxes would feel terrible around trees and fences.
  SkyCraft agrees: its final player collision (`fabric/.../world/TriCollider.java`) runs against
  Skyrim's exact triangles because boxes made slopes stair-stepped; boxes stay only for mobs and
  items. Garry's Mod needs less than SkyCraft did: its movement already copes with slanted
  surfaces (map brushes), so Lua only answers "where does this moving box first touch these
  triangles" exactly (a swept separating-axis test: 3 box axes, the face normal, 9 edge
  crosses) and Garry's Mod's own walking, sliding and step-up do the rest. SkyCraft had to write
  its own resolver (step-up, wall sliding) because Minecraft's only handles boxes.
  `gmsky_test_field` builds a test scene under the player on both sides: the 3a hill (8,192
  triangles), a flat floor with a post, a fence, a 12-sided tree trunk, stairs (12-unit steps)
  and a 50° ramp (too steep to walk). New file `gmod/lua/gmsky/field.lua`; entity
  `gmsky_field` (box solid, custom collisions, no physics object). Logged on both sides each
  second: `TestCollision` calls (box / ray), hits, Lua ms, slowest call, what the player
  stands on, feet minus the hill. First calls log their arguments, because whether `startpos` is
  the box's centre or its bottom isn't documented clearly (Garry's Mod wiki example treats it as
  the bottom; `gmsky_field_feet 0` switches to centre). Test: walk/sprint/jump/crouch over the
  hill, around the post, fence and trunk, up the stairs, against the ramp.
- **B. Ray grid into the field** (built 2026-10-04 after test 19 showed A works: player movement
  calls `TestCollision`, `startpos` is the bottom of the box, 0.03–0.3 ms of Lua per tick). Each
  new grid becomes the triangles of one persistent `gmsky_field` entity (not boxes: test 19).
  `PhysicsFromMesh`, the pieces, the crash guard and its holds, the 3a hill and the replay tool
  are removed. The "no ground" hold stays, and gives the player its speed back when it ends.
  **B fixes after test 20** ("strange hitches when launching myself horizontally"): at ~1,300
  units/s the player outran the grid 14 times (each grid takes 0.3–0.5 s to cast and was centred
  where the player was when casting *started*) and was frozen 3–25 ticks; and building 8,190
  triangles took 7–20 ms per grid on each side. Fixes: (1) Skyrim centres a new grid where the
  player will be when it is done (its velocity × the recent cast time + 0.1 s, snapped to the
  lattice); (2) above 1,000 Skyrim units/s the ray budget is 4 ms a frame instead of 1.5;
  (3) Garry's Mod keeps the grid's heights and makes a square's two triangles only when a trace
  first touches it (no build cost); long traces are tested in 128-unit sections in order, so a
  long view ray doesn't scan the whole grid. The hold stays as the last resort. SkyCraft's way
  (small regions streamed nearest-first, continuously, in a wider radius; freezing only after a
  teleport) is how step D will stream.
- **C. Step 5, moving the origin** (planned 2026-10-04; works, test 22). With the field, a re-centre only shifts
  numbers: no objects are rebuilt, nothing is waited for, the player keeps its speed.
  **Checked in SkyCraft first:** it has no floating origin (Minecraft worlds are big enough).
  What carries over is its teleport rule: Skyrim never uses Minecraft's position until Minecraft
  has acknowledged the newest teleport number (`teleportSeq` / `teleportAck`, `Game.cpp`), so the
  two sides never mix numbers from two frames. Our player state already carries its own anchor
  in every tick, which gives the same guarantee for the player; the plan below extends it to the
  ground.
  - **Who decides:** Garry's Mod (it knows the map's size; it owns the player). Skyrim's origin
    and epoch stay the same: a re-centre is not a new origin, only a new **anchor** (the Garry's
    Mod point that matches Skyrim's origin). Since `skyrim = origin + (gmod − anchor) / scale`,
    moving the player and the anchor by the same amount leaves the Skyrim position unchanged.
  - **When:** once the player is more than 8,000 Garry's Mod units (≈15,000 Skyrim units) from
    "home" (the middle of the map's open space, measured at start: gm_flatmap has ±16,300 every
    way) along x, y or z. At any time, also in the air and in noclip (test 17 reached the wall in
    noclip). §15's "only on the ground" was there to hide a visible jump; in the Skyrim window
    there is none (the Skyrim position doesn't change), and the Garry's Mod window isn't played.
  - **What moves, in one server tick, before the player state is written:** the player
    (`SetPos(pos + d)`, d = home − pos in whole units, so the float position stays exact); the
    anchor (new module function `gmsky.ShiftAnchor`); the ground entity (below); a hold position.
    Velocity is kept (logged before and after to prove it).
  - **Ground grids become anchor-relative (protocol 7, same layout):** Skyrim now writes
    `(skyrim − origin) × scale`, i.e. Garry's Mod units relative to the anchor, instead of
    Garry's Mod world units. Otherwise a grid cast before a re-centre and arriving after it would
    sit in the old place (the race SkyCraft's acknowledgement avoids). Garry's Mod puts the one
    ground entity at the anchor and its triangles are relative to the entity, so a re-centre
    moves one entity (no physics object: it is a plain position change). The client follows the
    networked entity position. The step A test scene is built relative to its entity the same way.
    Step D's streamed shapes will use the same anchor-relative units.
  - **Skyrim's smoothing** (step 2) blends player positions relative to each tick's own anchor,
    so a re-centre between two ticks doesn't show as a 8,000-unit blend.
  - **Not moved:** other Garry's Mod entities (props, NPCs). None are used yet; noted for later.
  - **Logs:** Garry's Mod: `origin: RE-CENTRE #n` with the player's position before/after, the
    shift, the anchor before/after, speed before/after, and the Skyrim position before/after
    (must match). Skyrim: the puppet's `big-steps` stay 0 and its `max-step` stays small through
    a re-centre. `gmsky_recentre_at` (default 8000) sets the distance; a small value (e.g. 500)
    makes it easy to trigger while walking for the test.
- **D. Skyrim's real shapes (SkyCraft's stage C, ported; planned in detail 2026-10-04; D1–D3 built; works, test 23).** The
  ray grid only sees the top surface every 48 units: posts and rocks become spikes, and there are
  no walls, arches, roofs or interiors. Instead, Skyrim sends Garry's Mod the exact triangles of
  its collision around the player.
  **Read in SkyCraft first** (`skse/src/Collision.cpp/.h`, `Link.cpp` `WriteCollision`,
  `protocol/skycraft_protocol.h` collision ring, `fabric/.../world/SkyCollision.java`, `SkyTri.java`,
  `TriCollider.java`):
  - Skyrim's world is cut into cube **regions**; each frame, the main thread walks the loaded
    Havok world under its read lock for a few regions, nearest the player first, and copies out
    every shape touching the region; a worker thread turns them into triangles and writes one
    message per region into a **byte ring** in shared memory (32 MB in SkyCraft). A region message
    replaces that region's triangles. A "clear" message with a new number (epoch) drops everything
    (a new world/cell).
  - Layers taken: static, anim static (doors), transparent, trees, props, terrain, ground,
    invisible walls, stair helpers. Shapes decoded: MOPP/BV trees (queried with the region's box),
    lists, collections, compressed and extended meshes, triangle collections, convex lists,
    triangles, boxes, capsules/spheres (as boxes), convex hulls (from their planes), convex and
    plain transforms; anything else convex → its bounding box. Every Havok read is guarded
    (`__try`), and a fault skips that shape.
  - Regions next to the player are sent again every second (doors open, things move).
  - SkyCraft's final player collision uses these exact triangles (its voxels are for Minecraft's
    blocks and lighting, which we don't need). Stair helpers (Skyrim's invisible ramps over
    stairs) are ramps Skyrim's own player walks on.
  **What we take, what we change:**
  - Taken as is (adapted, credited): the region streaming, layer list, shape decoding, triangles
    from boxes/capsules/hulls, the guarded reads, the byte ring with pad messages, the clear epoch,
    the 1 s refresh near the player, SkyCraft's harvest limits per frame (3 regions, 2.5 ms).
  - Not taken: voxels, digging, materials, contact-shadow boxes, Minecraft's axis swap (both of our
    games are Z-up).
  - **Regions are 512 Skyrim units** (≈ 272 Garry's Mod units; SkyCraft: 8 blocks = 560), ±5
    around the player horizontally, 3 below, 2 above (as SkyCraft): 726 regions, ±2,800 units.
  - **Order: what the player will reach first.** First the 3×3×3 regions around the player, then
    the rest nearest-first around the point the player will be at in 0.5 s (our velocity
    look-ahead from step B). Above 1,000 units/s the limits double (6 regions, 5 ms), like the
    grid's fast budget.
  - **Units:** triangles are sent in Garry's Mod units relative to the anchor (like the grids, so
    re-centres need nothing). Regions are numbered in Skyrim's world (they don't move on a
    re-centre); a new origin epoch sends "clear".
  - **Stair helpers** are sent as normal solid triangles (Skyrim's own player collides with
    them, so Garry's Mod walking them matches Skyrim), with a flag so they can be told apart later.
  - **Garry's Mod side, in Rust, not Lua:** a building can have thousands of triangles; turning
    them into Lua tables each frame and testing them in Lua would cost too much (step A: 0.02 ms
    per sweep with a 64-unit cell lookup over ~8,000 triangles). The module drains the ring into
    a triangle store (per region, with a 64-unit cell lookup like `field.lua`) and does the swept
    test itself: `field.lua`'s swept separating-axis test ported line by line, with the same
    rules (stop 1/32 short; a box already overlapping may move out or along, not further in; long
    traces in 128-unit sections). Server and client share one module in the same process
    (`gmod/stub` loads one copy per process), so one store answers both. Garry's Mod Lua only
    calls `gmsky.Sweep(...)` from `ENT:TestCollision`. The Lua version stays (test scene, ray
    grid); the harness checks both give the same answers.
  - **Waiting for collision** (SkyCraft's `holdUntilReady`): when the region under the player's
    feet (and the one below) hasn't arrived yet, the player is held, as with "no ground" now;
    released when they arrive. Not while in noclip.
  - Garry's Mod drops regions more than 8 regions from the player; Skyrim forgets that it sent a
    region once it is more than 6 away, so anything Skyrim believes Garry's Mod has, it has.
  - **Switch:** `gmsky_ground shapes` (default) or `gmsky_ground grid` (the step B ray grid, for
    comparison). Garry's Mod tells Skyrim which one it wants with a player-state flag
    (`GMSKY_PF_SHAPES`); Skyrim does only that one.
  - **Protocol 8:** new byte ring (16 MB, after the 64 KB of slots), shared-memory name
    `Local\GmodSkyrim_v2` (the size changes, and an old process must not open the new block).
    Messages: pad, clear {epoch}, region {rx, ry, rz, epoch, count, flags} + triangles
    {9 floats, flags}. Layout checked by both layout tests.
  - **Logs:** Skyrim, each second: regions harvested, triangles, harvest ms, worker ms, ring bytes
    and "ring full" waits, faults; the first time each shape type is seen; per region near the
    player: triangle count. Garry's Mod: messages read, regions and triangles stored, ring bytes
    behind, sweeps / hits / µs / slowest, whether the region under the player has arrived, holds.
  - Sub-steps, each tested on its own: **D1** the ring and the module's store (harness: fake
    Skyrim messages, wrap-around, clear, replace, evict); **D2** the Rust sweep and the entity
    using it (harness: same answers as the Lua sweep; a box falls onto a region's floor; holds
    until a region arrives); **D3** Skyrim's harvester (`skse/src/Collision.cpp`, built only;
    the in-game test is the real check). In-game test: walk into a building's wall, up stairs,
    under an arch, into an interior (a load door: new origin epoch), past trees and fences.

### Phase 2 steps: the overlay (planned 2026-10-04)

Goal: Garry's Mod's hands, weapon and HUD (health, ammo, crosshair, tool gun screens, chat)
drawn inside Skyrim's window, and the Garry's Mod window out of the way.

**Read in SkyCraft first** (`skse/src/Overlay.cpp`, `fabric/.../client/FrameExporter.java`,
`protocol/skycraft_protocol.h` overlay slots, its DESIGN §9): what SkyCraft shipped is the
"through normal memory" path, not GPU sharing. Minecraft renders hand + HUD + screens on a
**transparent** background into its main render target, copies it back from the graphics card
asynchronously (a few staging buffers; a frame is shipped once its copy has finished, typically a
frame later) into one of **three picture slots** in shared memory (up to 3840 × 2160 × 4 bytes
each; a "triple buffer", so the writer never waits for the reader and the reader always gets a
whole picture). Skyrim hooks its swap chain's `Present` (the call that shows a finished frame;
patched in its function table), uploads the newest picture into a texture (`Map` with
`WRITE_DISCARD`) and draws it over the whole screen in one triangle with premultiplied alpha
(colour already multiplied by its transparency). It skips the overlay while a Skyrim menu is open
and hides Skyrim's own first-person arms and weapon (`HideFirstPersonMeshes`).

**What is different for Garry's Mod (the unknowns, measured first):**
- **Transparency:** Source does not promise a meaningful alpha (transparency) channel when it
  draws models: depending on the shader it writes 1, 0 or depth into it. If the alpha can't be
  trusted, the fallback is **two-background matting**: draw the overlay once on black and once on
  white; where they differ, the pixel is partly see-through (alpha = 1 − (white − black)), which
  gives exact transparency for any shader, at twice the drawing cost.
- **Drawing only our things:** Garry's Mod normally draws the world, then the view model (hands
  + weapon), then the HUD. We need the last two alone, at Skyrim's resolution. Lua can draw into
  a render target (an offscreen picture): the view model and hands with `cam.Start3D` +
  `DrawModel` (and the weapon's own view-model hooks), and the HUD with `render.RenderHUD` plus
  the `HUDPaint` hooks. Whether that matches what Garry's Mod shows is the experiment.
- **Getting the picture out:** Lua can't read a render target quickly (`render.Capture` encodes a
  PNG). The module can: it runs inside Garry's Mod's process, finds the Direct3D 9 device, and
  copies the render target to normal memory (`GetRenderTargetData`), then into shared memory.
  Which Direct3D 9 Garry's Mod uses (plain 9, 9Ex, or a translation layer such as DXVK) is
  logged first; GPU sharing (9Ex) stays a later improvement.
- **Garry's Mod is single-player with `-insecure`** (VAC off through the game's own switch), so a
  module looking at its own process's Direct3D device touches no anti-cheat (AGENTS.md rule 5).

**Found in 2a (tests 25-30):** Garry's Mod draws with Windows' own Direct3D 9 (no DXVK). Drawing
the view model or a second view (`render.RenderView`) from `PostRender` gives nothing / only the
sky. What works: in Garry's Mod's **own frame**, clear the screen to transparent at the first
`PreDrawViewModel` / `PreDrawPlayerHands` and capture at the end of its 2D drawing (`DrawOverlay`:
that includes the HUD, the crosshair and Garry's Mod's panels: notifications, chat, menus). By
default Source writes depth, not transparency, into the alpha channel (13-90 on solid pixels);
with `render.SetWriteDepthToDestAlpha(false)` the 3D view model's alpha is right in one frame, but
the 2D parts (HUD panel, crosshair, panels) write none (test 30).

**Kept in line with SkyCraft (re-checked 2026-10-04, user request):** the target is SkyCraft's
pipeline, step for step:
| SkyCraft | Garry's Mod |
|---|---|
| Minecraft doesn't draw its world (`LevelRendererMixin` cancels it): hand + HUD on a transparent background | The world is wiped at the view model (same result). Later, to save its cost: not drawing it at all |
| Minecraft writes correct alpha itself; one frame | One frame, Garry's Mod's own alpha: depth-in-alpha off (3D right); the 2D parts made to write it (test 31: forced alpha writes with "over" blending of the alpha channel) |
| Async GPU readback (`FrameExporter`: a few staging buffers, ship the newest finished) | The same in the module (Direct3D 9 `GetRenderTargetData` into system-memory surfaces, used a frame later) |
| Three picture slots in shared memory | The same |
| Skyrim `Present` hook draws it with premultiplied alpha, skipped in menus; Skyrim's arms hidden | The same |
The black/white pair (two-background matting) is **only a measuring tool** for the experiment: it
needs two frames (wrong edges when the hands move, a flickering window), which SkyCraft doesn't
do; it is used only if Garry's Mod's 2D parts can't be made to write alpha.

**2a after test 31: why the 2D parts write no alpha (planned 2026-10-04; of the troubleshooting
ideas listed after test 31 the user chose 1 (read Direct3D's settings), 2 (Lua test patches) and
3 (read VRMod) first, and likes 5, the native fix below VGUI (STATUS's first idea), if it also
covers Lua-drawn UI).**

*Idea 3, read VRMod (done, research only).* VRMod (Garry's Mod's VR addon) has the same problem: it
shows Garry's Mod HUD and menus on see-through panels. Read in its maintained x64 fork
(https://github.com/Abyss-c0re/vrmod-x64, `lua/vrmod/ui/cl_hud.lua`, `cl_ui.lua`; no licence
stated, so read only, nothing copied). What it does:
- **Menus (Derma, i.e. Lua UI):** each panel is set to "painted manually" and painted by VRMod
  into its **own render target**: `PushRenderTarget`, `cam.Start2D`,
  `render.OverrideAlphaWriteEnable(true, true)`, `render.Clear(0, 0, 0, 0)`,
  `render.SetWriteDepthToDestAlpha(false)`, `panel:PaintManual()`, everything set back, pop. **No
  `OverrideBlend`** (so partly see-through pixels get alpha × alpha, e.g. 0.5 → 0.25: good enough
  for VR). The render target is then shown with a translucent material, i.e. its alpha is used.
- **HUD:** it does **not** use Garry's Mod's own HUD. It draws its own health / armour / ammo
  ("vitals") in Lua into a render target the same way (troubleshooting idea 6: our own Lua HUD). The engine HUD
  (`render.RenderHUD`) is optional, off by default, and described as "often empty / noisy"
  (matches our test 25).
- So VRMod never gets the engine's HUD or the engine's own VGUI pass to write alpha either; it
  avoids them. Two things it does that we haven't tried: alpha writes forced **without**
  `OverrideBlend`, and painting Lua UI **into our own render target** with `PaintManual`.
- A likely reason (from memory of Source SDK 2013 shader code, **not verified for Garry's Mod**):
  Source's shaders switch alpha writes **off for translucent materials** (they keep them only for
  fully opaque ones, to store depth there). The HUD's and panels' materials are translucent; the
  view model is opaque, which would explain why exactly the 3D part works.

*Idea 2, Lua test patches (step 2a-T32).* A new console command `gmsky_test_alpha2d [name]`
(`gmod/lua/gmsky/overlay2d.lua`), Garry's Mod alone. It draws **known** shapes whose correct
alpha we know in advance, so no black/white reference is needed. Over 3 consecutive frames
(variants), the screen is cleared to transparent black at `PreDrawHUD` (depth-in-alpha off), so
the frame shows only 2D drawing, and captured in `DrawOverlay`:
- variants: `plain` (no override), `write` (VRMod's: `OverrideAlphaWriteEnable` only), `over`
  (test 31's: also `OverrideBlend` with "over" for alpha);
- patches, each 40 × 40 px, white, alpha 255 / 128 / 46 (46 ≈ the health panel's 0.18):
  `surface.DrawRect` and `surface.DrawTexturedRect` and `draw.RoundedBox` in `HUDPaint` (Lua 2D
  through VGUI's surface), `render.DrawQuad` with the colour material (Lua 2D *not* through
  VGUI's surface), and a Derma panel painted by the engine's VGUI pass (Lua UI);
- once, VRMod's way: a second Derma panel painted manually into our own render target
  (`<name>_rt.png`);
- the engine HUD (health corner) is in the same captures.
`tools/alpha2d_report.py` prints, per variant and patch, the measured alpha against the expected
one (and against alpha × alpha, the "no separate alpha blending" result). Logs: `mat_queue_mode`,
frame numbers, which hooks ran.

*Idea 1, read Direct3D 9's settings natively (step 2a-T32, same test).* New
`gmod/module/src/d3d.rs`, `gmsky.D3DState(label)`. **Read only: nothing is hooked or written.**
- Finds Garry's Mod's Direct3D 9 device (the object every draw goes through). First it makes a
  throwaway **null device** (`D3DDEVTYPE_NULLREF`: draws nothing, on a hidden 16 × 16 window,
  released at once; the usual way overlay tools learn this), to learn the addresses of a real
  device's functions. Then it scans `shaderapidx9.dll`'s data for a pointer to an object with
  those functions (memory read with `ReadProcessMemory` on our own process, so a bad pointer can't
  crash; nothing is called on an object whose functions don't match), and confirms it by asking
  the object whether it is a device (`QueryInterface`). Logged once: where, creation flags (a
  "pure" device can't be read; multithreaded or not). Unit test (`cargo test`): the same reads on
  a null device and on a real hidden 16 × 16 device on this PC, which also shows a real device has
  the null device's functions.
- Each call logs the blending settings at that moment: alpha in `COLORWRITEENABLE`,
  `ALPHABLENDENABLE`, `SRCBLEND` / `DESTBLEND`, `SEPARATEALPHABLENDENABLE`, `SRCBLENDALPHA` /
  `DESTBLENDALPHA`, `ALPHATESTENABLE`, and the current render target's format and size.
- Called from the test at: `PreDrawHUD` (after the override), in `HUDPaint` after the first
  patch, at the end of `HUDPaint`, in the Derma panel's paint, in `DrawOverlay`.
- **Only when `mat_queue_mode` is 0.** By default Source draws on a second thread
  (`mat_queue_mode -1`): Lua's hooks then run before the real drawing, so a read would describe a
  different moment (and touch the device from another thread). The test refuses and says so; the
  user sets `mat_queue_mode 0` (the game's own setting) for this test.
- Limit: Source applies settings lazily, at the next draw, so a read shows "what the last draw
  used", close to but not exactly each patch. Exact per-draw settings would need the hook of idea 5.

*What decides idea 5:* if Lua UI and engine 2D show alpha writes **off** in `COLORWRITEENABLE` (or
blending without a separate alpha rule) while they draw, the native fix (force them on below
VGUI) should work, and it would cover Lua UI too, since Lua's `surface` / Derma drawing goes
through the same VGUI surface and the same Direct3D calls (the patches check this). If alpha
writes are already **on** and alpha still comes out 0, the shaders output alpha 0, and idea 5 as
planned would not be enough.

**Steps, each tested on its own:**
- **2a. Experiment, Garry's Mod alone** (like 3a). The module logs which `d3d9.dll` is loaded
  (path, file version) and whether DXVK / other layers are present. `gmsky_test_overlay` (client
  console) draws the view model, hands and HUD into a render target (cleared transparent), for a
  few frames, and saves three pictures to `logs/overlay/` through the module (never into the
  game folder): the plain picture, its alpha channel as grey, and the two-background matte. Logs:
  draw time per frame, render target format. I look at the pictures and the numbers to choose
  the alpha method. Test: hold the crowbar, the physics gun and the tool gun, run the command for
  each; the HUD shows health/ammo.
- **2b. Picture to shared memory.** The module copies the render target to normal memory each
  frame and writes it into three picture slots (SkyCraft's triple buffer; protocol bump; a second
  shared memory block sized for the largest picture, so the 16 MB collision block stays as it is).
  Skyrim publishes its window size; Garry's Mod draws at that size. Harness: the triple buffer
  never hands out a half-written picture. Logs: copy ms, frames written, slot swaps.
  Detailed plan: "Step 2b in detail" below.
- **2c. Skyrim draws it.** Present hook (as SkyCraft), upload, full-screen triangle, premultiplied
  alpha; skipped while a Skyrim menu is open or the puppet is off; Skyrim's first-person arms and
  weapon hidden (as SkyCraft). Logs: frames drawn, picture age in frames (latency), upload ms.
  Test: the crowbar and HUD appear in Skyrim's window, move with the view, disappear in menus.
- **2d. The Garry's Mod window out of the way** (still rendering: Source stops drawing when
  minimized, so it is moved behind Skyrim / off screen instead), and which HUD each game shows
  (Skyrim's compass, health bars and crosshair vs Garry's Mod's), decided with the user.

**Step 2d in detail (decided with the user 2026-10-05, after the blur):**
- **Skyrim's HUD while the overlay is live: hide** the crosshair (the sneak eye still shows while
  sneaking, as SkyCraft), the health / magicka / stamina bars, the compass, the enchantment charge
  meters and the arrow count. Kept: the activation prompt ("E Open"), enemy health bars,
  notifications and subtitles. Everything hidden comes back when the overlay stops.
- **Future (user, 2026-10-05): Garry's Mod's HUD takes over what Skyrim's showed:** stamina and
  magicka bars in HOLOHUD, and quest markers, locations and enemies on HOLOHUD's compass and
  minimap (Skyrim sends them through the link). Until then quest markers are not visible while
  the overlay is live.
- **The Garry's Mod window: moved off-screen** while the overlay is on (just past the right edge
  of all screens; a minimized Source game stops drawing), and put back where it was when the
  overlay switches off. Unknown: whether Source keeps drawing a window that is off-screen; the
  5 s status line (frames presented and published) measures it.
- **2d-1, Skyrim's HUD** (new `skse/src/Hud.cpp`, adapted from SkyCraft's `HideHud` /
  `HideCrosshair`, MIT): on the game thread, every frame while the overlay is live (Skyrim's HUD
  shows the crosshair again whenever it updates it), sets `_visible` (and the crosshair's
  `_alpha`) of the HUD movie's elements; once, logs which element paths exist in this Skyrim
  (paths are guesses for the compass: `CompassShoutMeterHolder`, which also holds the shout meter).
  "Live" is the same test as the arms (2c-2). Logs: hidden / shown changes, paths found.
- **2d-2, the window** (module, `capture.rs` neighbour `window.rs`): when `gmsky_overlay` switches
  on, Garry's Mod's main window (this process's visible top-level window) is moved with
  `SetWindowPos` (no resize, no activation, no Z-order change); its old position is kept and
  restored when the overlay switches off. `gmsky_overlay_hidewindow 0` leaves it alone. Logs: old
  and new position. Typing into Garry's Mod's console still works by switching to its window
  (Alt+Tab): it keeps the focus while off-screen, and its picture is shown in Skyrim's window.

**HUD decision after test 32 (2026-10-05, the user): a downloaded HUD, HOLOHUD 2.** (A "full Lua
UI" of our own was started and cancelled the same day; nothing of it is kept.) The user installed
**HOLOHUD 2** (D/GL4, "Customizable Holographic HUD", Workshop 3459525275), which has a compass and
a radar (later: enemies and quest markers from Skyrim on it). Read from its `.gma` (in the session
scratchpad, never in the repo or the game):
- It draws the HUD in **`HUDPaintBackground` / `HUDPaint`** (only with the camera weapon in
  `DrawOverlay`). Its panels (`vgui.Create`) are only its settings menu. Test 32: alpha forced from
  `PreDrawHUD` is right for drawing in `HUDPaint`, so its HUD should get alpha too.
- It renders its elements into its own render targets in `PreDrawHUD` and lays them on the screen
  as two layers: a translucent background ("frame") and an **additive** glowing composite (text,
  lines, compass, radar; plus blur, scanlines, colour fringes).
- **The open question is the additive layer.** On screen it adds light. Skyrim will draw our picture
  with premultiplied alpha (out = colour + (1 − alpha) × Skyrim), which can show added light
  exactly, **if** the additive layer adds colour and *no* alpha. If forcing alpha writes makes it
  write alpha, Skyrim would be darkened behind the glow (dark halos).
- **How it is measured (test 33):** `gmsky_test_alpha2d` gets a fourth frame, `grey`: like `write`
  but cleared to mid-grey instead of black. In linear light, Garry's Mod's real result over grey is
  known exactly from that frame, and our one-frame picture (`write`: colour + alpha over black)
  predicts it as colour + (1 − alpha) × grey. `tools/alpha2d_report.py` compares the two per pixel
  (error in sRGB levels 0-255; where; how much of it is additive-only, i.e. colour with alpha 0).
  Grey, not white: on white an additive glow would clip at full brightness and hide the answer.
  The colour test patches can be switched off (`gmsky_test_alpha2d <name> nopatches`) so they
  don't cover HUD elements. Two consecutive frames: HOLOHUD's animations (flicker, scanlines) add
  some difference that isn't an alpha error; the report shows where.
- **Colours are sRGB over linear-light blending** (test 32): step 2c must composite in linear light
  (decode, blend, encode).
- **Tests 33-34:** HOLOHUD's own drawing writes alpha 1 over the whole screen (curved "3D visor" and
  flat), forced or not, so its alpha is unusable as it is. Its colours are right, and the alpha
  worked out from a black and a grey frame is right (plates 0.37, glow 0).

**HOLOHUD 2, patched (decided 2026-10-05, the user: "our own personal copy … no need to worry about
updates"; keep the curve if not too hard).**
- **What it draws** (`lua/holohud2/render.lua`): every frame `ComputeHUD` (in `PreDrawHUD`) builds two
  full-screen layers in render targets: `MAT_FRAME`, the see-through background plates (drawn on
  screen with "over" blending in `HUDPaintBackground`), and `MAT_COMPOSITE`, everything else (glow,
  text, lines, compass, radar), drawn **additively** in `HUDPaint`, i.e. it adds light. Then
  `PaintOver` elements drawn directly. With the 3D visor, both layers are drawn on a strip of
  polygons that bends them (`surface.DrawPoly`) instead of one full-screen quad.
- **The patch:** replacement `RenderHUDBackground` / `RenderHUD` that draw the same layers, on the
  same quad or polygons (so **the curve stays**), but with alpha writes set per layer: frame layer
  **on**, with "over" for alpha (alpha = plate alpha); glow layer **off** (adds colour only, which is
  exactly how Skyrim's premultiplied drawing shows added light). Afterwards, alpha writes go back to
  what gmsky wants (`gmsky.alphaForced`, set by our capture code; off otherwise). The branch with
  HOLOHUD's post-processing off (`holohud2_r_pp 0`) calls the original.
- **Where things live:**
  - Our code only, in git: `gmod/holohud2_patch/render_append.lua` (appended to the end of the copy's
    `render.lua`, so it sees that file's local variables; `init.lua` picks up the drawing functions
    after `render.lua` has run). HOLOHUD has no licence file (all rights reserved), so **none of its
    files go into git.**
  - `tools/build_holohud.py`: extracts the user's Workshop `.gma` (read only) into
    `build/holohud2_gmsky/` (never tracked), checks that `render.lua` still has what the patch
    needs, appends the patch, writes `gmsky_patch.txt` (source size/date, patch hash).
  - `tools/install_holohud.ps1`, **run by the user** (it writes into the game folder):
    copies that folder to `garrysmod\addons\holohud2_gmsky\`, replacing only that folder. The
    Workshop HOLOHUD 2 must be **disabled** in Garry's Mod's Addons menu, or both copies load.
- **Tests 36-38: the plates' "glass" was a fake.** HOLOHUD pastes a blurred, fully opaque copy of
  the screen under each plate (`modules/render.lua` `BlurRect`), then black at 94/255 on top. A blur
  of Skyrim's world can't come out of Garry's Mod, so the patch turns it off (`holohud2_r_blur 0`,
  HOLOHUD's own saved setting). Test 38: plates then alpha 94 exactly. Glow: drawn with separate
  alpha blend factors ZERO, ONE (keeps the picture's alpha), after two failed tries with the alpha
  write switch (tests 36, 37).
- **Blur behind the plates, done in Skyrim (planned 2026-10-05, after step 2c works; user asked
  for it):**
  - Garry's Mod sends a **plate mask** next to the picture: HOLOHUD's plate layer alpha (`RT_FRAME`,
    > 0 exactly on plates, test 38), copied by the patch into a one-channel picture, quarter size
    (the blur is soft anyway; ~0.2 MB at 1280×720 instead of 3.7 MB). Goes in the same triple
    buffer as the picture (step 2b), so mask and picture always belong to the same frame.
  - Skyrim, in the Present hook before laying our picture on: copy its finished frame, blur it
    (downsample twice + a few-tap blur in a small pixel shader), then mix: scene = blurred × mask +
    scene × (1 − mask); then our picture on top as planned (premultiplied, linear light).
  - Logs: mask pixels > 0 per frame, blur ms. Test: plates show a blurred Skyrim behind them;
    switching it off (`gmsky_blur 0`, new console variable) shows plain see-through plates.
  - Not from SkyCraft (it has no blur; its overlay only adds an "invert" blend for the crosshair,
    `Overlay.cpp`). Not tested; nothing built.
  - **Revised before building (2026-10-05, after step 2c):** no separate plate mask. The picture's
    own alpha already marks the plates (94 exactly there, 0 where nothing is, tests 38-39), so the
    blur weight is taken from it: w = min(1, a / 0.35) (plates: full blur; anti-aliased edges:
    less). Under opaque things (hands, a = 1) the blur is drawn but fully covered, so it costs
    nothing visible. Side effect: every see-through Garry's Mod background is frosted, including
    the console and menus the user keeps in the overlay. No change to the HOLOHUD patch or the
    block layout.
  - **How, in the existing draw (one pass, no extra blending step):** in Skyrim's `Present` hook,
    before drawing: copy the back buffer (`CopyResource` into a texture of the same format family,
    read through an sRGB view: linear light), shrink it to half and then quarter size (bilinear),
    blur the quarter-size one (separable Gaussian, horizontal then vertical, in 16-bit float
    textures). Then the overlay's pixel shader outputs colour = P + (1 − a)·w·B and alpha
    1 − (1 − a)(1 − w), so the unchanged premultiplied blend gives exactly
    out = P + (1 − a)·(w·B + (1 − w)·S) (P picture, B blurred Skyrim, S Skyrim, all linear).
  - **Switch:** `gmsky_overlay_blur` (Garry's Mod console, default 1) sets a new flag bit
    `GMSKY_OVF_BLUR` in each picture's header (existing `flags` field, layout unchanged).
  - **Logs:** frames blurred, CPU time of the blur passes; the snapshot report checks that pixels
    with alpha 0 and opaque pixels are exactly as before, and that see-through pixels are smoother
    than the unblurred prediction.
- **Test (35):** `gmsky_test_alpha2d hh nopatches` with `holohud2_r_3d 0`, then with the visor on
  (`holohud2_r_3d 4`). The report's grey check must now show small errors (HOLOHUD's own alpha
  matches the alpha the black/grey pair implies) and no "alpha everywhere".

### Step 2b in detail (planned 2026-10-05)

Goal: every Garry's Mod frame, its finished picture (hands, weapon, HUD on a transparent
background, as in tests 30-39) lands in shared memory, where Skyrim (step 2c) can pick up the
newest whole picture at any moment.

**Read in SkyCraft first** (`fabric/.../client/FrameExporter.java`, `skse/src/Link.cpp`
`AcquireOverlayFrame`, `protocol/skycraft_protocol.h` overlay slots): the picture is copied off the
graphics card into one of a few "staging" copies, and only shipped once that copy has finished
(about a frame later), so the game never waits for the graphics card. Then it goes into one of
**three picture slots**: the writer always has a private slot to fill, the reader always has a
whole one to read, and a single number (which slot is "in the middle", plus a "new picture" bit)
is swapped atomically between them. Neither side ever waits for the other.

**What is different for Garry's Mod: where to take the picture.** Two facts decide it:
- The copy needs Garry's Mod's Direct3D 9 **device** (the object every draw goes through). Lua
  doesn't have it. Test 32's search for it failed: it found 10 objects that look like devices (a
  function table in `d3d9.dll`) but compared them with a plain device's functions only. Most
  likely Garry's Mod made a Direct3D 9**Ex** device ("Ex": the Vista-and-later version, which
  Source uses when it can), whose functions differ. **Not verified.**
- By default Source draws on a second thread (`mat_queue_mode -1`): Lua's hooks (`DrawOverlay`)
  run *before* the real drawing happens, on another thread. Copying from a Lua hook would copy the
  wrong frame, from the wrong thread. Only `mat_queue_mode 0` (slower) would make that right.

So the copy happens in a **hook on the device's `Present`** (the call that hands a finished frame
to Windows; SkyCraft hooks the same call on Skyrim's side). It runs on the thread that really
draws, exactly once per finished frame, whatever `mat_queue_mode` is. The hook is installed by
changing one entry of the device's function table (in Garry's Mod's own memory; nothing on disk
changes; this `d3d9.dll` gives every device its own copy of the table, found after test 40). Garry's Mod runs single-player with
`-insecure` (anti-cheat off through the game's own switch), so this touches no anti-cheat
(AGENTS.md rule 5). Our module is never unloaded once loaded (the stub never frees it), so the
hook can't point at freed code.

**Sub-steps, each tested on its own:**
- **2b-1. Find the device, hook `Present`, count only.** The device search also accepts an Ex
  device: it learns an Ex device's functions from a throwaway Ex device (made on a hidden 16 × 16
  window and released at once, like the plain null device), and asks each candidate whether it is a
  device only if its functions match one of the two. A failed search is not repeated for 10 s
  (test 32 repeated it every call: 150 ms stalls), and logs every candidate. The hook replaces
  `Present` (and `PresentEx` on an Ex device) and does nothing but count: frames, which thread
  (compared with Lua's thread), the back buffer's format, size and multisampling (the "back
  buffer" is the picture being presented). Console: `gmsky_overlay_probe` (client). Logged every
  5 s with the client status. Unit test: the same search and hook on a real hidden device on this
  PC, plain and Ex. **Decides:** whether the device is found, whether `Present` runs (Source might
  present through a swap chain object instead), and the back buffer's format (has it an alpha
  channel? multisampled?).
- **After test 40 (device still not found):** the device's function table is *not* inside
  `d3d9.dll`: Windows' `d3d9.dll` gives each device its own copy in ordinary memory (unit test on
  this PC). Tests 32 and 40 only accepted tables inside `d3d9.dll`, so they skipped the device.
  Two routes now, in one build: (a) the direct scan also accepts a table outside the DLL if its
  `QueryInterface` is a real device's; (b) if (a) finds nothing, the Direct3D objects Source keeps
  (textures, surfaces) are asked for the device that made them (`GetDevice`; user's choice).
- **Test 41: 2b-1 works.** Found by route (a). Ex device, pure, multithreaded. `Present` (not
  `PresentEx`) once per frame; on this PC the drawing thread is Lua's thread even with
  `mat_queue_mode -1`. Back buffer `A8R8G8B8`, no multisampling: alpha is there, no resolve needed.
- **2b-2. The picture block and the triple buffer** (no game needed). A second shared memory
  block `Local\GmodSkyrim_overlay_v1` with its own magic and version (the 16 MB link block and
  `GMSKY_VERSION` stay as they are): a control part (SkyCraft's swapped number; frames published;
  Skyrim's window size, written by Skyrim), three slot headers (width, height, pixel format, frame
  number, the precise clock at capture, so Skyrim can log the picture's age), and three pixel slots
  for up to 3840 × 2160 × 4 bytes (about 100 MB of the page file in total; Windows only uses memory
  for the pages written). Each slot also has room for the quarter-size plate mask of the planned
  blur (unused until then, so the blur needs no layout change). C header + Rust copy, layout
  asserted in both. Unit test: a writer thread and a reader thread hammering the triple buffer;
  every picture the reader gets must be whole (all of it from one frame). `tools/overlay_read.py`
  opens the block and saves the newest picture as a PNG with its numbers.
- **2b-2 built (2026-10-05):** as above, with one change from SkyCraft: all three slot numbers live
  in the one 32-bit word (layout in `gmsky_protocol.h`), so a restarted side can't end up sharing a
  slot. Also `fake_overlay` (a fake picture writer, for testing 2c without Garry's Mod).
- **2b-3. Copy every frame.** Lua's live overlay mode (`gmsky_overlay 1`, client console
  variable): every frame, what the tests did for one frame: clear to transparent at the view
  model, depth-in-alpha off, alpha writes forced from `PreDrawHUD` (`gmsky.alphaForced` for the
  HOLOHUD patch), all reset in `PostRender`. The `Present` hook then: copies the back buffer into
  a picture of our own on the graphics card (`StretchRect`, which also resolves multisampling),
  asks the graphics card to tell us when that is done (an "event query"), and in a later frame,
  once it is done, copies it to normal memory (`GetRenderTargetData` into a system-memory surface)
  and from there into the writer's slot, then swaps. Three copies in flight, as SkyCraft. Logs
  every 5 s: frames written, frames skipped (all copies busy), copy ms (graphics card to memory,
  memory to slot), slot swaps. Test: `python tools\overlay_read.py` while Garry's Mod runs gives a
  PNG with the hands and HUD and a frame number that keeps rising.
  Details added before building (2026-10-05):
  - Lua marks each frame it prepared (`gmsky.OverlayPrepared()` in `PostRender`); the hook copies
    only marked frames, so a frame drawn without the preparation (overlay just switched on, a
    loading screen) never reaches Skyrim. If no view model was drawn (no weapon, a camera), the
    clear happens at `PreDrawHUD` instead, so the world is never sent.
  - The hook also replaces the device's `Reset` / `ResetEx` (a resolution change): our pictures
    on the graphics card are released first, as Direct3D 9 requires, and made again at the new
    size on the next frame.
  - New code in `gmod/module/src/capture.rs`; `present.rs` only calls it.
- **2b-4. Skyrim's window size.** The Skyrim plugin opens the picture block and writes its back
  buffer size there each frame (read in its own `Present` path in 2c; for now from the renderer's
  screen size). Garry's Mod logs both sizes and whether they match. Making Garry's Mod draw at that
  size automatically is left to 2d (it means changing Garry's Mod's window size); until then the
  user sets Garry's Mod's resolution to Skyrim's, and 2c scales if they differ.
  Built 2026-10-05 (`skse/src/Overlay.cpp`): the size comes from the swap chain's description
  (what the 2c `Present` hook will see), and Skyrim also logs Garry's Mod's newest picture (frame,
  age) without taking it, to show the two processes share the block before anything is drawn.
  `tools\launch_gmod.ps1 -Width W -Height H` sets Garry's Mod's size.

**Unknowns, measured in 2b-1 / 2b-3:** the back buffer's format (if it is `X8R8G8B8`, there is no
alpha in it and the copy would have to come from Source's own render target instead); whether
`StretchRect` from the back buffer is allowed (it needs the back buffer to be a render target,
which it always is, and the same format); the cost of a 1280 × 720 copy (3.7 MB, expected
~1-3 ms on the render thread).

### Step 2c in detail (planned 2026-10-05)

Goal: Garry's Mod's hands, weapon and HUD appear in Skyrim's window, over Skyrim's world, every
Skyrim frame, with the right transparency.

**Read in SkyCraft first** (`skse/src/Overlay.cpp`, `Game.cpp` `HideFirstPersonMeshes` and its menu
check): it replaces entry 8 (`Present`) of Skyrim's swap chain function table at start-up
(`kDataLoaded`). In the hook, before the real `Present`: take the newest picture, copy it into a
texture on the graphics card (`Map` with `WRITE_DISCARD`), and draw one triangle covering the
screen that samples it, blended "premultiplied" (out = picture + (1 − picture alpha) × Skyrim).
Every graphics setting it changes is saved first and put back after. Not drawn while a Skyrim menu
is open or while loading. Skyrim's own first-person meshes (arms, weapon) are hidden while
Minecraft drives, by marking the meshes "culled" (never the nodes, which the camera and animation
use), and exactly those are shown again afterwards.

**Different from SkyCraft, on purpose:**
- **Linear light.** Garry's Mod blends in linear light and stores sRGB-encoded colours (test 32), so
  a stored colour is sRGB(linear colour × alpha). Mixing it with Skyrim's sRGB picture directly (as
  SkyCraft does) makes see-through parts too dark or too bright. Instead: our texture is created as
  an sRGB format (the graphics card decodes when sampling: linear, premultiplied), and Skyrim's back
  buffer is drawn into through an sRGB "view" (the card decodes Skyrim's pixel, blends in linear
  light, encodes again). The shader only copies. If the sRGB view of the back buffer can't be made
  (it depends on the back buffer's format and how its swap chain presents), it falls back to
  SkyCraft's way and logs "approximate blending".
- **Stale pictures are not drawn:** if the newest picture is more than 0.5 s old (Garry's Mod
  stopped, paused, or `gmsky_overlay 0`), nothing is drawn, so a frozen HUD never hangs over Skyrim.
- **Drawn only while the puppet drives the player** (`Puppet::IsActive`), no Skyrim menu is open
  (`Input::SkyrimMenuOpen`, the same check input uses).

**Sub-steps, each tested on its own:**
- **2c-1. Present hook, take, upload, draw.** New `skse/src/Draw.cpp`; `Overlay.cpp` gains the
  reader side (attach as reader, take, slot pixels). Logs once a second: presents, pictures taken,
  frames drawn, frames not drawn by reason (menu, puppet off, no picture yet, stale), upload ms
  (average, max), picture age when drawn (ms), back buffer format, linear or approximate blending,
  which thread presents (against the game thread). Test: the crowbar and HUD appear in Skyrim's
  window, move with the view, disappear in menus; numbers as above. Skyrim's own arms still show
  (2c-2).
- **2c-2. Skyrim's first-person arms hidden** while the overlay is drawn (SkyCraft's
  `HideFirstPersonMeshes`, on the game thread in the per-frame driver, not in the `Present` hook).
  Logs the number of meshes hidden / shown again. Refined before building: "while the overlay is
  live" = the puppet drives the player and Garry's Mod's newest picture is under 0.5 s old (not
  "drawn this frame", which would show Skyrim's arms during every Skyrim menu). The model is walked
  every frame while hidden, because Skyrim rebuilds it (weapon drawn / changed) and new meshes
  must be hidden too (SkyCraft does the same). New file `skse/src/Arms.cpp`.

- **2c-1 done (tests 44, 45):** linear blending confirmed by snapshot (≤ 1 level). Garry's Mod's
  halo library (physgun grab outline) made the frame opaque: while the overlay is on, `halo.Render`
  is wrapped so the frame's alpha is copied before and written back after (`liveoverlay.lua`).
  **Garry's Mod's menus and console are part of the overlay and show in Skyrim: kept (user's
  decision, 2026-10-05).**

**Not in 2c:** Skyrim's own HUD (compass, crosshair, health) and the Garry's Mod window: 2d.

**Phase 2 closed (2026-10-05, after test 50).** Its "done when" (hands, weapon and HUD drawn
inside Skyrim; Garry's Mod window hidden) is met. Left open on purpose, not blocking: Garry's Mod's
window size isn't matched to Skyrim's automatically (`tools\launch_gmod.ps1 -Width W -Height H`
does it by hand), and quest markers / stamina / magicka in HOLOHUD (user's "future" list, 2d).
One gap found while closing it belongs to Phase 3: Skyrim sends the mouse wheel but Garry's Mod
ignores it, and keys 1-6 aren't sent at all, so weapons can't be switched from Skyrim's window
(step 3a).

### Phase 3 steps: combat (planned 2026-10-05)

Goal (§12 table): fight a bandit camp with a crowbar. Garry's Mod's weapons hit Skyrim's NPCs
through Skyrim's own hit system (they bleed, stagger, get angry, count as crimes), Skyrim's NPCs
hurt the Garry's Mod player, and **Garry's Mod's health decides death** (§8).

**Read in SkyCraft first** (`skse/src/Combat.cpp`, protocol `ActorRecord` / `McEvent` /
`kInHurt`, its DESIGN §8): Skyrim writes a table of nearby actors every frame (`WriteActorTable`:
formId, position, yaw, height, width, health fraction, level, hostile / dead / essential / in
combat flags, name). Minecraft spawns an invisible stand-in for each. A Minecraft hit on a
stand-in becomes an event that Skyrim feeds into its **own melee hit function** (ID 38586, only
after checking that the engine's melee code at ID 38627 + 0x4A8 really calls it; otherwise plain
`DoDamage`), with a vanilla weapon standing in for blood and sounds, skill experience, and
`StartCombat`. Hits on the Skyrim player are **refunded** in Skyrim each frame (health topped up)
and sent to Minecraft with the attacker and kind (from `TESHitEvent` and a `HandleHealthDamage`
hook); damage over time is batched every 0.5 s. While Minecraft drives, the Skyrim player is
**essential** (can't die in Skyrim); Minecraft's death kills the Skyrim player (`KillImpl`), and
Skyrim's own death / reload flow runs. Damage scaling: Minecraft → Skyrim × (5 + 0.25 × level),
Skyrim → Minecraft ÷ 5.

**Different for Garry's Mod:** Garry's Mod has 100 health like Skyrim's player, and its weapons do
tens of damage per hit (crowbar about 10, the .357 and crossbow far more; from memory, **not
checked**: 3d logs the real numbers), so the scaling is probably closer to 1:1 than SkyCraft's
(Minecraft swords do ~7). Defaults chosen in 3d / 3e,
both Garry's Mod console variables. Stand-ins must **not** be physics objects (tests 12-17:
creating physics objects near the moving player crashed `vphysics.dll`); a bounding box answered to
traces is enough for bullets and the crowbar, the same way the ground is answered (step A).

**Steps, each tested on its own:**
- **3a. Weapon choice from Skyrim's window** (input bridge, no new protocol fields).
  - *First build (2026-10-05), rejected by the user:* the server picked the next weapon itself
    (`cmd:SelectWeapon`), a "quick switch" with no weapon menu, and Q was "last weapon". The user:
    the weapon menu must show as in Garry's Mod, and **Q is Sandbox's spawn menu** (`+menu`).
  - *Rebuilt:* Skyrim's keys are turned into Garry's Mod's own **binds on its client side**, the
    way a real key press is: first the `PlayerBindPress` hook (where HOLOHUD 2's weapon menu
    listens: `invnext` / `invprev` / `slotN` open and move it, `+attack` picks the weapon and is
    swallowed, read in its `elements/weaponselection.lua`), then, if no hook took it, the bind is
    run as a console command (`ply:ConCommand`). Mouse wheel up / down = `invprev` / `invnext`,
    1-6 = `slot1`-`slot6` (bits 16-19 of `GmskyInput.buttons`, the slot key held), left / right
    button = `+attack` / `+attack2`, Q = `+menu` while held (`GMSKY_IN_MENU`, 0x800).
  - **Attack now comes from Garry's Mod's own command:** the server takes `IN_ATTACK` /
    `IN_ATTACK2` from the player's command (filled by `+attack` on the client) instead of from
    Skyrim's buttons, so a click that a menu swallowed never fires the weapon. If the client side
    stops reporting (0.5 s), the server falls back to Skyrim's buttons as before.
  - **Cursor mode** (spawn menu, any Garry's Mod menu with a mouse cursor, `vgui.CursorVisible()`):
    Skyrim's mouse moves Garry's Mod's menu cursor (`gui.InternalCursorMoved`, the way VRMod drives
    menus), its buttons and wheel click and scroll (`gui.InternalMousePressed` / `Released` /
    `MouseWheeled`), and the view doesn't turn. The real Windows cursor isn't in the overlay
    picture, so a cursor arrow is drawn in `DrawOverlay`. `gmsky_cursor_speed` (default 1) =
    pixels per Skyrim mouse count. Typing into the spawn menu's search box: not covered.
  - The physics gun holding something (attack held): the wheel isn't turned into binds; the server
    passes it in the command (`cmd:SetMouseWheel`) and the gun pushes / pulls the prop.
  - Module: the input reader keeps a separate baseline per side (server and client both read the
    same input); `gmsky.SetClientInput(cursor)` / `gmsky.ClientInput()` tell the server that the
    client is handling binds and whether a cursor is up.
  - Logs: every bind (taken by a hook or run), cursor on / off with position, per-second counts.
    Harness: binds for wheel / slot / attack / Q, a hook taking `+attack`, cursor mode, physgun
    wheel, server attack from the command, fallback.
- **3b. Nearby NPCs, Skyrim → Garry's Mod (log only).** New latest-value slot `actors` (protocol
  v10): the nearest 64 actors within 6,000 Skyrim units (~3,200 Garry's Mod units, beyond a bow
  shot; SkyCraft's `WriteActorTable`), position in Garry's Mod units relative to the anchor (like
  the ground grids; the module adds the anchor and refuses a table from another origin), yaw,
  height, radius, health fraction, level, flags, name; written every Skyrim frame as SkyCraft does
  (the stand-ins in 3c move every tick). New `skse/src/Actors.cpp`, `gmod/lua/gmsky/actors.lua`. Garry's Mod logs the count and the
  nearest few each second. Test: walk up to Whiterun guards; the log names them with distances
  that match Skyrim's.
- **3c. Stand-ins in Garry's Mod, with Skyrim's own hitboxes.** Planned in detail below
  ("Hitboxes: how bullets hit Skyrim's NPCs").
- **3d. Garry's Mod hits Skyrim's NPCs.** A one-way event queue Garry's Mod → Skyrim (protocol bump).
  `EntityTakeDamage` on a stand-in: the damage is cancelled there and sent as a hit (formId,
  damage, damage type, hit position, direction, force). Skyrim applies it like SkyCraft's
  `ApplyHit` (own hit function, checked first; stand-in Skyrim weapon by damage type for blood and
  sound; skill experience; `StartCombat`). Damage scaling: console variable, default decided with
  the user (§14 question 5). Test: crowbar a bandit until they die; Skyrim's log shows each hit.
  **3d in detail (planned 2026-10-05):**
  - *Found in 3c-3 (test 59):* Garry's Mod's copy agrees with Skyrim's parts on 99.3-100% of rays;
    where the parts are in Skyrim's physics world (corpses, some living NPCs) Skyrim's own
    projectile ray hits exactly them; for the other living NPCs it hits their movement capsule
    (CHARCONTROLLER) instead, so Skyrim's arrows are coarser there than our copy. We keep the
    parts (head shots need them).
  - *Decided (the user):* Skyrim damage = Garry's Mod damage × **3** (`gmsky_damage_scale`, flat
    for now, tuned later); **head × 2**, everything else × 1 (`gmsky_headshot_scale`); explosions
    have no body part (their damage is placed at the blast).
  - **Event queue Garry's Mod → Skyrim** (protocol v13): a ring of 64 hit events in the slots
    area. Garry's Mod writes an event, then advances `head`; Skyrim keeps its own read position
    (starts at `head` when it attaches, so old hits never land; if Garry's Mod restarted, `head`
    goes back and Skyrim follows). Lost events (more than 64 between two Skyrim frames) are counted.
    Event: formId, Skyrim damage (already scaled), hit position and direction (Garry's Mod units
    relative to the anchor), damage force, Garry's Mod damage type, body group, bone index, kind
    (bullet / melee / explosion / other).
  - **Garry's Mod:** `gmsky_actor:OnTakeDamage` → scale → event (`gmsky.SendHit`); logged.
  - **Skyrim (new `Combat.cpp`, adapted from SkyCraft's `Combat.cpp` `ApplyHit`, MIT):** every frame
    while the puppet drives, drain the queue; for a living NPC feed the hit into Skyrim's own melee
    hit function (ID 38586, used only after checking that the engine's melee code at ID 38627 +
    0x4A8 really calls it; otherwise plain `DoDamage`) with a vanilla stand-in weapon chosen by
    damage type (bullets: Hunting Bow, for arrow-like blood and sound; clubs: Iron Mace; slashes:
    Iron Sword; explosions: no weapon, `DoDamage` and Skyrim's explosion knock-back, as SkyCraft's
    `ApplyExplosion`), at the bone that was hit (blood and impact sound there), then `StartCombat`
    if not fighting yet. Skyrim's hit function does the rest: hit reactions, aggro, **crime**
    (hitting a citizen is assault, as in Skyrim), kill credit. Armour is not applied (SkyCraft's
    choice: the damage is set directly). Skill experience: not in 3d.
  - **3d-2, corpses:** a hit on a dead NPC pushes its ragdoll: an impulse along the shot on the
    body part hit (bullets, melee) or on all its bodies away from the blast (explosions), as
    SkyCraft's `FlingRagdoll`. Strength from Garry's Mod's damage force; tuned in game.
    **In detail (planned 2026-10-05, after test 62):** Garry's Mod already sends hits on corpses
    (test 60: 5 counted, not applied); only Skyrim changes (`Combat.cpp`, SkyCraft's
    `FlingRagdoll` / `UpdateFlings`, MIT). Bullet / melee: the body part hit (same index as the
    hitbox copy; corpses' parts are dynamic bodies in Skyrim's physics world, test 59) gets a
    push along the shot of 0.25 m/s per Garry's Mod damage, 1.5-15 m/s (pistol 5 → 1.5, .357 40
    → 10, crowbar 10 → 2.5), plus blood and the impact sound as on the living. Explosion: every
    body of the ragdoll is thrown out and up from the blast at 0.1 m/s per Garry's Mod damage,
    3-15 m/s. A living NPC caught by an explosion is remembered for 2 s: if it dies in that time
    (Skyrim turns it into a ragdoll a moment later), its body is thrown the same way. Damage
    isn't applied to corpses. Numbers are a first guess, tuned in game. Logs: `COMBAT corpse
    <name>: <kind> on <part> -> <speed> m/s on <n> bodies` and the flings of the newly dead.
    **Test 63 (the user):** shots too tame (logs: 1.5 m/s, the minimum, on every pistol / SMG
    hit); an RPG moves the whole body without changing its pose until it hits something; the
    user prefers Garry's Mod's ragdolls and asked whether Skyrim's could be matched to them.
    *Answer:* Garry's Mod's own ragdoll can't be used: it would be a Garry's Mod physics object
    on Skyrim's ground, which Garry's Mod's physics can't see yet (DESIGN "Found in test 60",
    tests 12-17), and it would need a Garry's Mod model with Skyrim's skeleton. Instead Skyrim's
    ragdoll is **pushed the way Garry's Mod pushes its own**:
    - The push is Garry's Mod's real damage force (its bullets: Source's `BULLET_IMPULSE` per
      ammo type, x3.5; melee and blasts: Source's damage-force formulas), sent with the hit
      (already in the event) times `gmsky_ragdoll_force` (default 1), converted from kg·inch/s
      to Havok's kg·m/s (x 0.0254).
    - Bullet / melee: applied **at the point hit** on that body part (Garry's Mod's
      `ApplyForceOffset`), so the part spins as well as moves; at most 30 m/s on that part.
    - Blast: shared over the ragdoll's bodies by mass, each body pushed **out from the blast
      from its own position** (plus a little up), so the limbs fly apart instead of the body
      moving in one piece; at most 30 m/s.
    - A shot that kills: its push is kept for up to 1 s and applied when Skyrim has made the
      ragdoll (as the blast already was).
    - No force from Garry's Mod (0): the old damage-based speeds.
  - Logs: Garry's Mod: every hit sent (NPC, part, Garry's Mod damage, Skyrim damage); Skyrim: every
    hit applied (path used, health before → after, killed or not), events lost.
  - Test (3d-1): shoot a hostile (bandit, wolf): it reacts, bleeds, fights back, dies; head shots
    take fewer bullets. Shooting a townsperson is a crime in Skyrim (a bounty); test on hostiles.
  - **Found in test 60 (hits work; for later, after Phase 3's core):** Garry's Mod's *world* objects
    and effects aren't in Skyrim, because the overlay only carries the hands, weapon and HUD:
    grenades, rockets, AR2 balls, explosions, tracers and the physgun beam are invisible. And
    Garry's Mod's *physics* objects don't meet Skyrim's world or NPCs (grenades and AR2 balls fall
    through; the ground and stand-ins answer traces only). Two later steps, planned when we get
    there: (a) **draw Garry's Mod's world objects and effects into the overlay**, hidden behind
    Skyrim's walls (needs both games' depth; the hard part); (b) **Skyrim's world and NPCs for
    Garry's Mod's physics** (physics ground away from the player, where tests 12-17's crash
    doesn't apply, or moving those objects by traces), then AR2 balls and grabbing corpses with
    the physgun (moving Skyrim's ragdoll).
- **3e. Skyrim's NPCs hurt the Garry's Mod player.** SkyCraft's refund + attacker / kind
  detection; the player essential in Skyrim while the puppet drives; an event queue Skyrim →
  Garry's Mod; Garry's Mod applies it as damage from the attacker's stand-in (so its damage
  direction indicator points at them), slash / bullet / burn by kind. Test: let a bandit hit you;
  HOLOHUD's health drops, Skyrim's stays full.
  **3e in detail (planned 2026-10-05, from SkyCraft's `Combat.cpp` `HitSink` /
  `HandleHealthDamageHook` / `BridgePlayerDamage` / `SetEssential` and `SkyCombat.hurtPlayer`, MIT):**
  - *Skyrim (Combat.cpp):* while the puppet drives, the Skyrim player is **essential** (Skyrim
    can't kill them; a huge hit knocks them down for a moment at most) and their health is **topped
    back to full every frame**; the amount topped up is the hit. Who and how come from two
    listeners: Skyrim's hit event (`TESHitEvent`: attacker, weapon or spell, projectile or not,
    power attack, blocked) and a hook on the player's `HandleHealthDamage` (the attacker of any
    health damage, for spells and poison). A health drop within 0.35 s of a hit event is that hit
    (melee / projectile, sent at once); anything else (spells, poison, falling, unknown) is added
    up and sent every 0.5 s. When the puppet stops: not essential any more, nothing refunded.
    Health the save already lacked when the puppet starts is topped up once and not sent.
  - *Event queue Skyrim -> Garry's Mod* (protocol v14): a ring of 32 hurt events, written like the
    hit ring but the other way. Event: attacker formId (0 = none), Skyrim damage, weapon / spell
    formId, kind (melee / projectile / magic / other), flags (power attack, blocked in Skyrim).
    The module keeps the server's read position; `gmsky.ReadHurts()` returns the new ones.
  - *Garry's Mod (new `gmod/lua/gmsky/hurt.lua`, server):* each event becomes damage on the
    player: Garry's Mod damage = Skyrim damage x `gmsky_hurt_scale` (**default 1**: both players
    have 100 health, so Skyrim's fights stay as deadly as in Skyrim; Skyrim's armour already
    reduced it), attacker and inflictor = the attacker's stand-in (else the world), damage
    position = the attacker (from the actors table), type by kind: melee `DMG_SLASH`, projectile
    `DMG_BULLET`, magic `DMG_BURN`, other `DMG_GENERIC`. So Garry's Mod's own armour (suit
    battery), god mode, pain sounds and HOLOHUD's damage indicator all work as for a Garry's Mod
    hit. A power attack also shoves the player away from the attacker (300 units/s).
  - Not covered: Skyrim potions and healing don't heal Garry's Mod's player; Garry's Mod's death
    (3f). Falling: Garry's Mod has its own fall damage; if Skyrim also hurts the puppet when it
    lands, it shows as "other" damage in both logs (then it gets filtered).
  - Logs: Skyrim `COMBAT player hurt by <name> (<id>): <damage> <kind>...` per hit and a
    per-second line (hurts sent, health topped up, essential on / off); Garry's Mod `HURT <name>:
    <skyrim> Skyrim -> <gmod> Garry's Mod damage, health a -> b` per event.
- **3f. Death.** *First plan (replaced 2026-10-05):* Garry's Mod's death kills the Skyrim player
  (SkyCraft's `KillPlayer`) and Skyrim reloads a save. **The user after test 61:** dying should
  work like Garry's Mod: you respawn, no save reload; and control must stay with Garry's Mod.
  **3f in detail (planned 2026-10-05):**
  - *Found in test 61:* on death Skyrim switched the puppet off ("Garry's Mod player is dead"),
    which also handed the keyboard and mouse back to Skyrim, so the click to respawn never reached
    Garry's Mod. After respawning (from Garry's Mod's own window), Garry's Mod put the player at
    the map's spawn point, 16,000 units away; that re-centred the anchor and Skyrim made a new
    origin elsewhere.
  - *Decided (the user):* you come back at the **last calm spot**: where you last stood while no
    one was fighting you.
  - **Skyrim (`Puppet.cpp`):** a dead Garry's Mod player no longer switches the puppet off: Skyrim's
    player stays where Garry's Mod's body is (still essential, still topped up), input keeps going
    to Garry's Mod (so a click respawns), the camera follows Garry's Mod's dead view (eye low on
    the ground). Logs the death and the respawn.
  - **Skyrim -> Garry's Mod "in a fight" flag:** `GmskySkyrimState.flags` bit `GMSKY_SF_IN_COMBAT`
    (no layout change, the field was unused): some living actor within 6,000 units is in combat
    with the player as its target (SkyCraft's `AnyoneFighting`), checked 4 times a second.
    Module: `gmsky.SkyrimFlags()`.
  - **Garry's Mod (new `gmod/lua/gmsky/respawn.lua`, server):** twice a second, while alive, on the
    ground, not in noclip, not held by the terrain hold, Skyrim connected and anchored, the flag
    off and no hurt from Skyrim in the last 3 s: the position (relative to the anchor, with the
    origin epoch) and view yaw are the calm spot. On death: logged (attacker, where, the calm
    spot's age and distance). On the respawn after a death (`PlayerSpawn`): the player is put at
    the calm spot (Garry's Mod's own respawn does the rest: full health, Sandbox's weapons);
    without a calm spot for the current origin (e.g. died right after a load door), where they
    died. The terrain hold catches a spot whose ground hasn't arrived. Skyrim's puppet moves its
    player there like any other step.
  - Not covered: Skyrim NPCs stay hostile and may come after you; nothing is lost on death.
  - Test: let a bandit kill you; Garry's Mod's death screen shows in Skyrim's window; click: you're
    back at full health where you stood before the fight, no save reload, no new origin in the logs.
    **Works (test 62).**
- **F1: Garry's Mod's console** (the user asked after test 62; key chosen by us: Skyrim doesn't use
  F1). Typing from Skyrim's window into Garry's Mod isn't bridged, so the console opens in Garry's
  Mod's **own window**: F1 sets `GMSKY_IN_CONSOLE` (0x1000) in the input buttons (no layout change)
  and Skyrim allows another process to take the front (`AllowSetForegroundWindow`); Garry's Mod's
  client moves its window from off-screen to the middle of Skyrim's window, brings it to the front
  and runs `showconsole`. When the console closes (~ or Esc in Garry's Mod) or the window loses the
  front (click back into Skyrim), the console is hidden, the window goes back off-screen and
  Skyrim's window is brought to the front. In single-player the game pauses while it is open.
  *Test 63:* `showconsole` from Lua is refused by Garry's Mod (the user opened it with ~); the
  window part worked. *Fix:* the module posts Garry's Mod's own console key (~, `toggleconsole` by
  default) to its window (a Windows key message to that one window), once, after it is in front.

### Phase 4 steps: the full world (planned 2026-10-05, after Phase 3)

Goal (§12 table): real Havok shapes, interiors, load doors, floating origin across the whole map.
**Already built and tested in Phase 1:** Skyrim's exact collision shapes streamed to Garry's Mod
(test 23: buildings, interiors, stairs, doors refreshed every second), a new origin on every
location change or when Skyrim moves the player itself (load doors in test 23), the floating
origin (re-centres, test 22: 109,000 Skyrim units in one direction). **Never tested:** fast
travel, Whiterun / Solitude / Markarth / Riften / Windhelm (their own worldspaces), a dungeon with
many load doors, combat and respawn inside and right after doors, sleeping / waiting, and **water**:
Garry's Mod knows nothing of Skyrim's water, so the player probably walks on the bottom of rivers
and lakes while Skyrim's player is put there too (not checked).

**Steps:**
- **4a. World audit (one guided play run, logs only).** New Skyrim log line once a second:
  `WORLD interior/exterior '<cell>' in '<worldspace>' | water surface z / none, feet above / below
  it | swimming yes/no | puppet on/off`. The route: walk into Whiterun (city worldspace), into a
  house and out, fast travel by the map to Riverwood, wait an hour (T), walk into the river until
  under water, into Bleak Falls Barrow (dungeon, several load doors) and fight in it, die once in
  it. From the logs: every origin change and how long the puppet waited, holds, jumps, re-centres,
  the collision stream after each door, water depths. Each problem found becomes its own step.
- **4a done (test 66, the user: "It was just fine"):** Whiterun (own worldspace), Warmaiden's,
  fast travel to Riverwood, the Sleeping Giant Inn (slept in a bed), Bleak Falls Temple: 10
  location changes, the puppet back 16-25 ms after each; ground holds released in 2-5 ticks; the
  fast travel's loading screen stopped Skyrim for ~1 s, Garry's Mod's hold let go ("Skyrim not
  connected") and the new origin put the player right. **Found:** T (wait) didn't reach Skyrim:
  only Esc, ~, J, M, F9 did. *Fixed:* T, Tab, I, P and F5 reach Skyrim too (Garry's Mod has no use
  for them in single-player). **Water:** "can't swim, Garry's Mod treats it as air"; Skyrim itself
  switched its player to swimming when the feet were 61-191 units under the surface. The logged
  surface was the cell's water level (-250 everywhere in Riverwood), not necessarily the river's.
- **4b. Water and swimming (planned 2026-10-05).**
  - *Skyrim:* every frame, the water surface where the player is: Skyrim's own "relevant water
    height" for the player (rivers, lakes, pools; it is what Skyrim swims by), else the cell's water
    level (CommonLib `TESObjectREFR::GetWaterHeight`). Sent in `GmskySkyrimState` (protocol v15:
    `reserved0` becomes `water_z`, Garry's Mod units relative to the anchor; flag
    `GMSKY_SF_WATER` 0x2 when there is water). The `WORLD` line logs it next to the character
    controller's own water height and the cell's, to compare.
  - *Garry's Mod (new `gmod/lua/gmsky/water.lua`, both sides for prediction; module
    `gmsky.WaterZ()`):* how deep the player is, as Source counts it (1 = feet, 2 = waist, 3 =
    eyes under). From waist deep, a `Move` hook replaces Garry's Mod's movement with Source's own
    water movement (`CGameMovement::WaterMove`, re-written in Lua): no gravity; you swim where you
    look; Jump swims up; with no keys you sink slowly (60 units/s); speed 0.8 x walking speed;
    water friction (`sv_friction`) and acceleration (`sv_accelerate`); the move slides along
    Skyrim's collision with hull traces. Waist out of the water: Garry's Mod's own movement again
    (gravity), so you bob at the surface. Jump at the surface facing a bank climbs out (Source's
    water jump). Falling into deep water: no fall damage (the landing never happens).
  - Skyrim: its player is where Garry's Mod's is, so Skyrim swims, draws the underwater view and
    counts breath itself; drowning damage reaches Garry's Mod through step 3e (as "other").
  - Logs: Garry's Mod: entering / leaving the water (level, surface, depth), per second time
    swimming and the deepest; Skyrim: the `WORLD` line.
  - Test: walk into Riverwood's river: you sink slowly, swim where you look, Space swims up, you
    climb out on the bank; Skyrim shows you underwater; staying under long enough hurts you.
- **4b done (tests 67-68):** swimming, bobbing, climbing out and drowning work; W + Space swims
  forward and up together after the fix in test 68 (keys scaled like Source's `CheckParameters`).
  The menu keys T, Tab, I, P, F5 work (test 68).
- **Phase 4 is done (2026-10-05).** Its §12 list (real shapes, interiors, load doors, floating
  origin) was met in Phase 1; 4a found nothing else to fix beyond the keys and water.

### Garry's Mod's world objects and effects in Skyrim (planned 2026-10-05, after Phase 4)

**The problem (found in test 60).** The overlay picture only carries Garry's Mod's hands, weapon
and HUD: `liveoverlay.lua` wipes the frame to transparent where the view model starts, which also
wipes everything else Garry's Mod drew in the world. So grenades, rockets, AR2 balls,
explosions, tracers, the physgun beam, props and Garry's Mod's own NPCs are invisible in Skyrim.

**What is needed.** (1) Garry's Mod keeps those things in the picture and drops only its own map
(gm_flatmap's floor and sky). (2) They must be hidden where a Skyrim wall, hill or person is in
front of them. That needs to know, for every pixel, which game's thing is nearer: "depth" (how
far each pixel is from the camera). Both games have it, but in different places.

**How (2) is done: SkyCraft's way (the user's decision after test 70).** SkyCraft draws its
"world layer" depth-tested against **Skyrim's own depth buffer** (`WorldRender.cpp`, MIT): Skyrim
walls hide Minecraft blocks exactly, leaves and all. We do the same with Garry's Mod's picture:
- Garry's Mod sends **two pictures** instead of one: the **world layer** (everything Source drew
  after the map: grenades, rockets, explosions, tracers, beams, props, NPCs) and the **hands / HUD
  layer** (view model, hands, HUD, menus; always on top), plus a **depth picture** for the world
  layer: for every pixel, how far the nearest Garry's Mod object is from the camera.
- Skyrim, while drawing the pictures at the end of its frame, reads **its own depth buffer**
  (the main one, `RENDER_TARGETS_DEPTHSTENCIL::kMAIN`'s shader view, as SkyCraft), turns both into
  distances along the view, and drops every world-layer pixel that has Skyrim geometry in front
  of it. Then the hands / HUD layer goes on top, as today.
- *(Rejected first plan, for the record: Garry's Mod hiding its objects behind Skyrim's
  collision triangles. Collision isn't the visible shape (leaves, grass, clutter), so it would
  look wrong. The user: "Do it the way skycraft does".)*

**Getting Garry's Mod's depth.** Direct3D 9 can't read Source's own depth buffer. Source has a
shader made to write depth as a colour: `DepthWrite` with `$color_depth 1` (Source SDK 2013
`depthwrite_ps2x.fxc`: red = the pixel's projected depth / 4000, green = blue = 0, alpha = 1).
After the frame, Lua draws the same view a second time (`render.RenderView` with the main view's
position, angles, FOV, near and far, no view model, no HUD) into a half-size floating-point
picture (`RGBA16F`), with every object drawn through that material
(`render.MaterialOverride`), after wiping the map the same way. "No depth" (nothing drawn there)
stays 0. A pixel whose green / blue isn't 0 or alpha isn't 1 was drawn by something that ignored
the override (a particle or sprite, maybe): counted, and treated as "no depth" (always shown).
Half size: 1/4 of the copying; Skyrim takes the nearest of the 4 depth pixels around each
picture pixel, so edges aren't cut.

**Steps:**
- **W1. World objects kept in the picture** (built; test 70 showed it never ran: Garry's Mod
  reports "drawing the sky" on every main view of a map with a 2D sky (the wiki says so), and
  W1 skipped those. *Fix:* only the 3D sky and depth passes are skipped.)
- **W2. Two layers + Garry's Mod's depth (Garry's Mod side, module).**
  - Lua: once the world layer is drawn (where the view model, hands or HUD start),
    `gmsky.OverlayGrabWorld()`: the module copies the current picture on the graphics card
    (`StretchRect`, as the capture already does at Present) into the world layer of this frame's
    copy set; Lua then wipes the colour to transparent and Source draws the hands and HUD on it.
    In `PostRender`, the depth view (above) into a half-size `RGBA16F` target, then
    `gmsky.OverlayGrabDepth(far)` copies it the same way. New settings:
    `gmsky_overlay_depth 1/0`, `gmsky_overlay_depth_far 4000` (the shader's constant; checked
    in game, see logs).
  - Module (`capture.rs`): each copy set gets a world layer and a depth picture next to the
    HUD picture; all three come to normal memory together. The depth is turned into distances
    (Garry's Mod units, `red x far`; 0 = none) as 32-bit floats.
  - Overlay block (protocol): no new block; the slot holds the HUD picture, then the world layer
    (same size), then the depth (`depth_width` x `depth_height` floats). Flags
    `GMSKY_OVF_WORLD` 0x8, `GMSKY_OVF_DEPTH` 0x10; `reserved0` becomes `depth_width` /
    `depth_height` (16 bits each). Pictures too big for the slot (above about 2560x1440) are
    sent as one layer, as today (counted).
  - Logs (5 s): world layers and depth pictures sent; depth pixels used / "foreign" (drawn
    without the override); **agreement**: of the world layer's solid pixels, how many have a
    depth (should be near 100%; low means the second view doesn't line up with the first, e.g.
    the FOV); **calibration**: the depth at the screen centre against Garry's Mod's own trace to
    the entity under the crosshair (tells whether 4000 is right).
- **W3. Skyrim's depth test (Skyrim side, `Draw.cpp`, adapted from SkyCraft (MIT)).**
  - Skyrim's depth: the main depth buffer's shader view; probed every 5 s (0.5 s until known)
    with 16 samples read back without waiting, as SkyCraft's `RunProbe` / `ReadProbe`: flat
    (cleared, e.g. a loading screen: no test), standard (1 = far) or reversed (0 = far).
    Distance from depth with the camera's near / far (`viewFrustum`), SkyCraft's
    `SceneViewDepth`.
  - One drawing pass: world layer, minus pixels where Skyrim is nearer than Garry's Mod by more
    than 1% + 6 Skyrim units (SkyCraft: 0.2% + 1.5; ours is wider because Garry's Mod's picture
    is one frame older), then the HUD layer over it, then onto Skyrim's frame as today (blur
    unchanged). No depth (old Garry's Mod picture, unknown Skyrim depth): the world layer is drawn
    untested.
  - Logs (1 s): depth probe result (once and on change), world layers drawn with / without the
    test.
  - Test: throw a grenade behind a house corner, fire the RPG past a tree, spawn a prop and
    walk behind a wall: hidden exactly where Skyrim's geometry is in front, visible elsewhere;
    hands and HUD never hidden.
**Test 71 (the user: black boxes around see-through effects, objects "extremely jittery" when
moving around them, depth "mostly shows on top") and what the logs said:**
- *On top:* 0% of the depth picture had a depth. GMod's own `DepthWrite` is fine (its compiled
  `depthwrite_ps30.vcs`, disassembled: red = distance / far plane, green = blue = 0, alpha = 1), but
  the objects were drawn with their **normal** materials: the engine ignores
  `render.MaterialOverride` for its own drawing of entities in `render.RenderView`. And the probe of
  Skyrim's depth called it "flat" whenever all 16 samples were sky (1.0), turning the test off for 5 s.
- *Jitter:* copying three pictures cost 4.7 ms a frame (was 1.2) and the second view 2 ms, so Garry's
  Mod ran at ~80 frames/s instead of ~180 and its pictures reached Skyrim 20-27 ms old. Skyrim draws
  its world with its camera of *now*, the objects come from Garry's Mod's camera of 20 ms ago: moving
  sideways, they slide back and forth. (SkyCraft never had this: it draws its blocks inside Skyrim
  with Skyrim's own camera.)
- *Black boxes:* see-through effects (sprites, tracers, glows) also write their texture's alpha, so
  their black corners became opaque black over Skyrim.

**W4. The fixes:**
- *Depth from the main view itself:* no second view. Right after the main view's see-through pass
  (`PostDrawTranslucentRenderables`, the same camera matrices), Lua pushes the half-size RGBA16F
  target and draws each visible Garry's Mod entity with a model itself (`ent:DrawModel()`) under the
  DepthWrite override (the documented way the override works), then grabs it. Exact alignment, no FOV
  question, ~no extra cost. Effects without a model (sprites, particles, tracers) have no depth:
  they are re-projected as if far away and never hidden (known limit, logged).
- *Re-projection (Skyrim):* each picture carries the camera Garry's Mod drew it with (eye relative to
  the anchor + origin epoch, angles, horizontal FOV, aspect; the slot header grows to 128 bytes,
  overlay block version 2). For every screen pixel, Skyrim finds where that point was in Garry's Mod's
  picture (3 steps: take the depth there, place the point in the world from Garry's Mod's camera,
  project it with Skyrim's own camera matrix, correct), takes the colour from there, and does the
  depth test with that point's distance in **Skyrim's current** camera. The hands / HUD are not
  re-projected (they move with the view). Logged: the camera difference (units, degrees) per second.
- *Cost:* the world layer and depth are only copied while there is something to show (a drawable
  Garry's Mod entity nearby, or a shot / effect in the last second); per-stage copy times logged.
- *Black boxes:* alpha writes are switched off for the main view's see-through pass: effects add
  light (alpha 0), solid objects keep alpha 1.
- *Probe:* all-sky samples (all 1.0) keep the known convention instead of switching the test off;
  probed every second.

**After tests 72-74: depth from Garry's Mod's own depth pass (W5, planned 2026-10-05, not built).**
Three home-made ways to draw depth failed (DepthWrite through `MaterialOverride` drew nothing,
tests 72-73; fog depth turned pure white in game, test 74; details in MODLOG and STATUS). The
references the user pointed to (meetric1's GMod shader guide, the Valve wiki "Depth buffer" page,
Evgeny Akabenko's GShader Library, the Garry's Mod wiki) show the way every working GMod depth
addon does it, and it is the engine's own:
- Return `true` from the **`NeedsDepthPass`** hook. Garry's Mod then runs Source's
  `SSAO_DepthPass` every frame: the scene drawn a second time by the engine itself with DepthWrite
  (`$color_depth`) into **`_rt_ResolvedFullFrameDepth`** (`render.GetResolvedFullFrameDepth()`).
  The engine sets the shader up properly, which is what our Lua override never managed.
- Make that target **32-bit float** first (Garry's Mod wiki example: `GetRenderTargetEx(
  "_rt_resolvedfullframedepth", 1, 1, RT_SIZE_FULL_FRAME_BUFFER, MATERIAL_RT_DEPTH_SHARED,
  bit.bor(4, 8, 256, 512), 0, IMAGE_FORMAT_R32F)` with `IMAGE_FORMAT_R32F = 27`), so distance is not
  cut to 8 bits. GShader Library does the same ("increasing the bit depth").
- Value: red = distance along the view / a constant (my disassembly of `depthwrite_ps30.vcs`:
  divided by the far plane `g_vNearFarPlanes.y`; the Valve wiki says the TF2 branch divides by 4000).
  Not sure which; the module logs raw values at the centre with `OverlayDepthCentre` and the
  Garry's Mod crosshair trace distance, so the constant is read from the numbers, not guessed.
- The module grabs that texture's surface (format D3DFMT_R32F = 114 instead of 113) and copies it
  at the Present like today; Skyrim's side (re-projection, depth test) stays as it is.
- It holds only opaque things (no effects), like today's plan. It also holds gm_flatmap's ground;
  that does not matter, because depth is only used where the world layer has a Garry's Mod pixel.
- The fog path, its calibration squares and the Lua-drawn depth target are removed.

**RPG black patch (test 74):** the stamp found no see-through sub-material on the RPG (0 left out),
and the black patch pulses: it is the RPG's laser glow **sprite**, drawn by the weapon's own code
whenever its model is drawn, so the stamp's second `DrawModel` draws it too, as solid. Plan: stamp
the view model through a stand-in `ClientsideModel` of the same model, bone-merged to it
(`EF_BONEMERGE`), whose drawing runs no weapon code. Not built.

Not in these steps: Garry's Mod's physics against Skyrim's world (props, grenades and AR2 balls
falling through), which is a separate plan.

**Hitboxes: how bullets hit Skyrim's NPCs (planned 2026-10-05, the user asked for this plan before 3c).**

*What counts as "the right hitbox".* Skyrim's own projectiles (arrows, bolts, spells) hit an NPC's
**per-bone body shapes**: capsules and small hulls fixed to the skeleton (head, spine, arms, legs;
for creatures their own set), the same bodies that become the ragdoll when it dies. They follow
the NPC's animation every frame. (From memory of how Skyrim works, **not verified**: an arrow
sticking in the bone it hit suggests it. Step 3c-1 measures it.) Skyrim's melee doesn't use
hitboxes at all (a reach and angle check), so for Garry's Mod's bullets, crowbar and explosions
the per-bone shapes are the only "real" target. SkyCraft uses one box per NPC; Minecraft's own
hitboxes are boxes, so that was exact for it, but it is too coarse for guns (shots over a
shoulder or between the legs would hit).

*Options considered:*
1. One box or upright capsule per NPC from its bound size (SkyCraft's way, what 3b already
   sends). Simple, but misses the real shape: arms out, crouching, lying, creatures.
2. **Skyrim's per-bone shapes, copied to Garry's Mod every frame (recommended).** Skyrim reads
   each NPC's body shapes (the decoder exists: `Collision.cpp` already turns Havok capsules,
   boxes and hulls into positions for the ground), converts them like everything else, and sends
   them; Garry's Mod's stand-in answers every trace against those shapes (`TestCollision`, the
   same way the ground works). Every Garry's Mod weapon that traces (bullets, the crowbar's hull
   trace, crossbow bolts, explosion line-of-sight) then hits exactly what a Skyrim arrow would.
3. Let Skyrim decide every bullet: Garry's Mod sends each fired bullet's ray, Skyrim casts it
   against its own physics. Exact by definition, but only for `FireBullets` (the crowbar,
   projectiles and explosions don't use it), and a hit would land a frame later than Garry's Mod
   thinks. Kept as a *checking tool* (below), not the main path.

*How option 2 works:*
- **Skyrim (new `Hitboxes.cpp`, game thread):** for each actor in the table within a shorter
  range (bullets beyond ~3,000 Skyrim units are rare; tunable), walk its 3D for collision bodies
  on Skyrim's body layers (`kBiped`, `kBipedNoCC`; `kDeadBip` once dead), decode each shape with
  the existing decoder, and send one record per part: a capsule (two end points + radius) or a
  small hull, in Garry's Mod units relative to the anchor, with the bone's name (so "head" is
  known). Expected size: ~15-25 parts per person, a few KB a frame for a dozen NPCs. The slots
  area (64 KB) can't hold it next to the rest for 64 NPCs, so it goes in its own block or a bigger
  slots area (protocol bump; decided when building).
- **Fallback if the bodies aren't readable or don't follow the animation while alive** (3c-1
  tells): capsules between the skeleton's bone nodes (which certainly animate: head, neck,
  spine, upper / lower arms and legs), sized from a small table per body part and the NPC's
  scale. Close, not exact, and logged as such.
- **Garry's Mod (`gmsky_actor` entity per NPC):** no physics object (tests 12-17); a box around
  all its parts as the entity's bounds (so the engine only asks it about traces that come near);
  `TestCollision` checks the ray (or hull) against each part and returns the nearest hit, and
  remembers which part it was. Moved every tick from the newest record, blended like the player
  (Skyrim's frames vs Garry's Mod's ticks).
- **Body parts:** Garry's Mod normally gets a "hit group" (head, chest, arm...) from its own
  models; custom collision can't return one, so the stand-in records the part it reported and
  step 3d sends that bone with the hit (Skyrim plays blood there; whether a head shot does more
  damage is for the user to decide in 3d).
- **Walking into NPCs:** traces with a size (the player's movement box) are answered with one
  upright capsule per NPC, not the limbs, so you bump into people smoothly instead of snagging on
  arms. Bullets (size zero) use the parts.

*Delay.* Garry's Mod sees each NPC's shapes about one Skyrim frame plus up to one tick late
(~15-30 ms). A walking NPC (~100-200 Skyrim units/s) moves 2-6 units in that time; a running one
(~400) up to ~12. Measured in 3c (below); if it matters, Garry's Mod moves the shapes ahead by the
NPC's speed (it knows it from consecutive frames), as Skyrim already does for the ground.

*How we prove the hitboxes match, by numbers (no looking):*
1. **Shapes, 3c-1 (Skyrim only):** log, per NPC kind (person, wolf, mudcrab, ...) alive and dead,
   how many body parts were found, their physics layer, shape kind, bone, size and whether they
   are in Skyrim's physics world. Every frame for the nearest three living NPCs and the nearest
   corpse: each part's position **in its bone's own frame**; if the parts follow the animation that
   stays fixed while the bones move (drift ≈ 0, bones moving), if they don't the drift grows with
   the bones' movement (fallback).
2. **Conversion check (automatic, both sides):** Skyrim also computes, for the parts it just
   sent, a few test rays (through the head, the chest, past the shoulder) with its own maths;
   the harness and an in-game command check that Garry's Mod's `TestCollision` gives the same hit
   points (within 0.1 unit) for the same records.
3. **Against Skyrim's real physics (the decisive test):** console command `gmsky_hitbox_test`:
   with an NPC under the crosshair, Garry's Mod sends a fan of rays (e.g. 21 × 21 over the NPC);
   Skyrim casts every ray against its own physics (body layers only) **and** answers which part
   Garry's Mod's copy says it hits, in the same Skyrim frame (Skyrim checks Garry's Mod's copy
   with the same maths, so both are judged at one moment and the delay doesn't blur the
   result). Report: rays hit in both / only Skyrim / only Garry's Mod, the same body part or not,
   and the distance between hit points. Target: ≥ 98% agreement on a standing NPC; the
   disagreements should sit on part edges. Repeated on a walking NPC to measure the delay's
   effect with the normal (delayed) copy.
4. **While playing:** once a second, Garry's Mod's trace along the aim names the NPC and body
   part under the crosshair, and Skyrim's own crosshair pick names its target: agreement count in
   the logs.

*Decided (the user, 2026-10-05):* option 2 with the bone fallback, target as above. **Corpses
must stay hittable**, exactly as in Skyrim: shooting or blowing up a body works (the stand-in stays
after death, with the ragdoll's shapes, `kDeadBip`; 3d pushes the ragdoll, SkyCraft's
`FlingRagdoll`). **Head shots do extra damage** (amount decided in 3d).

*Found in 3c-1 (test 56):* people have 19 capsules (COM, three spine, neck, head, arms, hands,
legs, feet), dogs / wolves / rabbits 22, a chicken 7; all capsules; they follow the animation
(in their bone's frame they move ≤ 0.02 units a frame while the bones move up to 6). A carried
bow has its own box on the body layer (gear, not body). Most living NPCs' parts are **not** in
Skyrim's physics world (the corpse's are): how Skyrim's arrows hit the living is open, and 3c-3's
check against "Skyrim's real physics" has to use what Skyrim's projectiles really hit, not a plain
ray cast against the world.

**3c-2 in detail (planned 2026-10-05):**
- **Protocol v11:** new slot `hitboxes` (in the 64 KB slots area, ~16.8 KB): up to 24 actors
  (nearest first, within 3,000 Skyrim units, living or dead) and 512 parts (~26 people). Part:
  kind (capsule: two end points + radius, exact; box: world box, marked approximate), body group,
  flags (approximate, gear), in Garry's Mod units relative to the anchor. Actor: formId, first
  part, part count, flags (dead, parts in Skyrim's physics world).
- **Body groups** are Garry's Mod's hit groups, chosen in Skyrim from the bone's name: head;
  chest (upper spine, neck); stomach (lower spine, COM, pelvis); left / right arm (upper arm,
  forearm, hand, clavicle; a creature's front legs); left / right leg (thigh, calf, foot, a
  creature's back legs); generic (tails and anything else); gear (bones that aren't body: bow,
  weapon, shield, quiver). Logged per NPC kind with the bone names, so a wrong guess shows.
- **Garry's Mod module** (new `hitboxes.rs`): keeps the newest table; does the maths in Rust
  (unit-tested): ray against capsule / box with the hit point, normal, part and group;
  `gmsky.HitboxActors(last)` (per actor: formId, flags, box around its parts),
  `gmsky.HitTrace(id, start, delta, extents)`, `gmsky.HitPartAt(id, point)` (nearest part, for
  where damage landed).
- **`gmsky_actor` entity** (new `gmod/lua/gmsky/standins.lua`, both sides like `gmsky_field`):
  one per actor in the table, removed when it leaves; no physics object; custom collisions;
  bounds = the parts' box plus a margin (the client's copy of the entity lags behind the server's
  by Garry's Mod's interpolation; the shapes themselves come from the module on both sides, so
  only the bounds need slack). `TestCollision`: a line (bullets) is tested against every body
  part (not gear); a box (player movement, the crowbar's fallback trace) against one upright
  capsule per actor around its parts, widened by the box's size, so you bump into people without
  snagging on arms. A trace that starts inside the shape doesn't hit it (nobody gets stuck when
  an NPC walks into them). `gmsky_actor_solid 0` lets the player walk through NPCs.
  **Changed after test 57 (the user: bumping "felt really weird"):** a fixed, flat-sided,
  world-aligned box per living NPC instead of the capsule ("hull", like Garry's Mod's own NPCs),
  at the NPC's feet from the actors table (not from the animated parts, which made the capsule
  wobble), half width min(Skyrim's bound radius, 0.4 × height) clamped to 6-24, as tall as the
  NPC; corpses have none (you walk over bodies, as over Garry's Mod's ragdolls).
- **Logs:** stand-ins made / removed, parts per actor, traces asked / hit per second; every
  damage a stand-in takes (from, weapon, damage, damage type, body group, distance from the
  nearest part) — not passed to Skyrim yet (3d); once a second the NPC and body group under
  Garry's Mod's crosshair (Skyrim's own crosshair target next to it moves to 3c-3, with the check
  against Skyrim's real physics).

**NPCs walking into the player (after test 58, the user's choice: "NPC shoves me"):** while the
player's box overlaps a living NPC's hull, `SetupMove` gives the player a speed out of it along the
shorter way (depth / 0.08 s, at most 300 units/s); Garry's Mod's movement applies it (never into
a wall), friction stops it once clear. Logged: shoved ticks and deepest overlap per second.

**3c-3 in detail (planned 2026-10-05, after test 56 showed most living NPCs' parts are not in
Skyrim's physics world):** `gmsky_hitbox_test` (server console command; works from the console in
single-player):
- Garry's Mod takes the NPC under the crosshair, lays a 21 × 21 grid of rays (441) over its parts'
  box, square to the view, from in front of it through it, and answers each ray from its copy
  (hit or not, body group, distance). It sends the rays to Skyrim (new slot `hit_test_request`,
  protocol v12; rays relative to the anchor).
- Skyrim, on the next frame, answers each ray twice (slot `hit_test_result`):
  1. **"Skyrim now":** the same ray maths against the parts it reads *that frame* (no conversion,
     no delay). Garry's Mod vs "Skyrim now" = conversion error + the copy's delay. Standing
     target: should agree on ≥ 98% of rays (misses at part edges only).
  2. **Skyrim's own physics** (Havok ray cast in the NPC's cell, as a projectile's collision
     layer): what it hits first (layer, and whether it belongs to this NPC, and how far). For
     corpses and NPCs whose parts are in the physics world this checks our shape reading against
     Skyrim's own physics; for the other living ones it shows what a projectile-like ray hits
     instead (the character controller capsule? nothing?) — the open question from test 56.
- Report (Garry's Mod log, one block per test): rays hit in both / only Garry's Mod / only Skyrim
  now; same body group; distance difference (avg, max); the delay (table age); the Havok answers
  counted by what they hit and, where they hit this NPC's parts, their distance against "Skyrim
  now".
- Cost: once per command, not per frame.

*Sub-steps:* **3c-1** read and log the body shapes (Skyrim only; decides option 2 or the
fallback). **3c-2** send them (protocol bump) and build the stand-ins in Garry's Mod, with the
harness check (2). **3c-3** `gmsky_hitbox_test` (3) and the crosshair agreement (4) in game.
Hits doing anything is 3d.

### Garry's Mod's things drawn inside Skyrim (geometry route, planned 2026-10-05, after test 74)

**Decision (user, 2026-10-05):** render Garry's Mod's world objects inside Skyrim, the way
SkyCraft does, because it removes the jitter. This replaces the picture world layer (W1-W5). W5
(the engine's depth pass) is **not built**.

**Why it removes the jitter.** Today Skyrim pastes a flat picture that Garry's Mod drew 10-20 ms
earlier with its own camera; when the two cameras differ, the objects slide. In the geometry
route, Skyrim draws the object's triangles at their place in the world, with **Skyrim's camera of
this frame**. A prop standing still is then glued to the world like any Skyrim rock. Depth also
comes for free: the triangles are drawn into Skyrim's own depth buffer, so walls hide them and
they hide what's behind them.

**What SkyCraft does (read 2026-10-05, MIT):**
- Minecraft side (`AvatarExporter.java`): every Minecraft frame, it runs Minecraft's own entity
  and particle renderers, but into a "collector" of its own instead of the graphics card. Out come
  the posed triangles (position, texture coordinate, colour) grouped by texture. It sends them
  through a byte ring in shared memory (`kRenScene`, relative to an origin near the camera).
  Textures go once (`kRenTexture`, `kRenAtlas`). Positions are taken at Minecraft's render time
  (interpolated between physics ticks).
- Skyrim side (`WorldRender.cpp`): a hook right after `Main::RenderWorld`
  (`REL::ID(36559) + 0x85E`, the call to `REL::ID(107142)`, checked before patching) draws the
  triangles into **Skyrim's HDR scene target** (`kMAIN`) with **Skyrim's main depth buffer** bound,
  so they are depth-tested and write depth. It also writes motion vectors and normals so Skyrim's
  anti-aliasing (TAA) and reflections treat them like Skyrim's own objects. The shader lights them
  with Skyrim's sun, ambient, shadows and fog. Camera: Skyrim's own world -> clip matrix, re-based
  on the camera position. The hands and HUD stay a picture (`Overlay.cpp`).

**Differs from SkyCraft (on purpose, said up front):**
1. *Transport.* SkyCraft sends fully posed triangles every frame. We send each **model once** (its
   triangles in rest pose, with which bones move each corner) and then, every frame, only **where
   each object is and how its bones are posed** (a few matrices). Skyrim's graphics card bends the
   model into the pose ("GPU skinning", what Source itself does). Same picture as SkyCraft's
   result, far less data: a Source ragdoll is 5,000-10,000 triangles.
2. *Where the triangles come from.* Minecraft's renderer can be pointed at a collector; Source's
   can't from Lua. We read the model files through Garry's Mod's own `util.GetModelMeshes`
   (triangles, texture coordinates, normals, bone weights, rest pose) and the textures through
   `file.Read(<texture>.vtf, "GAME")` (Garry's Mod's file system, so Workshop and mounted content
   work too). Read at run time from the game; nothing is written into the repo (AGENTS.md rule 1).
3. *Effects.* SkyCraft's collector also catches particles. Source's tracers, explosions, AR2 balls
   and the physgun beam are drawn by the engine in C++, which Lua can't capture this way. They are a
   separate, later step (G7) with its own options. **Not solved by this plan.**

**Steps (one at a time, each with an in-game test):**

- **G1. Skyrim draws a test object in its own frame.** Skyrim only. Port SkyCraft's in-frame pass:
  the hook after `Main::RenderWorld`, its checks of the targets (HDR scene, depth size), its saving
  and restoring of Skyrim's graphics state, its camera re-basing. Draw one plain cube (1 m, white,
  lit by a fixed light), placed once 300 units in front of the player on the first frame after a
  save loads, then fixed in the world (a constant in the plugin, on for G1, off afterwards; the plugin has no
  settings file). Logs: hook installed (bytes checked),
  targets and formats, frames drawn, ms per frame, the cube's screen position when the player
  looks at it. **Test:** the user walks around the cube and behind a wall: it stays put, no
  jitter, the wall hides it. This proves the hook, the depth and the camera on our setup before
  any Garry's Mod data.
- **G2. The render block.** New shared memory `Local\GmodSkyrim_render_v1` (the main block's
  layout doesn't change): a byte ring Garry's Mod -> Skyrim like SkyCraft's render ring, with
  messages `Texture` (once per texture), `Model` (once per model + skin + body groups: vertices with
  position, normal, texture coordinate, up to 3 bones and weights; triangles; which texture per
  part; material flags), `Frame` (every Garry's Mod frame: anchor epoch, time stamp, a list of
  objects: model id, colour, and each bone's matrix relative to the anchor), `Forget` (a model
  Skyrim may free). Rust writer in the module, C++ reader in the plugin, unit tests (round trip,
  ring wrap-around, a reader joining late), a fake writer tool. No game needed.

  *G2 in detail (2026-10-05).* Block `Local\GmodSkyrim_render_v1`, 64 MB of ring (SkyCraft's size):
  a control header (magic, version, size, `writer_session`, `reader_epoch`, counters) at 0, the
  ring counters at 0x1000 / 0x1040 (head = bytes written by Garry's Mod, tail = bytes read by
  Skyrim), data from 0x1080. Messages as the collision ring: 8-byte header (type, bytes), 8-byte
  aligned, `PAD` = "skip to the start"; a message may use at most half the ring.
  - `RESET` {session}: Skyrim drops every model and texture it holds. Garry's Mod sends it first in
    every stream, and again whenever `reader_epoch` changes (a Skyrim that started after it). A
    Skyrim that attaches skips everything unread (tail = head) and adds 1 to `reader_epoch`, so
    Garry's Mod resends everything.
  - `TEXTURE` {id, width, height, format, bytes} + pixels: format BC1 / BC2 / BC3 (Direct3D 11 reads
    Source's DXT1 / 3 / 5 directly) or RGBA8; top row first; one mip level (Skyrim makes the rest).
  - `MODEL` {id, vertex count, index count, part count, bone count} + parts {first index, index
    count, texture id, material flags, alpha reference} + vertices {position, normal (model space,
    Garry's Mod units), u, v, 4 bone numbers, 4 weights (0-255, summing to 255)} + u32 indices.
  - `FORGET` {kind, id}: a model or texture Skyrim may free.
  - `FRAME` {frame number, time (QueryPerformanceCounter), origin epoch, object count, matrix count}
    + objects {model id, first matrix, matrix count, colour RGBA8, flags} + matrices (3 x 4 floats,
    rows of [rotation | translation]): a model-space point -> Garry's Mod world units relative to
    the anchor of that origin epoch. Bone b's matrix is `ent:GetBoneMatrix(b)` x the model's
    inverse bind pose (`util.GetModelMeshes`' second result, "poseToBone"), so a vertex is
    `sum(weight x matrix[bone] x position)`. Skyrim converts with `GmodToSkyrim` (§5, no axis swap).
    Skyrim keeps only the newest frame it read; frames naming a model it doesn't hold are counted
    and skipped.
  - Skyrim (`skse/src/Render.cpp`) drains the ring each frame in the in-frame hook (as SkyCraft's
    `DrainMessages`), checks every message's sizes, keeps models / textures on the CPU, and logs
    once a second: messages by type, bytes, resets, bad messages, models / textures held, the newest
    frame's objects and age. Uploading to the graphics card is G3.
  - Tests without a game: Rust unit tests (each message round trip, wrap-around, full ring refuses,
    late reader + reset), C++ `protocol_test` (layout numbers equal Rust's; the C++ reader reads a
    ring written by a C++ copy of the writer), `fake_render` (writes a test model, texture and
    moving frames into the real block, for G3's first test without Garry's Mod).
- **G3. Props.** Garry's Mod (client Lua): every frame, the objects with a model within range
  (not the local player, its view model, our hitbox stand-ins, `gmsky_field`, brush models
  `*N`, `GetNoDraw()` objects). New models are read once with `util.GetModelMeshes` and their
  textures with `file.Read`; the module decodes the VTF header and sends the biggest picture
  (DXT1 / DXT5 straight as BC1 / BC3, which Direct3D 11 reads natively; other formats turned into
  RGBA8). Skyrim draws them with their textures, Skyrim's camera, depth, scaled by the §5
  conversion. The world layer picture is switched off by default (`gmsky_overlay_world 0`).
  Logs: models / textures sent (bytes, formats, any it couldn't read and why), objects per frame,
  Lua ms, Skyrim draw ms. **Test:** spawn props; they stand in Skyrim's world, hidden by walls.

  *G3 in two halves (2026-10-05).*
  - **G3a, Skyrim draws what the block holds** (`World.cpp`, tested with `fake_render`, no Garry's
    Mod): models into vertex / index buffers, textures into textures (RGBA8 as sRGB with Skyrim
    making the smaller mip levels; BC1-3 as sRGB, one level), uploaded when a model or texture
    arrives or changes, freed on RESET / FORGET. Every Skyrim frame, the newest Garry's Mod frame,
    if its origin epoch is the one Skyrim published (`origin` slot): per object, its matrices into
    a buffer the vertex shader reads, and its triangles drawn per part. The vertex shader bends the
    model (up to 4 bones per corner, the weights summing to 1) and places it at `origin - camera +
    position / scale` (§5; the camera subtraction in double precision on the CPU). Same pass,
    targets, depth and motion vectors as G1; fixed light (Skyrim's light is G5); alpha-tested parts
    drop pixels under `alpha_ref`; no face culling yet. The test cube is switched off.
    `fake_render` follows Skyrim's current origin epoch, so its bar stands 150 Garry's Mod units
    (282 Skyrim units) east of the origin and 40 up. Logs: GPU models / textures held, uploads and
    their ms, objects drawn / skipped and why (epoch, unknown model, bone count), triangles, ms.
  - **G3b, Garry's Mod sends its props** (Lua + module): the rest of G3 above. In detail:
    - *Which objects* (client Lua, new `gmod/lua/gmsky/render.lua`, `gmsky_render 1`): every entity
      with a model within 8,000 units, except the local player and what it holds or wears (its
      weapons, view model, hands), our own `gmsky_*` entities (hitbox stand-ins, the ground), brush
      models (`*N`), `GetNoDraw()` and fully transparent ones. This includes Garry's Mod NPCs,
      ragdolls, rockets and grenades: they all go through the same bones path (G4 is then only
      checking that they look right).
    - *A new model* (cached per model + skin; at most one new model per Garry's Mod frame, as it
      costs a few ms): `util.GetModelMeshes(model, 0, 0, skin)`; each mesh becomes a part. Its
      `triangles` list is sent as is (3 corners per triangle, no sharing; simple, at most ~3x the
      vertices). Per corner: position, normal, u, v, up to 3 bones + weights (`weights`). The bind
      poses' matrices (`poseToBone`) go along. Body groups: the model's defaults for now (mask 0).
    - *Its materials*: `Material(mesh.material)`; flags from `$flags` (translucent 0x200000,
      additive 0x80, alpha test 0x100, self-illum 0x40, no-cull 0x2000) and
      `$alphatestreference`; the texture `$basetexture`, read once per texture with
      `file.Read("materials/<name>.vtf", "GAME")` (all mounted content). The module decodes the VTF
      (`gmod/module/src/vtf.rs`): header 7.0-7.5, the biggest mip level no larger than 1024, formats
      DXT1 / DXT3 / DXT5 / BGRA8888 / BGR888 / RGBA8888 / RGB888 / ABGR8888 / BGRX8888 /
      DXT1_ONEBITALPHA / I8 / IA88 decoded to RGBA8 (so Skyrim can make the mip levels); others are
      logged and drawn white.
    - *Every Garry's Mod frame* (`PostDrawOpaqueRenderables`, once per frame, not for the 3D sky or
      depth passes): for each object, `SetupBones()` then each bone's `GetBoneMatrix(b)` (the
      entity's own matrix if a bone has none), 12 numbers each, in one flat table to
      `gmsky.RenderFrame`. The module multiplies each by the model's `poseToBone`, subtracts the
      anchor, and writes the FRAME with the origin epoch it holds.
    - *The module* (`gmod/module/src/render_out.rs`, client side): owns the `Writer`; keeps every
      model and texture message it sent, and after a RESET (a Skyrim that started later) sends them
      all again before frames. New Lua functions: `RenderTexture(id, vtf bytes)`, `RenderModel(id,
      bones, vertices, indices, parts, bind poses)` (flat number tables), `RenderFrame(flat)`,
      `RenderStatus()`. Reading flat Lua tables needs `lua_rawgeti` and `lua_objlen` (added to
      `lua.rs`).
    - The world layer picture goes off by default (`gmsky_overlay_world 0`; `1` brings it back), so
      objects aren't drawn twice.
    - Logs (Garry's Mod, every 5 s): objects sent, models / textures known and failed (with why),
      Lua ms for new models and per frame; the module's line: messages, bytes, refused, resets.
    - Tests: VTF decoding unit tests (hand-made files: DXT1 / DXT5 blocks with known colours, BGR888,
      a 7.3 file with a resource table, mip choice); harness check: `render.lua` with fake
      entities and a fake `util.GetModelMeshes` sends the right model / frame numbers.
- **G4. Moving and bending things.** Bone matrices every frame (`ent:SetupBones()`,
  `GetBoneMatrix`), skinning in Skyrim's vertex shader: ragdolls, Garry's Mod NPCs, rockets,
  grenades. **Test:** ragdolls flop, rockets fly, all in Skyrim's world.
- **G5. Light like Skyrim's, and see-through materials.** SkyCraft's lighting (sun, ambient,
  shadows, fog, its exposure curve) with the models' normals; motion vectors and normals targets
  (so TAA doesn't smear them); `$translucent` / `$additive` / `$alphatest` / `$selfillum` drawn in
  a second pass, as SkyCraft's blended pass. **Test:** props look lit like the rocks next to them,
  day and night.
- **G6. Smoothness check.** Logs per Skyrim frame: the age of Garry's Mod's newest frame and how far
  each moving object jumped. A still prop can't jitter (it's in the world). A **moving** object
  updates at Garry's Mod's frame rate (85-180/s in test 74); if the logs show uneven steps, Skyrim
  interpolates between Garry's Mod's last two frames on its own clock (what SkyCraft does for the
  player, `McState` tick stamps). Known risk, said now: **a prop held by the physgun** follows
  Garry's Mod's view, which reaches Skyrim 10-20 ms after Skyrim's mouse look, so it may wobble
  against the screen while turning. Measured here; fixed only if the numbers show it.
- **G5a + G6 bundled (user's choice, 2026-10-05, after tests 79-80: "models are dark, just a bit,
  and there is some slight desync when dragging them around with the physgun").**
  - *G5a, Skyrim's light* (SkyCraft's `GatherLighting` and its shader's `Lighting`, MIT): every
    frame Skyrim's directional light (sun / moon outside, the cell's light inside: direction, colour
    x fade), its directional ambient (the weather's six colours outside, the cell's or its lighting
    template's inside), the 8 point lights nearest to reaching the camera (torches, fires, spells:
    position, radius, colour x dimmer x fade) and fog (near, far, power, clamp, near / far colours).
    Lit = ambient by the normal + sun x max(0, n . sun) + each point light x (1 - (d / radius)^2) x
    max(0, n . l); fog mixed in by distance as Skyrim's shaders do. Drawn into the HDR scene, so no
    exposure curve and no grading (SkyCraft's HDR path: Skyrim's tone mapping does that). Skyrim's
    normals target is written too (SkyCraft's `EncodeSkyrimNormal`), so its ambient occlusion sees
    our surfaces. Parts with `$translucent` / `$additive` go into a second pass after the solid ones
    (blended, depth test without depth write), as SkyCraft's blended pass. Logged every 30 s and on
    a cell change: sun direction / colour, ambient up / down, point lights used, fog.
    **Differs from SkyCraft:** no Minecraft sky / block light in the vertices (Source models have
    none); shadows are not in G5a.
  - *G5b, shadows (next step, not in this bundle)*: Skyrim's sun shadows on our objects and our
    objects' shadows on Skyrim (SkyCraft's `CaptureSunShadows` / `SkyrimSunShadow` / its shadow map
    and multiply pass).
  - *G6, held objects* (the risk named in G6 above). Why they lag: Skyrim's camera is Garry's Mod's
    newest view plus the mouse movement Garry's Mod hasn't applied yet (`Puppet::ViewNow`), but a
    held prop's place comes from Garry's Mod's frame, drawn with Garry's Mod's older view and 12-26
    ms old on arrival (tests 76-77). So while you turn, the prop trails. The fix is SkyCraft's rule
    for the player's own body (`RenAvatar` is sent relative to the feet Skyrim's camera follows, so
    it "can't drift from the camera"): a held object is placed relative to the camera, not the world.
    - Garry's Mod (server, `render.lua`): physgun, gravity gun and +use pickups / drops set
      `ent:SetNW2Bool("gmsky_held", ...)` (`OnPhysgunPickup` / `PhysgunDrop`,
      `GravGunOnPickedUp` / `GravGunOnDropped`, `OnPlayerPhysicsPickup` / `OnPlayerPhysicsDrop`).
    - Each FRAME carries the view Garry's Mod drew it with (`render.GetViewSetup()`: eye relative to
      the anchor, pitch / yaw / roll); objects carry their entity number and a HELD flag.
      Render block **version 2** (frame header 32 -> 56 bytes; `Object.reserved` becomes `entity`).
    - Skyrim: for a held object, `p_now = R_skyrim_now x R_gmod_then^T x (p - eye_then)`, i.e. the
      same place in front of Skyrim's camera of this frame as it had in front of Garry's Mod's camera.
      Other objects stay world-fixed.
    - Motion vectors for moving objects: Skyrim keeps each entity's matrices from the frame before,
      so its TAA sees the object's own motion (not only the camera's), which otherwise smears moving
      objects. Held objects: no screen motion.
    - `gmsky_render_held 0` (Garry's Mod client) sends no HELD flag, to compare. Logs once a second
      (Skyrim): held objects, how far Garry's Mod's view of their frame was from Skyrim's camera
      (degrees, units) and the frame's age.
    - **Differs from SkyCraft:** SkyCraft does this for the player's body only; we do it for any
      object the player holds.
    - *Result (test 81):* built and active, but Garry's Mod's view and Skyrim's camera differed by
      only 0.01 degrees on average, so it changes little; the visible drag lag is probably Garry's
      Mod's own physgun physics. **Parked as later polish (user, 2026-10-05).**
- **G5b + surface maps bundled (user's choice, 2026-10-05, after test 81: "a bit flat").**
  - *Skyrim's sun shadows on our objects* (SkyCraft's `CaptureSunShadows` / `SetSkyrimShadows` /
    `SkyrimSunShadow`, MIT): a hook on `BSShadowDirectionalLight`'s vtable entry 0x0A (its Render)
    copies the sun's first two shadow cascades the moment Skyrim has drawn them (the shared shadow
    map array is reused for other lights later in the frame), with each cascade's light transform
    and split as they were then. Our pixel shader looks each surface point up in them (cascade by
    view depth, 3x3 samples) and takes the sun's share away where Skyrim's trees, buildings or
    terrain block it. Conventions (row / column matrix, depth direction) decided from the matrix
    itself as SkyCraft does, logged once.
  - *Our objects' shadows*: our own sun shadow map (2048 x 2048, orthographic down the sun's rays,
    4,480 Skyrim units each way, snapped to whole texels and turned only in quarter-degree steps,
    SkyCraft's `SetSunShadowMatrix`), drawn with our objects (bones and alpha test included). Used
    (a) by our objects' pixel shader (they shadow themselves and each other) and (b) by a full-screen
    pass before them (SkyCraft's `OverlayPS`, multiply blend): each Skyrim pixel's position comes
    back from Skyrim's depth, and where our objects block its sun, the sun's share of its light is
    taken away (estimated from Skyrim's ambient and sun at that normal). Where Skyrim's own shadow
    mask (`kSHADOW_MASK`) already says shaded, nothing is taken (no double shadows); the mask is
    used only after a probe shows it holds 0-1 values with some lit (SkyCraft's `ReadProbe`).
    Outside only (an interior's light has no shadow map here); off while the directional light (sun,
    or the moon at night) is near or below the horizon. *Test 83: works outside in daytime.*
    Interiors (Skyrim's shadow-casting point / spot lights) would need their own work: not in
    SkyCraft, not planned yet.
  - *Normal maps*: each part's `$bumpmap` (not `$ssbump` ones) goes as a second texture, flagged
    linear (not sRGB). Corners carry their tangent (MeshVertex `userdata`: tangent x, y, z,
    handedness; `tangent` with handedness 1 if not). Surface normal = T x nx + B x ny + N x nz, B =
    handedness x (N x T). Not sure which way Source's green channel points: one constant in the
    shader, logged; checked by eye.
  - *Phong shine*: `$phong` parts with `$phongexponent` (default 5) and `$phongboost` (default 1);
    the mask is the normal map's alpha, or the base texture's with `$basemapalphaphongmask`. Sun
    and point lights get a highlight `boost x mask x max(0, r.v)^exponent`. No `$envmap`
    reflections, no `$phongexponenttexture`, no Fresnel ranges yet.
  - Render block **version 3**: `GmskyRndVertex` 40 -> 56 bytes (tangent), `GmskyRndPart` 24 ->
    32 bytes (normal map id, phong exponent, boost; flags `PHONG` 0x20, `BASEALPHA_PHONGMASK`
    0x40), `GmskyRndTexture.flags` `LINEAR` 0x1. Flat tables: 20 numbers per corner, 8 per part.
  - Logs: shadow cascades captured (size, format, slices, conventions), our shadow map's casters and
    ms, shadow mask probe result, normal maps / phong parts per model.
- **Phong fix + indoor shadow probe (user, 2026-10-05, after test 84: "father gregori looks like he's
  drenched in sweat ... Can you grab said textures from skyrim objects to compare values?").**
  - *What the files say* (read-only from both installs; extracted to the scratch folder, not the
    repo). Garry's Mod's own archive overrides HL2's materials:
    | Material | exponent | boost | Fresnel ranges | mask (average) |
    |---|---|---|---|---|
    | Grigori's head (`grigori_head.vmt`) | 10 | 1 | [0.1 0.5 1.5], tint [0.9 0.8 0.7] | normal map alpha 0.22 |
    | Grigori's robe (`monk_sheet.vmt`) | 10 | 1 | [0.05 0.5 1] | 0.32 |
    | Citizen (`citizen_sheet.vmt`) | 6 | 0.3 | [0.05 0.3 0.5] | 0.50 |
    | Skyrim male head (`malehead.nif`) | glossiness 33 | strength 2.69 | colour (0.63 0.76 1.0) | `MaleHead_S` 0.017 |
    | Skyrim male body | glossiness 23 | strength 4.50 | | `MaleBody_1_S` 0.018 |
    | Skyrim steel sword | glossiness 80 | strength 1.0 | + environment map | normal map alpha 0.19 |
    Effective head-on strength (boost x mask x Fresnel): Grigori's head as G5b drew it 1 x 0.22 =
    **0.22** (no Fresnel); in Source 0.22 x 0.1 = **0.022**; Skyrim's skin 2.69 x 0.017 = **0.046**.
    So G5b's shine was about 10x Source's and 5x Skyrim's on faces seen head-on: the missing
    `$phongfresnelranges`.
  - *Fix*: Source's phong as VertexLitGeneric does it: Fresnel `f = (1 - n.v)^2`, value =
    lerp(min, mid, 2f) below 0.5, lerp(mid, max, 2f - 1) above (default ranges [0 0.5 1] when the
    material has none; not sure of that default, logged per part); `$phongtint` (default white);
    `$phongexponenttexture` (exponent = 1 + 149 x red; `$phongalbedotint` tints by the albedo by the
    green channel); highlight x max(0, n.l). Render block **version 4**: `GmskyRndPart` 32 -> 64
    bytes (Fresnel ranges, tint, exponent texture id; flag `ALBEDOTINT` 0x80).
  - *Indoor shadows, step I1: probe only (log, no drawing change).* Skyrim's indoor shadows come from
    a few shadow-casting lights (`BSShadowFrustumLight`, `BSShadowParabolicLight`), each with its own
    shadow map(s). Not in SkyCraft. Hook their Render (vtable entry 0x0A, as the sun's) and log,
    for each light drawn: its class, position, radius, its descriptors (count, slice, render
    target, port, camera) and where its light transform puts the light itself and points 100 units
    along each axis (to learn the projection: perspective for frustum lights; parabolic is
    unknown); the depth buffer bound (size, format, array size). Also the per-second count of
    active shadow lights by class. Steps I2 (Skyrim's indoor shadows on our objects) and I3 (our
    objects' shadows from those lights) are planned from these numbers.
- **Test 85 results and the next fix (2026-10-05).**
  - *Phong*: now very subtle; the user is fine with it for now. **Polish item: hone the phong values**
    (and the other polish items below).
  - *Too dark (user: "The models are still TOO DARK").* Cause found by reading how vanilla Skyrim
    lights (Community Shaders' reconstruction of Skyrim SE's `Lighting.hlsl` and `Color.hlsli`;
    read, nothing copied; not checked against Skyrim's own compiled shaders, which have no names):
    vanilla Skyrim uses its colour textures **as stored** (sRGB numbers used directly, no
    gamma -> linear) and its light colours as they are. We read Source's colour textures through
    sRGB views (converted to linear), which makes a 0.5 texel 0.21 (2.3x darker) and a 0.3 texel
    0.07 (4x darker). Fix: colour textures uploaded as plain UNORM, as Skyrim reads its own; switch
    `gmsky_render_linear_albedo 1` (frame flag 0x20) converts in the shader instead, to compare.
  - *Indoor shadow probe I1 results:* every indoor shadow light was `BSShadowParabolicLight` (also
    "omni"), 2 descriptors each; maps in a shared 4096 x 4096 x 8 depth array (format 53, R16, view
    D16), one slice per light, the two descriptors in its lower and upper halves (ports y 2048-4096
    and 0-2048). Each transform is affine (w = 1), as v x M: light-local x = world z - light z,
    y = -/+ (world y - light y) (the two descriptors differ only in this sign), z = 1.111 x (world
    x - light x) - 0.11. The descriptors' cameras are orthographic (+-1, near 0.1, far 1) at the
    light, forward +x, up +y / -y. So the paraboloid projection itself is done in Skyrim's shaders,
    not in this matrix; which local axis splits the two halves is not known yet. Next step I1b:
    read back a few texels of a light's map and match them with known geometry, or find the
    lookup in the lighting shader, before drawing anything with them.
  - *Polish list (user, 2026-10-05):* phong values; physgun drag lag; Skyrim's directional ambient
    is a 3x4 matrix in its shader (`mul(DirectionalAmbient, float4(n, 1))`), we use the weather's
    six colours (SkyCraft's way); Source's `$halflambert`; Source `$envmap` reflections on metal.
    *Added 2026-10-06 (test 98):* outdoor sun shadows too soft and faint next to Skyrim's own (the
    user: indoor ones are "way better"). Checked: the comparison isn't reversed (lit = compared depth
    at or before the stored one, as before the change). Likely cause: the softness is one uv radius
    (`kShadowSoftUv` 0.006) for all maps; on the sun's 4,096-texel cascades that is about 25 texels,
    which covers far more ground than on the indoor maps, so dark edges are smeared into grey. To
    try: a separate, smaller sun radius (Skyrim's own `ShadowSampleParam.z`, read from its shader
    constants, would be best); log the cascade's width in units to set it in world units.
- **Test 86 results and step N1: NPC shadows (2026-10-05).** Brightness fix: right (user:
  "Perfect"). New report: NPCs "don't seem to be affected by shadows", "pop out a ton and look very
  flat"; lights do reach them. NPCs and props go through the same shader and the same shadow
  lookups, so the reason is not certain. Two parts:
  - *Contact shadows (SkyCraft's BlobAO, MIT), which we had left out.* **Removed again 2026-10-06 at
    the user's request** ("remove that fake contact shadow"); the feet data stays for the NPC probe.
    SkyCraft darkens the ground
    softly under every player's and mob's feet, because its bodies are drawn after Skyrim's own
    ambient occlusion and "would otherwise look pasted onto the ground". Garry's Mod now marks NPCs,
    nextbots and players (`GMSKY_RND_OBJ_CONTACT`) and sends their feet (`GetPos`) and size
    (collision box width and height); render block version 5, the object grows to 48 bytes.
    Skyrim's darkening pass (the one that already lays our sun shadows on Skyrim) also darkens up to
    24 blobs, nearest first: radius 1.25 x max(width, 35 Skyrim units), as SkyCraft's 1.25 x max(width,
    half a block). It runs day and night, indoors and out (shadows on or not). Like SkyCraft, the
    same darkening also falls on our props' upward faces, not on the characters themselves.
  - *User, later: an NPC dragged into a shaded area stays fully lit,* so Skyrim's sun shadow does
    not reach NPCs at all (a real fault, not only the missing contact shadows). Switch
    `gmsky_render_shadow_debug 1` (frame flag 0x40) colours our objects flat: red = in Skyrim's sun
    shadow, green = sunlit, blue added = in our objects' shadow, yellow = sun shadows off.
  - *A probe, to know rather than guess, whether Skyrim's sun shadows reach the NPCs.* Every 2 s, for
    up to 5 characters, Skyrim's sun shadow and ours are read at three heights (feet, middle, top)
    on the graphics card and logged ("contact shadows: probe ..."), 1 = lit, 0 = shadowed. If an NPC
    standing in a building's shadow logs 1.00, the lookup is wrong for it; if it logs 0.00 but looks
    bright, the shading (ambient) is the problem.
  - *Test 88 result:* the probe read 0.00 (shadowed) on NPCs in shade, so the lookup was right; the
    user found the cause: self-illuminated materials skipped all lighting, so the whole surface
    stayed lit. Fixed the Source way: light and shadow the whole surface, then blend to the glow by
    its mask (`$selfillummask`, else base alpha) and `$selfillumtint` (render block version 6).
  - Differences from SkyCraft: SkyCraft also has VoxelAO (contact shadows where Minecraft blocks
    meet Skyrim's world); not done for props (it needs a voxel grid of Minecraft blocks).
- **Next: indoor shadows, steps I1b-I3 (planned 2026-10-06, after test 89; waiting for the
  user's go-ahead).** Indoors Skyrim has no sun; its shadows come from a few point lights that cast
  shadows (`BSShadowParabolicLight`, 2-3 active at a time, test 85). Today our objects take those
  lights as plain lights: no shadow on them, none from them.
  **Differs from SkyCraft:** SkyCraft has no indoor shadows (it adds Skyrim's shadow lights to its
  point lights, as we do now). So this part is ours, built on Skyrim's own data the way SkyCraft
  uses the sun's.
  - **I1b. Learn how Skyrim reads its parabolic maps (probe only, nothing drawn differently).**
    Known (test 85): per light, two halves of one slice of a 4096 x 4096 x 8 D16 array, each with an
    affine transform (light-local axes: x = world z, y = -/+ world y, z = 1.111 x world x - 0.11,
    relative to the light). Unknown: how a direction becomes a texel (the paraboloid divide), which
    half is which, and what depth is stored (distance / radius, or something else).
    Method, by numbers, not by eye: right after Skyrim renders a light's map (the parabolic hook we
    have), copy its slice. In our frame, a full-screen probe turns Skyrim's depth back into world
    positions (as the sun-shadow pass does), and for each of ~8 candidate formulas looks up the
    copied map and checks: is a pixel's predicted depth equal to the stored one where the light
    clearly sees it (floor and walls near the light)? The log line per light gives each formula's
    match rate ("H3: 94% of 4096 samples within 1%"). The right formula should match about 90% or more,
    the wrong ones near chance. If none does, stop and report (STATUS.md rule) instead of guessing.
    *Built (2026-10-06, user: "Start on the indoor shadow plan"):* every 5 s the parabolic hook
    copies one light's slice (a different light each time; 4096 x 4096 D16, about 32 MB, a copy every
    5 s). The same frame, a probe pass takes a 32 x 32 grid of Skyrim's pixels, turns them back into
    positions, and for each of **96 candidates** looks the point up in the copy. A candidate picks the
    paraboloid's axis (world z, y or x; y is the one that flips between the two descriptors, so the
    likely one), which hemisphere sits in the map's lower half, the order and signs of the other two
    axes, and whether the second half is mirrored; uv = (the other two components) / (1 + |axis
    component|) of the unit direction, into that half. The numbers come back to the CPU, which tests
    three depth formulas per candidate: distance / radius, (distance / radius - 0.1) / 0.9 (the
    descriptors' near 0.1 / far 1), and the best straight line. Logged ("indoor shadow probe I1b"):
    best and median match rate, the top 6 candidates with each formula's rate, per-half rates and
    correlation, and 10 (distance / radius -> stored) pairs of the best one.
    **Not covered by the 96:** a uv not divided by (1 + axis) (for example a cube map or an
    octahedral layout), halves side by side instead of stacked, or the scaled light-local z (1.111)
    used in the divide. If the best candidate is near the median, it's one of these.
    *Test 91:* no consistent match. Depth looks like distance / radius (slope 0.99-1.01 in two runs,
    paraboloid axis world x), but the uv layout is not one of the 96. Most high scores were flat-line
    fits to uniform texels. Attempt 2: the first 4 captures are dumped to `logs/parabolic/` (light,
    1024 points relative to it, the raw 4096 x 4096 map) to fit offline and to look at the layout.
    *Test 92:* the maps are two stretched discs stacked (top / bottom halves); my best layout matched
    22%; stopped (STATUS.md). *Attempt 3 (user: "Just use community shaders"):* Skyrim's own lookup
    as Community Shaders reconstructs it (`Utility.hlsl`, MIT): the point through the light's
    transform **including its 1.111 stretch**, the -z side in the lower half with v flipped,
    depth = |p| / radius. 35% exact depth matches offline; the misses lean deeper (likely the map's
    straight-line fill across curved paraboloid triangles). So the test is now Skyrim's own answer:
    each shadow light writes lit / dark into one channel of Skyrim's screen shadow mask (the
    light's `maskIndex`), and the probe logs how often our lookup agrees with it, per bias.
    **Differs from SkyCraft:** SkyCraft has no indoor shadows; this part follows Community Shaders.
    *Test 93: confirmed.* Our lit / dark agrees with Skyrim's own shadow mask on 99.86% of 4,942
    points at bias 0.005 (in units of the light's radius); 97.1% at 0, 98.2% at 0.02. **I1b done.**
  - *I2 built (2026-10-06):* each parabolic map copied as Skyrim draws it into 4 slots of ours
    (least recently drawn light gives up its slot); our point lights with a slot get Skyrim's lookup
    (bias 0.005, 3 x 3 texels) on their diffuse and shine. Lights listed both as lights and as shadow
    lights are used once. Details and test: MODLOG.md.
  - *I2 works (test 94).* *I3 built (2026-10-06):* our objects drawn into our own dual-paraboloid
    map per slot (1024 x 1024, Skyrim's layout and formula); the darkening pass removes the blocked
    lights' share from Skyrim's pixels, and our objects shadow each other with the same maps.
  - *I3 works (test 95, user: "Flawless").* *Softer edges (built 2026-10-06):* both kinds of indoor
    lookup use Skyrim's own filter (8 filtered samples on a spiral, turned per pixel and per frame,
    smoothed by Skyrim's TAA) instead of 3 x 3 hard samples; radius 0.006 of the map (chosen by eye;
    Skyrim's own radius, fPoissonRadiusScale, not known).
  - **I2. Skyrim's indoor shadows on our objects.** Copy each active shadow light's slice after
    it renders (kept with its transforms, as `CaptureSunShadows` does for the sun). In our pixel
    shader, each point light that is a shadow light gets a visibility from its map (the formula
    from I1b, 3 x 3 samples, small offset off the surface), multiplying its diffuse and shine.
    Test: an NPC walked behind a pillar next to a torch-lit wall goes dark on the side the pillar
    hides; with `gmsky_render_shadow_debug 1` the colours show it (indoor version of the colours).
  - **I3. Our objects' shadows onto Skyrim indoors.** For each of the 2-3 shadow lights, render
    our objects into a small depth map of our own (two paraboloid halves, the same formula, 1024 x
    1024 each). The darkening pass then, per Skyrim pixel, removes that light's share where our
    objects hide it: share = the light's estimated contribution (colour x falloff x n.l, as our
    shader computes it) / (ambient + all lights), the indoor form of the sun's `keep`. Test: a
    prop between a torch and a wall throws a shadow on the wall that moves with it.
  - **Not sure:** whether Skyrim redraws every light's map every frame (if not, copies stay valid
    until it does, as with the sun); whether interior shadow lights ever use the frustum
    (spotlight) kind (test 85 saw none); the cost of 2-3 extra small depth renders (logged per
    second).
  - **Other open items, for after this or whenever you choose:** G7 effects (tracers, explosions,
    physgun beam: needs your choice of route); G8 removing the old picture-route code (W1-W4);
    props falling through Skyrim's ground; a Combine soldier texture that resolves to Source's
    error texture (test 89 log); the polish list above.
- **G7. Effects (later, needs its own plan and your choice).** Options: (a) for effects drawn from
  Lua (`render.DrawBeam`, `DrawSprite`, `DrawQuadEasy`), a collector like SkyCraft's that wraps
  those calls; (b) for engine effects, send events (explosion at X, tracer from A to B) and draw
  their sprites in Skyrim; (c) Skyrim's own explosion and magic effects. Unknown which engine
  effects Lua can see at all; the first step is a log of what happens when you shoot.
- **G8. Clean-up.** When G3-G5 cover the world objects, the world layer and depth picture (W1-W4
  code in `liveoverlay.lua`, `capture.rs`, `Draw.cpp`) are removed. The hands / HUD picture stays.
  **Detailed plan (2026-10-06, after test 96; the user: "Commit and continue").** The world layer
  has been off by default since G3b (`gmsky_overlay_world 0`), so removing it should change nothing
  you can see. One step, revertible as one commit:
  - *Garry's Mod, `liveoverlay.lua`:* removed: `gmsky_overlay_world`, `gmsky_overlay_depth`, the
    wipe at the entity pass, the see-through pass with alpha writes off, the fog depth picture
    with its calibration squares, the camera sent with the picture, the world-layer copy, the
    "something happened" hooks (shots, effects, new entities) and the "live overlay world" status
    line. Kept: the overlay switch, the wipe where the view model / hands / HUD start, alpha
    forced for the HUD, the halo fix, the view model and hands stamped solid, the hidden window,
    the blur flag, `gmsky_overlay_shot`, the status line.
  - *Garry's Mod module, `capture.rs` / `lib.rs`:* removed: `gmsky.OverlayGrabWorld`,
    `OverlayGrabDepth`, `OverlayView`, `OverlayDepthCentre`; the extra copies, the fog
    calibration and depth conversion, their counters and tests. The HUD picture path is unchanged.
  - *Protocol:* the flags `GMSKY_OVF_WORLD` / `GMSKY_OVF_DEPTH` and the slot header's depth size
    and camera fields go; the header stays 128 bytes (the space becomes `reserved`). The overlay
    block's version goes 2 -> 3, so an old build on either side refuses the block instead of
    misreading it (rebuild both, as always).
  - *Skyrim, `Draw.cpp`:* removed: the world-layer and depth textures, the re-projection, Skyrim's
    depth probe and the "world layer drawn ..." log line. The HUD picture, its sRGB blending and
    the blur stay as they are.
  - *Skyrim, `World.cpp`:* the in-frame drawing asked that probe whether Skyrim's depth is
    reversed. The probe only ran while a world layer came, so in every test since G3b it said
    "probe not ready, from the matrix" (tests 91-96 logs). The camera matrix's answer is now the
    only one; the log line stays.
  - *Harness:* the W1-W4 check is replaced by one that the frame is still wiped at the view
    model, with the view model stamped solid.
  - Test: hands, weapon and HUD look as before (with HOLOHUD's blurred plates); props, NPCs and
    effects look as before; Garry's Mod's log has no "live overlay world" line; Skyrim's log no
    "world layer drawn" line. **Done (test 97).**
- **After test 97 (the user's two notes).**
  - *FOV fix (the physgun halo smaller than the prop).* Garry's Mod's FOV is the width of a 4:3
    view, widened for a wider screen; Skyrim's `worldFOV` is the width of its actual screen (its
    frustum at 100 on 16:9 was tan 50 degrees; Garry's Mod's view at 100 is tan 57.8). Copying the
    number made Skyrim's view narrower: Skyrim's props came out bigger than Garry's Mod's halo,
    hands and anything else in its picture. `Camera.cpp` now converts with Skyrim's screen shape,
    and logs both half-view tangents every second (equal = same view). Differs from SkyCraft,
    whose comment says Skyrim's FOV is a 4:3 width too; the frustum we logged says otherwise.
  - *Outdoor shadows like indoor ones.* The sun used 3 x 3 samples (hard yes / no on Skyrim's
    cascades, so stair-stepped edges). Now the same filter as indoors and as Skyrim's own
    (Community Shaders' `SampleShadowPCF`): 8 rotated spiral samples on Skyrim's cascades and on
    our sun map (same softness in world units as Skyrim's nearest cascade), and Skyrim's blend from
    cascade 0 to cascade 1 where they overlap (`startSplitDistances`), so no line between them.
  - *Proposed, waiting for the user: G9, the hands, weapon and HUD drawn in Skyrim too.* See the
    hand-over of 2026-10-06: the view model could be drawn like the props (it is a model); Lua's
    2D HUD drawing (HOLOHUD, Sandbox's HUD, menus) has no geometry to send and would stay a picture
    or need a collector for Lua's 2D drawing calls. Not started; first see whether the FOV fix
    already makes the two match.

**Not sure (checked by logs, not guessed):** whether `util.GetModelMeshes` gives normals and the
model's real skin materials for every model (the wiki lists a `skin` argument); whether
`file.Read` returns `.vtf` files from VPKs and Workshop addons intact (binary); how much Lua time
`GetModelMeshes` costs on a big model (it is cached per model, so once); whether the hook point
SkyCraft uses behaves the same with our plugin's other hooks.

### One collision system: Skyrim's world as Garry's Mod physics (planned 2026-10-06, waiting for the user's go-ahead)

**History of this plan.** A first version (same day) kept the player on the trace ground and
added physics ground for props only, with the player filtered out. The user rejected running two
systems side by side and pointed at **InfMap2** (`https://codeberg.org/googer/infmap2`, read in
`reference/infmap2`, commit `8077c17`, not tracked) as the way to do it properly.

**The problem in plain words.** Garry's Mod has two ways of asking "is something there?":
*traces* (an invisible line or box swept through the world: player movement, bullets) and its
*physics engine* (`vphysics.dll`: props, ragdolls, grenades, AR2 balls). Today Skyrim's world
answers traces only (the `gmsky_field` entity answers `TestCollision` from the Rust triangle
store, step D), so physics objects fall through it to gm_flatmap's floor far below.

**What InfMap2 does.** InfMap2 makes huge maps in Garry's Mod. Its terrain is **ordinary physics
objects**: one `inf_chunk` entity per chunk (20,000 units), its triangles made with
`PhysicsFromMesh` (about 2,300 triangles in `gm_inf_bliss`), on the server **and** on the client
(the client needs them to predict the player's movement). Player movement, bullets, props,
ragdolls and vehicles all collide with that one thing. **One system.** The chunk entity's recipe
(`lua/entities/inf_chunk.lua`):
- `PhysicsFromMesh(triangles)`, then `SetSolid(SOLID_VPHYSICS)`, `SetMoveType(MOVETYPE_NONE)`,
  `EnableCustomCollisions()`;
- on its physics object: `EnableMotion(false)`, `SetMass(50000)`, game flags
  `FVPHYSICS_CONSTRAINT_STATIC` and `FVPHYSICS_NO_SELF_COLLISIONS`;
- `AddSolidFlags(FSOLID_FORCE_WORLD_ALIGNED)`, `AddFlags(FL_STATICPROP)`,
  `AddEFlags(EFL_KEEP_ON_RECREATE_ENTITIES)`, `DrawShadow(false)`; the physgun can't pick it up;
  an empty chunk gets a zero box in `COLLISION_GROUP_WORLD`;
- a chunk is made the moment any entity enters a chunk that has none, and removed (every 60 s)
  when no entity is in it any more.
Its other half is for maps bigger than Source's ±16,384 limit: every entity carries a chunk
number ("megapos"), all chunks physically sit on top of each other at the map's middle, a
`ShouldCollide` hook lets only things in the same chunk touch, and Lua's position functions
(`GetPos`, `SetPos`, traces, ...) are replaced to add the chunk offset back.

**Why we didn't do this before: the crash.** Tests 12-17 built Skyrim's ground with
`PhysicsFromMesh` and Garry's Mod crashed 5 times in `vphysics.dll+0xaaff0` (a 16-bit list in the
physics engine overflowing), every time when a new piece was created under the **moving**
player (STATUS.md "Known engine bug"). InfMap2 creates chunks under moving players and is a
public Workshop addon, so either its recipe avoids the crash, or its chunks are created so rarely
(one per 20,000 units) that it seldom happens, or our data triggers something theirs doesn't. **We
don't know which.** Our build had none of InfMap2's settings (no `FVPHYSICS_CONSTRAINT_STATIC`,
`FVPHYSICS_NO_SELF_COLLISIONS`, `FL_STATICPROP`, mass, world-aligned flag) and built far more
often (a new piece every few hundred units). So the first step tries to crash it on purpose.

**Licence.** InfMap2 has **no licence file** (checked 2026-10-06). Without one, its code is all
rights reserved by default: we read it to learn how it works and write our own code; we copy no
code from it. Credited in THIRD-PARTY-NOTICES as an idea source when we build on it.

**Steps, each tested on its own:**

- **S0. Measure the problem (logs only, no fix).** Once a second, for every physics object that
  isn't the player or ours (props, ragdoll parts, grenades): class, model, position, speed,
  awake/asleep, and its height above Skyrim's ground straight under it (`gmsky.Sweep` down from
  2,000 units above; "no ground known" if its region hasn't arrived). A line the moment one first
  goes more than 32 units below: `props: FELL THROUGH <model> at (...), ground z, prop z, speed`.
  Logged once: gm_flatmap's real floor under the player (where props end up). On the client too:
  how many client-only physics objects exist (client death ragdolls, gibs).
  *Test:* spawn 5 props, drop a grenade, throw a prop with the physgun. *Expect:* each logs
  `FELL THROUGH`. This is the number every later step must bring to zero.
- **S1. Crash experiment: InfMap2's recipe on Skyrim's real triangles** (Skyrim + Garry's Mod,
  as tests 12-15 crashed with). New Rust function `gmsky.RegionTris(rx, ry, rz)` (one region's
  triangles, for Lua) and console command `gmsky_test_physground <old|infmap>`: every half second
  it builds the regions around the player into one physics entity and removes the previous one,
  **with the player colliding with it** (no filter; it sits exactly on top of the trace ground,
  the same triangles). Each build is logged before and after, so a crash names the build. A fresh
  Garry's Mod for each run:
  1. *Control, `old`:* our tests 12-17 entity setup. We **expect a crash** within minutes (4 of 4
     runs that built under a moving player crashed). If it survives 5 minutes / 600 builds of
     walking, sprinting and jumping, the bug can't be reproduced this way: I say so and stop.
  2. *`infmap`:* InfMap2's recipe exactly. *Pass:* no crash in 10 minutes / 1,200 builds.
  3. *`infmap` with props:* a stack of props resting and one bouncing on the entity while it is
     rebuilt under them. *Pass:* no crash; props stay on top.
  If run 2 or 3 crashes, this route is blocked: I record it, write STATUS.md, and we decide
  together. If run 2 passes, I can also (optional, your call) find which of InfMap2's settings is
  the one that matters, by switching them off one at a time.
- **Rule from tests 101-103 (the user's, 2026-10-06): never two meshes for the same place at
  once, on either side, like InfMap2 (one mesh per chunk, ever).** Both setups crashed when a new
  mesh was built on top of the previous one while it still existed. A mesh is replaced by
  destroying the old one's physics first (`PhysicsDestroy`, immediate; `Remove` waits for the end
  of the tick), then building the new one in the same tick. (InfMap2's different chunks do sit in
  the same physical spot, but its `ShouldCollide` stops them colliding with each other; two meshes
  of the same ground that may collide never exist there.) S1's experiment follows this rule from
  run 2b on; S2's chunks lie on a fixed lattice and never overlap.
- **S2. Physics ground from Skyrim's regions (`gmsky_ground physics`).** New file
  `gmod/lua/gmsky/physground.lua`: InfMap2-style chunk entities, each holding the triangles of a
  block of 2×2×2 Skyrim regions (1,024 Skyrim units, ~544 Garry's Mod units; about 150 entities
  for today's area instead of 1,199; the size is a guess to tune from build times). Built on the
  server and, from the same Rust store (one module per process), on the client. Nearest the
  player first, then near other physics objects, at most 2 builds or 2 ms per tick. A chunk is
  rebuilt only when one of its regions' triangles really changed (a per-region version number in
  the store, so Skyrim's 1 s refresh of nearby regions doesn't rebuild unchanged ground).
  The player's existing "waiting for Skyrim's collision" hold now waits for the chunk under its
  feet. Switch: `gmsky_ground physics` / `shapes` (today's) for comparison only, one at a time,
  never both; the trace field is off in physics mode.
  *Logs, once a second:* chunks built / kept / removed, triangles, build ms (slowest), on both
  sides; "player standing on physics ground"; holds. *Test:* the test 23 walk (a building's walls,
  stairs, an arch, an interior through a load door, trees, fences, a fast physgun launch) in
  physics mode. *Pass:* no crash, feels the same as `shapes`, holds rare and short.
- **S3. Props, ragdolls and grenades on it.** Mostly free once S2 works. Added: a prop moving into
  a chunk that isn't built yet is frozen for that moment and released with its speed (like the
  player's hold; InfMap2 doesn't need this because it makes its terrain itself, instantly; we
  wait for Skyrim's data). Chunks stay while a physics object rests on them, like InfMap2's
  "remove when nothing is in it". Client-only physics (if S0 found any) collides with the client
  copy. *Test:* S0's test. *Pass:* 0 `FELL THROUGH`; props rest, roll down slopes, stack; a
  grenade bounces; a Garry's Mod ragdoll lands.
- **S4. Props live in Skyrim's world and in Skyrim's save** (the user's choice, 2026-10-06; its
  own steps P1-P5 below).
- **S5. Remove the trace ground.** After you've confirmed S2-S3 in play: `gmsky_field`'s shapes
  path, the ray-grid mode and their switches go; Skyrim's world is then only physics objects. The
  Rust sweep stays only if something still uses it (S0's log; otherwise it goes too). Skyrim's
  NPC stand-ins are trace-only and are not part of this plan (props hitting NPCs comes after).

**S4 in detail: props saved in Skyrim's world and Skyrim's save (planned 2026-10-06).**
The user's goal: place a prop in Windhelm, travel to Solitude and back, and the prop is still
exactly where it was; saving and loading Skyrim keeps the props consistent with that save.

*The rule that makes this robust:* a prop's real home is a **place in Skyrim**: a worldspace
(Tamriel, an exterior) or an interior cell, plus a Skyrim position. Its Garry's Mod position is
only worked out from that while you are near it. Windhelm and Solitude are far further apart than
Garry's Mod's map can hold (about 200,000 Skyrim units ≈ 106,000 Garry's Mod units; the map holds
±16,384), so props far away cannot stay in Garry's Mod at all. They are **unloaded** into a
record and **loaded** back when you come near, the same way Skyrim itself unloads far cells.

*Three states for every saved thing:*
1. **Active:** a normal Garry's Mod entity, simulated, on built physics ground.
2. **Waiting:** a Garry's Mod entity whose physics ground isn't built yet: frozen, speed kept
   (S3's hold). A prop the player froze with the physgun stays frozen either way (a separate
   "frozen by you" flag, so we never unfreeze it).
3. **Unloaded:** only a record, no entity. Happens when it is more than 10,000 Skyrim units from
   the player (≈ 5,300 Garry's Mod units, so with the player up to 8,000 from the map's middle it
   still fits inside ±16,384), or in a different worldspace / cell (a load door). Loaded again
   (as Waiting) when within 8,000 (the gap stops a prop flickering in and out at the edge).

*What a record holds:* a permanent id; the place (worldspace or cell, by Skyrim form ID, saved
in a load-order-safe way through SKSE's form ID resolving); the Skyrim position of the group's
first entity; and the group itself as **Garry's Mod's own duplicator data** (the system Sandbox's
saves and Advanced Duplicator use): model, skin, colour, material, bodygroups, frozen state,
ragdoll poses, and the constraints (welds, ropes) between the entities, positions relative to the
first entity. Using the duplicator means anything it can copy is saved the way Garry's Mod
itself would, and constrained contraptions load and unload as one piece. Compressed
(`util.Compress`). Not saved: things that only exist for a moment (grenades, AR2 balls, rockets,
gibs), Garry's Mod NPCs (later, if wanted).

*The re-centre* (the old S4 choice): option (a). Every Active and Waiting entity moves with the
player by the same shift in the same tick (speeds kept, constrained groups together). Their
Skyrim positions don't change, because the anchor moves by the same amount. InfMap2's chunk
numbers (option b) are not needed: anything that wouldn't fit is unloaded instead.

*Who keeps the records: Skyrim, inside its save.* Two ways were compared:
- **(A) Everything in Skyrim's save (chosen, the robust one).** SKSE lets a plugin write its own
  data next to each save (the "co-save", `.skse`, written and deleted together with the save by
  SKSE). All records go in there. A save then carries its props wherever it goes (copied, cloud
  synced, deleted), and loading an older save brings back exactly that save's props. Nothing is
  written into the Garry's Mod install (AGENTS.md rule 4).
- **(B) SkyCraft's planned way.** SkyCraft's design (`docs/DESIGN.md` §11, not built in SkyCraft)
  stores only a save id in Skyrim's save and keeps the data on the other game's side, keyed by
  that id. Smaller saves, but the two can drift apart (a copied or deleted save, a missing file),
  and here it would mean writing into Garry's Mod's `data` folder.
- Also considered: one invisible Skyrim object (a marker) per prop, so Skyrim's own save tracks
  where it is. Rejected for now: two places to keep in step (the marker and our data), and more of
  Skyrim's engine to get right.

*How the records stay current (the protocol):* Skyrim holds every record in memory. Garry's Mod
sends a record whenever a saved thing changes state (spawned, removed, unloaded, came to rest,
frozen / unfrozen) and once a second for anything still moving, through a new byte ring Garry's
Mod → Skyrim. When Skyrim saves, it first asks Garry's Mod for a final flush and waits up to
250 ms for it (logged; if it times out the save uses records at most 1 s old, and says so). When a
save is loaded, Skyrim tells Garry's Mod to remove every saved-thing entity **without** recording
them (so the loaded save wins), then sends the records near the player. A new game starts empty.
If Garry's Mod is restarted mid-session, Skyrim sends the records again. **Skyrim's save is the
truth for the world's props; Garry's Mod is still the truth for the player** (AGENTS.md).

*Collision far from the player:* Skyrim only collects collision ±2,800 Skyrim units around the
player today. Active things farther out need ground too, so Skyrim also collects the regions
around each Active, moving thing (it knows where they are from the records). Things at rest
don't need it until something wakes them; they are Waiting until their ground is there.

**P steps (after S3), each tested on its own:**
- **P1. Skyrim position for every saved thing, and the re-centre moves them.** Logged every
  second per thing: Garry's Mod position, Skyrim position (must not change during a re-centre).
  *Test:* `gmsky_recentre_at 500`, place props, walk around. *Pass:* props stay put, Skyrim
  positions identical (to 0.1 unit) before and after every re-centre.
- **P2. Unload and load by distance and place, records kept in Garry's Mod for now.** *Test:*
  place a prop and a welded contraption, walk 12,000 Skyrim units away (unload logged), come
  back (load logged), then into a house and out again. *Pass:* same Skyrim position to 0.1 unit,
  same rotation, welds intact.
- **P3. Records in Skyrim.** The new ring; Skyrim's record table; Skyrim sends records back.
  *Test:* place props, restart Garry's Mod only. *Pass:* props come back where they were.
- **P4. Skyrim's save.** SKSE co-save writing / reading / clearing, the flush on save, clearing on
  load. *Test:* your scenario: a prop in Windhelm, save, fast-travel to Solitude, place one there,
  save, fast-travel back (the Windhelm prop is there), load the first save (no Solitude prop),
  quit Skyrim and load again. *Pass:* every prop exactly where that save had it.
- **P5. Collision around far moving things** (Skyrim harvests near Active things). *Test:* push a
  prop down a long hill away from you. *Pass:* it rolls all the way, 0 `FELL THROUGH`.

**Not sure:** whether Garry's Mod's duplicator gives positions relative to a chosen entity cleanly
(`duplicator.SetLocalPos`); whether pasting re-creates every addon entity correctly (Sandbox
props and ragdolls certainly, others depend on the addon); how big records get (estimate: under
1 KB compressed per prop); whether SKSE's save callback can wait 250 ms without trouble.

**Differences from InfMap2:** our triangles come from Skyrim (streamed, can arrive late, so holds)
instead of a height function; chunks of 2×2×2 Skyrim regions (~544 units) instead of 20,000
units; chunks rebuilt when Skyrim's data changes (InfMap2 never rebuilds); our re-centre plus
unloading far props instead of chunk numbers; props saved in Skyrim's save (InfMap2 relies on
Garry's Mod's own saves); no code copied (no licence).
**Differences from SkyCraft:** SkyCraft adds Skyrim's shapes to Minecraft's collision query (one
system there too); Garry's Mod's physics engine has no query to hook, so we build physics objects.
SkyCraft has no crash to test for, no physics props, and no floating origin.
SkyCraft's save design keeps only an id in Skyrim's save (and never built it); we keep the
records themselves in Skyrim's co-save.

**Not sure (to be answered by logs):** whether InfMap2's recipe avoids our crash (S1); build cost
of Skyrim's densest regions (cities); whether the client's prediction matches the server's exactly
when both build the same mesh; whether ~150 networked chunk entities cost much bandwidth (they
never move); how player movement feels on physics meshes compared to the trace sweep (tests 10-16
walked on them fine).

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
| Floating origin re-centres cause visible jumps | Skyrim's position doesn't change in a re-centre (only Garry's Mod's numbers do); Skyrim logs its per-frame step through each one |
| `+lua_run` on the command line doesn't work | Type it in the console once per session |
| Garry's Mod updates break the module | It only uses the stable Lua C functions; stubs never change |
