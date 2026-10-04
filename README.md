# Gmod x Skyrim

Play Skyrim as a Garry's Mod player: Garry's Mod movement, weapons, health and HUD inside the real
Skyrim world. Both games run at the same time and talk through shared memory, the same approach as
[SkyCraft](https://github.com/chasmlol/SkyCraft). See [docs/DESIGN.md](docs/DESIGN.md).

> **Status: early.** Phase 0 done: Skyrim's player follows Garry's Mod. Phase 1 in progress: playing from the Skyrim window works.
> Single-player and offline only. Garry's Mod is always started with `-insecure` (VAC off).

## What works

- **Garry's Mod → shared memory** (tested in game 2026-10-04): after `lua_run require("gmsky")`
  in the console, Garry's Mod's player position, aim, speed and on-ground/crouch/alive state reach
  shared memory 66 times a second, and a test program (`fake_skyrim`) reads them. Closing Garry's
  Mod is detected within half a second.

- **Skyrim plugin, read-only** (tested in game 2026-10-04): loads through SKSE on 1.7.104, hooks the
  player's per-frame update, and reads Garry's Mod's live state through shared memory. It logs
  both players side by side to `logs/skyrim.log`. It doesn't change anything in Skyrim yet.

- **Skyrim's player follows Garry's Mod** (tested in game 2026-10-04, Phase 0 done): walking and
  looking around in Garry's Mod moves and turns the player in Skyrim, about 60 updates a second,
  standing on Skyrim's own ground and hills. Closing Garry's Mod gives Skyrim back its normal
  controls within 2 seconds. You control it from the Garry's Mod window, and Skyrim needs
  `bAlwaysActive=1` in its Skyrim.ini so it keeps running in the background.

- **Playing from the Skyrim window** (tested in game 2026-10-04, Phase 1 step 1): W/A/S/D, Shift
  (sprint) and the mouse move and turn the player with Garry's Mod's movement; E opens doors,
  containers and talks to people in Skyrim; Esc, ~, J, M, F9 open Skyrim's menus, and input goes
  back to Skyrim while a menu is open or the Skyrim window isn't in front. Garry's Mod runs at
  full speed in the background (start it with `tools/launch_gmod.ps1`). The view doesn't snap
  after menus. Mouse look: Skyrim shows the mouse movement before Garry's Mod has applied it,
  and turns its camera directly (Skyrim's own first-person camera trailed by up to 45 degrees
  during fast turns); the user reports only a very small delay left. Settings in Garry's Mod's
  console: `gmsky_sensitivity` (default 3), `gmsky_predict`, `gmsky_camera` (1 = on).

- **Smooth walking** (tested in game 2026-10-04, Phase 1 step 2): Skyrim blends between Garry's
  Mod's 66 updates a second instead of taking the newest one each frame; at steady speed the
  per-frame steps vary by 1–6% instead of 27–44%. Adds ~15 ms to the position only (not the view).
  `gmsky_smooth 0` turns it off.

- **gmsky loads by itself** (tested in game 2026-10-04): one small file,
  `garrysmod/lua/autorun/server/gmsky_autoload.lua`, installed once with
  `tools/install_gmod_autoload.ps1`, loads gmsky in single-player once you're in the map.

- **Garry's Mod walks on Skyrim's ground** (tested in game 2026-10-04, Phase 1 step 3c, test 16):
  Skyrim measures the ground around the player and Garry's Mod builds it as invisible collision,
  so Garry's Mod's own movement walks Skyrim's hills (on gm_flatmap). 65 ground updates in one
  session with no crash; each update builds only the 4 new pieces at the edge (~5 ms) and keeps
  the 12 it already has. Building a new piece under the moving player crashes Garry's Mod's
  physics, so pieces under the player are never rebuilt; `gmsky_reuse_pieces 0` turns that off
  and is only for testing.

## What doesn't work yet

- Sprinting fast downhill can drop the Garry's Mod player through the ground for a moment; it is
  put back on top within a second (seen once in test 16).
- Running off the edge of the built ground faster than Skyrim sends new ground would build a piece
  under the moving player, which is the case that crashes (not seen yet; not handled).

- The mouse wheel, weapons switching, and Garry's Mod's weapons/HUD in Skyrim's window.
- Camera position while walking is one frame late, and no crouch height or view bob (Phase 1 step 4).
- Jumping, crouching and Garry's Mod's own height: Skyrim's player stays on Skyrim's ground.
- Everything else in [docs/DESIGN.md](docs/DESIGN.md) §12.

## Requirements

- Skyrim Anniversary Edition **1.7.104**, SKSE64 for 1.7.104, Address Library for SKSE Plugins
- Garry's Mod, 64-bit (`gmod_win64.exe`)
- To build: Rust stable (MSVC), Visual Studio 2022 C++ tools, CMake

## Folders

| Folder | |
|---|---|
| `protocol/` | Shared-memory layout used by both sides |
| `skse/` | Skyrim SKSE plugin (C++) |
| `gmod/` | Garry's Mod module, loader stubs, Lua scripts |
| `tools/` | Test helpers and launch scripts |
| `docs/` | Design |
| `logs/` | Runtime logs (not tracked) |
| `build/` | Build output (not tracked) |
| `reference/` | Read-only SkyCraft clone (not tracked) |
