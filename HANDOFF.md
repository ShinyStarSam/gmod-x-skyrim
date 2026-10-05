# Handoff (2026-10-05)

Where things stand, for the next chat. Details: `STATUS.md` ("The current problem" is a running
log of every step), `MODLOG.md` (every change and test), `docs/DESIGN.md` §12 (the phases).

## Done

- **Phases 0-3 are done:** link, walking Skyrim with Garry's Mod movement, the overlay (Garry's
  Mod's hands, weapon and HUD in Skyrim's window, on at start-up), and combat. Combat covers
  hitting NPCs through Skyrim's hit function, NPCs hurting you, respawning at the last calm spot,
  and corpses pushed with Garry's Mod's force × 3.
- **Phase 4:**
  - **4a world audit (test 66) passed:** cities, interiors, fast travel, sleeping and a dungeon.
  - **Skyrim's menu keys:** T, Tab, I, P and F5 now reach Skyrim.
  - **4b swimming works (test 67):** this includes drowning damage reaching Garry's Mod.
- Protocol is **v15**. Last commit: see `git log -1`.

## Not tested yet: test 68

- **W + Space while swimming:** should swim forward and up together (`water.lua` now scales the
  keys like Source's `CheckParameters`).
- **`gmsky_overlay` on by default:** the overlay should appear without typing anything.
- **The new Skyrim menu keys:** Tab, I, P and F5 are untested. The user said T "worked just fine".

## Next steps (suggested order; ask the user)

1. **Run test 68**, then close Phase 4. Phase 4's §12 list (real shapes, interiors, load doors,
   floating origin) is otherwise met.
2. **"Found in test 60"** (DESIGN.md, in the 3d section). Garry's Mod's **world objects and
   effects are invisible** in Skyrim: grenades, rockets, AR2 balls, explosions, tracers and the
   physgun beam. Drawing them into the overlay hidden behind Skyrim's walls needs both games'
   depth; this is the hard part.
3. **Garry's Mod physics with Skyrim's world.** Today props, grenades and AR2 balls fall through,
   and the physgun can't grab corpses. This is also what real Garry's Mod ragdolls would need,
   which the user wants. Watch out: physics ground near the player crashed `vphysics.dll` in tests
   12-17.
4. **Phase 5 polish:**
   - auto-launch Garry's Mod from Skyrim;
   - save/load consistency;
   - Garry's Mod's window size matched to Skyrim's;
   - Skyrim potions don't heal Garry's Mod's player;
   - user's wishes: stamina / magicka bars, quest markers, locations and enemies in HOLOHUD.
5. Small known issues:
   - a bow hit (probably a bow bash) arrives as melee;
   - ending noclip more than ~2,100 units under the ground holds you.

## How to work here (read `AGENTS.md` first)

- **Planning:** plan in `docs/DESIGN.md` before code, one step at a time.
- **Logging:** after each change, add a `MODLOG.md` entry and give the exact test with what to
  expect. Commit only when asked.
- **Before handing over an in-game test:**
  - build the plugin: `cmake --build --preset release` in `skse/` (it copies `gmsky.dll` into
    Skyrim);
  - build the module: `cargo build --release` in `gmod/`;
  - run the harness: `cargo run --release -p gmsky-harness` in `gmod/`;
  - run the unit tests (`cargo test`).
  - Re-run the harness after *any* Lua default change (missed once, see MODLOG).
- **After each in-game test, save the logs yourself:** `tools\save_logs.ps1` writes
  `build\logs-ingame-test-N\`. `launch_gmod.ps1` also auto-saves before a launch, so the folder
  numbers can run ahead. Name the test after the folder that holds it.
- **Credits:** check `reference/SkyCraft` (MIT) before each step and credit what is adapted.
- **Bash quoting:** heredocs with quotes in them break in Bash here. Write Python edit scripts to
  the scratchpad with the Write tool instead.
