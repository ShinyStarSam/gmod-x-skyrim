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
  saw only ~20 changes per second. So Garry's Mod must not be throttled in the background
  (`engine_no_focus_sleep 0`), and Phase 1 should send the smoothed per-frame position from the
  client side, like SkyCraft sends Minecraft's partial-tick position, instead of the raw tick position.
- **Input:** Skyrim keeps the keyboard focus. Garry's Mod receives key and mouse data through shared
  memory and writes it into the player's command each tick with the `StartCommand` hook, so the
  Garry's Mod window does not need focus. We also set `engine_no_focus_sleep 0` so it does not slow
  down in the background.

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
(`SendLua`), which works (in-game test 2026-10-04). `+lua_run` on the command line was tried and
does nothing, so for now it is one console command per session. Automating it is a Phase 5 job.

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
6. **Skyrim plugin, puppet:** Skyrim's player follows Garry's Mod's position. Phase 0 done.

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
4. **Damage scaling:** a Garry's Mod crowbar does 10 damage, but Skyrim bandits have 50–300 HP.
   This gets decided in Phase 3.

## 15. Risks

| Risk | What we do about it |
|---|---|
| Player movement on many mesh chunks is jittery or slow | Measure in Phase 1 with logged frame times; fall back to fewer, larger chunks or box shapes |
| DirectX 9 → 11 picture sharing doesn't work | Copy through normal memory first, measure the cost |
| Floating origin re-centres cause visible jumps | Re-centre only while the player is on the ground; log each jump |
| `+lua_run` on the command line doesn't work | Type it in the console once per session |
| Garry's Mod updates break the module | It only uses the stable Lua C functions; stubs never change |
