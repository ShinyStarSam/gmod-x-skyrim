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
  containers and talks to people in Skyrim; Esc, ~, J, M, F9, T, Tab, I, P and F5 (test 68) open
  Skyrim's menus, and input goes
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
  2026-10-05, tests 42-50; on by itself since test 68): on at every start (`gmsky_overlay 0` in Garry's Mod's console turns it off).
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

- **Weapons and the spawn menu from Skyrim's window** (Phase 3 step 3a, tested in game
  2026-10-05, tests 53-54): the mouse wheel and 1-6 open Garry's Mod's own weapon menu (HOLOHUD's)
  and a click picks; Q holds Sandbox's spawn menu open, with a cursor moved by the mouse (drawn in
  the overlay) and no camera movement while it's out (`gmsky_cursor_speed`). The physics gun's
  wheel pushes / pulls a held prop. Not done: typing in the spawn menu's search box, the C menu.

- **Shooting Skyrim's NPCs** (Phase 3, tested in game 2026-10-05, tests 56-60): every Garry's Mod
  weapon that fires bullets, the crowbar and the RPG hit Skyrim's NPCs through invisible stand-ins
  shaped like Skyrim's own per-bone hitboxes (the copy matches Skyrim's on 99.3-100% of test
  rays, `gmsky_hitbox_test`), and the hits go through Skyrim's own hit system: NPCs bleed, react,
  fight back and die. Damage × 3, head shots × 2 more (`gmsky_damage_scale`,
  `gmsky_headshot_scale`). You bump into NPCs like Garry's Mod's own; they shove you aside.
- **Skyrim's NPCs hurt you** (Phase 3, tested in game 2026-10-05, test 61): their hits take
  Garry's Mod's health (HOLOHUD's bar), 1:1 (`gmsky_hurt_scale`); Skyrim's own health stays full and
  Skyrim can't kill you. Falling hurts once (Garry's Mod's own fall damage).
- **Dying and respawning** (Phase 3, tested in game 2026-10-05, test 62): you die in Garry's Mod
  and click to respawn, all from Skyrim's window; you come back at full health at the last spot
  where nobody was fighting you. No Skyrim save reload.
- **Shooting corpses** (Phase 3, test 64): Skyrim's ragdolls are pushed with Garry's Mod's own
  damage force × 3 (`gmsky_ragdoll_force`), at the spot hit; explosions throw the limbs apart;
  killing shots and blasts throw the body as it falls.
- **Swimming** (Phase 4, tests 67-68; W + Space swims forward and up together): Skyrim's rivers and lakes are water in Garry's Mod too: from
  waist deep you swim with Source's own water movement (swim where you look, Space up, sink
  slowly, climb out at a bank); Skyrim shows you underwater, and drowning hurts you.
- **F1 opens Garry's Mod's console** (test 64) in Garry's Mod's own window over Skyrim's; close it
  with ~ or Esc (or click back into Skyrim).

- **Garry's Mod's objects inside Skyrim's world** (geometry route, tests 75-80): props, rockets,
  grenades, broken pieces, Garry's Mod NPCs with their animations and weapons are drawn by Skyrim
  itself in its own frame, with their textures; they stay put in the world and hide behind walls.
  Lit by Skyrim's own sun, ambient and lights at Skyrim's brightness, with normal maps, Source's
  phong shine and glowing (self-lit) parts; outdoors they get Skyrim's sun shadows and cast their
  own onto Skyrim (tests 81-89). Indoors, Skyrim's shadow-casting torches and fires shadow them too,
  using Skyrim's own shadow maps, and they cast shadows from those lights onto Skyrim and onto each
  other (tests 94-95).

## What doesn't work yet

- Garry's Mod's **effects** are invisible in Skyrim (tracers, explosions, the physgun beam, sprites); planned as step G7.
- A prop dragged with the physgun lags a little behind the view; phong values not fine-tuned (polish list).
- Grenades and AR2 balls fall through Skyrim's world and don't hurt NPCs; the physgun can't grab
  NPCs or corpses (Garry's Mod's physics doesn't see Skyrim's world or NPCs).
- Skyrim potions and healing don't heal Garry's Mod's player. Skyrim's ragdolls are still
  Skyrim's (stiffer than Garry's Mod's); Garry's Mod's own ragdolls need its physics to see
  Skyrim's world first.

- Quest markers and locations: Skyrim's compass is hidden while the overlay is on, and HOLOHUD's
  compass and minimap don't show Skyrim's markers yet (planned). No stamina / magicka display.
- Garry's Mod's window size isn't matched to Skyrim's automatically: use
  `tools\launch_gmod.ps1 -Width W -Height H` with Skyrim's size (both are 1280×720 so far).
- Ending noclip more than ~2,100 Garry's Mod units under Skyrim's ground holds you in place;
  press V again.
- The ground is now Garry's Mod physics (`gmsky_ground physics`, the default since test 124):
  props land on it (tests 118-123). `gmsky_ground shapes` switches back to the older traced
  ground (props fall through it).
- Stairs: invisible ramps are laid over Skyrim's staircases (found by Skyrim's stair materials),
  so the player walks up and down smoothly; the big stone stairs work (test 133). Not yet right:
  props, grenades and bullets hit the ramps too (they should be for the player only); rough spots
  on curved flights, stacked-box steps and steps just too steep to walk are fixed but **not
  tested in game** (checked offline on test 133's triangle dumps).
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
