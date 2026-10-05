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

- **Garry's Mod walks on Skyrim's ground** (tested in game 2026-10-04, tests 20–21): Skyrim
  measures the ground around the player with rays, and Garry's Mod's own movement walks, jumps
  and slides on it. The ground is triangles answered from Lua (no Garry's Mod physics objects,
  which crashed Garry's Mod in tests 12–17). Test 21: 164 ground updates, no crash, no stalls;
  Skyrim measures ahead of a fast player, so noclip launches never outran the ground.

- **Jumping and noclip** (tested in game 2026-10-04, tests 17 and 21): Skyrim's player follows
  Garry's Mod's height; **V** toggles Garry's Mod's noclip from the Skyrim window; you fly
  through Skyrim's hills, and Skyrim's own physics doesn't push you back (contacts and Skyrim's
  "land rescue" are switched off for our player, as SkyCraft does).

- **Travel anywhere: no map edge** (tested in game 2026-10-04, test 22): when you get far from
  the middle of Garry's Mod's map, Garry's Mod moves you and its link to Skyrim's world back
  together, so Skyrim's position doesn't change. Test 22: 42 re-centres (37 while walking with
  `gmsky_recentre_at 500`, 5 in long noclip flights), 58,000 Garry's Mod units ≈ 109,000 Skyrim
  units from the start in one direction; position relative to the link identical before and
  after every time; speed kept; no holds; the user: "works flawlessly".

- **Skyrim's real collision** (tested in game 2026-10-04, test 23): Garry's Mod's movement
  collides with Skyrim's exact collision shapes: buildings, walls, stairs, arches, interiors,
  trees, fences, rocks. Streamed from Skyrim around you as you move (adapted from SkyCraft);
  `gmsky_ground grid` switches back to the older ray-grid ground.

- **Camera from Garry's Mod's eye** (tested in game 2026-10-04, test 24): crouching lowers the
  view smoothly, hard landings kick it (view punch), Skyrim uses Garry's Mod's FOV.
- **Crouch = Skyrim's sneak** (test 24): the stealth eye, NPC detection and sneak attacks follow
  Garry's Mod's crouch.
- `tools/launch_gmod.ps1` loads gm_flatmap directly (test 24), with `gm_flatmap.bsp` in
  `garrysmod/maps`.

- **Garry's Mod's hands, weapon and HUD inside Skyrim's window** (Phase 2, tested in game
  2026-10-05, tests 42-50): type `gmsky_overlay 1` in Garry's Mod's console.
  - Garry's Mod's hands, weapon, HUD (the patched HOLOHUD 2, see `tools/build_holohud.py`), menus
    and console are drawn over Skyrim's world every frame, 7-9 ms old, with correct transparency
    (blended in linear light; checked by snapshot, `gmsky_overlay_shot`).
  - Hidden while a Skyrim menu is open. The physgun's grab outline (halo) works (test 45).
  - HOLOHUD's plates are frosted: Skyrim is blurred behind them (`gmsky_overlay_blur 0/1`).
  - Skyrim's own first-person arms and weapon, crosshair, health / magicka / stamina bars,
    compass, charge meters and arrow count are hidden while the overlay is on; they come back
    when it's off.
  - Garry's Mod's window moves off-screen while the overlay is on and comes back afterwards
    (`gmsky_overlay_hidewindow 0` keeps it in place). Alt+Tab to it still works for its console.

## What doesn't work yet

- Quest markers and locations: Skyrim's compass is hidden while the overlay is on, and HOLOHUD's
  compass and minimap don't show Skyrim's markers yet (planned). No stamina / magicka display.
- Garry's Mod's window size isn't matched to Skyrim's automatically: use
  `tools\launch_gmod.ps1 -Width W -Height H` with Skyrim's size (both are 1280×720 so far).
- Ending noclip more than ~2,100 Garry's Mod units under Skyrim's ground holds you in place;
  press V again.
- Garry's Mod's physics props and ragdolls fall through Skyrim's ground (it is only answered to
  traces, not to Garry's Mod's physics engine).
- The mouse wheel and weapon switching from Skyrim's window (not tested).
- Everything else in [docs/DESIGN.md](docs/DESIGN.md) §12.

## Credits

Built on the approach and parts of the Skyrim plugin of [SkyCraft](https://github.com/chasmlol/SkyCraft)
by chasmlol (MIT). Details and license text: [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

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
