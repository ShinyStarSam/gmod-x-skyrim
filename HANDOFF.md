# Handoff (2026-10-05, after test 74)

Where things stand, for the next chat. Details: `STATUS.md` ("The current problem" starts with
this problem), `MODLOG.md` (every change and test, newest at the end), `docs/DESIGN.md` §12,
section "Garry's Mod's world objects and effects in Skyrim" (W1-W5).

## Done

- **Phases 0-4 are done:** link, walking Skyrim with Garry's Mod movement, the overlay (Garry's
  Mod's hands, weapon and HUD in Skyrim's window, on at start-up), combat (hits through Skyrim's
  hit function, NPCs hurting you, respawn, corpses pushed with Garry's Mod's force x 3), the world
  audit (test 66), Skyrim's menu keys, swimming (tests 67-68).
- Protocol is **v15**. Overlay block is **version 2** (128-byte slot header with the picture's
  camera; world layer + depth after the HUD picture). Last commit: `070c50a` (Phase 4).
- **Everything for the world layer (tests 70-74) is uncommitted** in the working tree: Lua
  `gmod/lua/gmsky/liveoverlay.lua`, module `gmod/module/src/capture.rs` + `lib.rs`, protocol
  `gmsky_protocol.h` + `overlay.rs`, Skyrim `skse/src/Draw.cpp`, harness, `tools/overlay_read.py`,
  docs. Commit only when the user asks.

## Current work: Garry's Mod's world objects and effects in Skyrim

Goal: props, rockets, grenades, AR2 balls, explosions, tracers and the physgun beam from Garry's
Mod show in Skyrim, **hidden behind Skyrim's geometry the way SkyCraft does it** (a depth test
against Skyrim's own depth buffer). The user rejected any cheaper approximation.

What works (tests 72-74): the world layer reaches Skyrim and is drawn; black boxes around effects
fixed (alpha writes off in the see-through pass); see-through props, RPG body and toolgun screen
fixed (alpha "stamp"); frame rate fine again (stamp recursion fixed); Skyrim re-projects the layer
to its own camera (matrix check correct) and its depth probe works.

**What does not work:**
1. **Depth is always empty, so objects always show on top.** Three Lua-drawn methods failed
   (STATUS.md lists them). **Do not try a fourth home-made one.**
2. **RPG: a pulsing black patch** at the barrel. It is the laser glow sprite the RPG's own code
   draws whenever its model is drawn; the stamp's extra `DrawModel` draws it as solid.
3. A little jitter around props when moving (Skyrim re-projects, but without depth it can't).
4. Effects without a model (tracers, explosions) will have no depth even after the fix (known limit).

## Next steps (in this order; one step at a time, ask the user before anything large)

1. **W5: the engine's depth pass** (plan in DESIGN.md, from the user's references):
   - Lua: `hook.Add("NeedsDepthPass", ..., function() return true end)` and, once at start, recreate
     `_rt_resolvedfullframedepth` as R32F (`IMAGE_FORMAT_R32F = 27`; exact call on the Garry's Mod
     wiki page `render.GetResolvedFullFrameDepth`). After the main view, grab
     `render.GetResolvedFullFrameDepth()` instead of our own target; send `render.GetViewSetup()`'s
     `zfar` along.
   - Module (`capture.rs`): accept D3DFMT_R32F (114) for depth, full size; distance = red x
     constant. **Not sure** whether the constant is the far plane (my disassembly) or 4000 (Valve
     wiki, TF2 branch): log the raw centre value next to the crosshair trace distance and decide
     from the numbers. Remove the fog path, the squares and the Lua depth target.
   - Check the user's references again first: meetric1/gmod_shader_guide (example 9),
     developer.valvesoftware.com/wiki/Depth_buffer ("Resolved Full Frame Depth"), GShader Library
     (Workshop 3542644649), EGSM wiki ("WorldPosDepthBuffer"), GM:NeedsDepthPass.
2. **RPG patch:** stamp the view model through a bone-merged `ClientsideModel` of the same model
   (no weapon code runs when it draws). Separate step.
3. Then: Garry's Mod physics with Skyrim's world (props, grenades, AR2 balls fall through; physgun
   can't grab corpses). Careful: physics ground near the player crashed `vphysics.dll` in tests 12-17.
4. Phase 5 polish: auto-launch Garry's Mod from Skyrim; save/load consistency; window size
   matched; Skyrim potions healing Garry's Mod's player; stamina / magicka bars, quest markers,
   locations and enemies in HOLOHUD.
5. Small known issues: a bow hit (probably a bow bash) arrives as melee; ending noclip more than
   ~2,100 units under the ground holds you.

## How to work here (read `AGENTS.md` first)

- **Planning:** plan in `docs/DESIGN.md` before code, one step at a time.
- **Logging:** after each change, add a `MODLOG.md` entry and give the exact test with what to
  expect. Commit only when asked.
- **Before handing over an in-game test:**
  - build the plugin: `cmake --build --preset release` in `skse/` (it copies `gmsky.dll` into
    Skyrim);
  - build the module: `cargo build --release` in `gmod/`;
  - run the harness: `cargo run --release -p gmsky-harness` in `gmod/` (186 checks today);
  - run the unit tests (`cargo test`).
  - Re-run the harness after *any* Lua change.
- **After each in-game test, save the logs yourself:** `tools\save_logs.ps1 -Test N` writes
  `build\logs-ingame-test-N\`. Next test is **75**.
- **Credits:** check `reference/SkyCraft` (MIT) before each step and credit what is adapted.
- **Bash quoting:** heredocs with quotes in them break in Bash here. Write Python edit scripts to
  the scratchpad with the Write tool instead.
