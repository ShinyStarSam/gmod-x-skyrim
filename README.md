# Gmod x Skyrim

Play Skyrim as a Garry's Mod player: Garry's Mod movement, weapons, health and HUD inside the real
Skyrim world. Both games run at the same time and talk through shared memory, the same approach as
[SkyCraft](https://github.com/chasmlol/SkyCraft). See [docs/DESIGN.md](docs/DESIGN.md).

> **Status: early.** Phase 0 (linking the two games) is in progress: both games are linked; Skyrim doesn't follow Garry's Mod yet.
> Single-player and offline only. Garry's Mod is always started with `-insecure` (VAC off).

## What works

- **Garry's Mod → shared memory** (tested in game 2026-10-04): after `lua_run require("gmsky")`
  in the console, Garry's Mod's player position, aim, speed and on-ground/crouch/alive state reach
  shared memory 66 times a second, and a test program (`fake_skyrim`) reads them. Closing Garry's
  Mod is detected within half a second.

- **Skyrim plugin, read-only** (tested in game 2026-10-04): loads through SKSE on 1.7.104, hooks the
  player's per-frame update, and reads Garry's Mod's live state through shared memory. It logs
  both players side by side to `logs/skyrim.log`. It doesn't change anything in Skyrim yet.

## What doesn't work yet

- Skyrim's player following Garry's Mod (Phase 0 step 6).
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
