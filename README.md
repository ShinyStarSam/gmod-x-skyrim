# Gmod x Skyrim

Play Skyrim as a Garry's Mod player: Garry's Mod movement, weapons, health and HUD inside the real
Skyrim world. Both games run at the same time and talk through shared memory, the same approach as
[SkyCraft](https://github.com/chasmlol/SkyCraft). See [docs/DESIGN.md](docs/DESIGN.md).

> **Status: early.** Phase 0 (linking the two games) is in progress: Phase 0 done: Skyrim's player follows Garry's Mod. Phase 1 next.
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

## What doesn't work yet

- Playing from the Skyrim window (input is not forwarded yet; Phase 1).
- Jumping, crouching and Garry's Mod's own height: Skyrim's player stays on Skyrim's ground.
- Loading gmsky automatically when Garry's Mod starts (`+lua_run` doesn't work; type it in the console).
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
