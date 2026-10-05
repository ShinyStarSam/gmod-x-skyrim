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
- **2b-4. Skyrim's window size.** The Skyrim plugin opens the picture block and writes its back
  buffer size there each frame (read in its own `Present` path in 2c; for now from the renderer's
  screen size). Garry's Mod logs both sizes and whether they match. Making Garry's Mod draw at that
  size automatically is left to 2d (it means changing Garry's Mod's window size); until then the
  user sets Garry's Mod's resolution to Skyrim's, and 2c scales if they differ.

**Unknowns, measured in 2b-1 / 2b-3:** the back buffer's format (if it is `X8R8G8B8`, there is no
alpha in it and the copy would have to come from Source's own render target instead); whether
`StretchRect` from the back buffer is allowed (it needs the back buffer to be a render target,
which it always is, and the same format); the cost of a 1280 × 720 copy (3.7 MB, expected
~1-3 ms on the render thread).

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
