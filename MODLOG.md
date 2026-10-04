# MODLOG

## 2026-10-04 Recon and design plan

**Changed:** added `docs/DESIGN.md` (the plan) and this file. Cloned SkyCraft into `reference/SkyCraft` for reading only.
**Why:** plan before code, per AGENTS.md.
**Tested how:** read-only checks of both game folders.
**Result:** SkyrimSE.exe is 1.7.104; `skse64_1_7_104.dll`, `skse64_loader.exe` and `Data/SKSE/Plugins/versionlib-1-7-104-0.bin` are present. Garry's Mod is 64-bit with DirectX 9 (`shaderapidx9.dll`); `garrysmod/lua/bin` does not exist yet. Visual Studio 2022, CMake 4.4.3, Git, Python 3.12 present. Rust (cargo/rustc/rustup) is not installed. vcpkg not found.
**Still broken / not tested:** nothing built. Every "not tested" item in DESIGN.md §14–15 is still open.
**Next:** Phase 0 step 1 (whitelist `.gitignore`, README, folder layout), after the open questions in DESIGN.md §14 are answered.

## 2026-10-04 Permissions and Rust

**Changed:** `docs/DESIGN.md` §3 and §14: recorded that `Skyrim Special Edition/Data/SKSE/Plugins` may be written at any time, and that Rust is installed.
**Why:** user answered the open questions.
**Tested how:** ran `C:\Users\shiny\.cargo\bin\cargo.exe --version` and `rustup show`.
**Result:** `cargo 1.99.0`, toolchain `stable-x86_64-pc-windows-msvc` (default). `C:\Users\shiny\.cargo\bin` is in the user PATH. `cargo` failed in the user's already-open terminal only because that terminal started before the install; a new terminal fixes it.
**Still broken / not tested:** Visual Studio 2022 with CommonLibSSE-NG.
**Next:** Phase 0 step 1.

## 2026-10-04 Phase 0 step 1: repo setup

**Changed:** added `.gitignore` (whitelist) and `README.md`.
**Why:** AGENTS.md rule 2; keep `reference/`, `build/`, `logs/` out of git.
**Tested how:** `git status --untracked-files=all` and `git check-ignore -v` on a test file in each ignored folder.
**Result:** only `.gitignore` and `README.md` show as new. `reference/SkyCraft/README.md`, `build/x.dll`, `logs/a.log` are ignored by lines 39–41.
**Still broken / not tested:** nothing to run yet.
**Next:** step 2, the shared-memory protocol.

## 2026-10-04 Phase 0 step 2: protocol

**Changed:** `protocol/gmsky_protocol.h` (layout, seqlock helpers, coordinate conversion), `protocol/test/protocol_test.cpp` + `protocol/CMakeLists.txt`, `protocol/src/lib.rs` + `protocol/src/shm.rs` (Rust copy and shared-memory open), root `Cargo.toml` workspace. DESIGN.md §12 steps 2–3 wording.
**Why:** both games need one agreed memory layout.
**Tested how:** `cmake -S protocol -B build/protocol -G "Visual Studio 17 2022" -A x64`, `cmake --build build/protocol --config Release`, `build/protocol/Release/protocol_test.exe`; `cargo test -p gmsky-protocol -- --nocapture`.
**Result:** C++: `sizeof(GmskyShared)=248 player_state@144 failures=0`, PROTOCOL TEST PASSED. Rust: 3/3 passed; seqlock stress 7,014,349 writes / 2,246,359 reads, zero torn reads. Layout sizes/offsets asserted at compile time on both sides with the same numbers.
**Failure on the way:** first C++ build failed (C2678: can't assign a whole struct through a `volatile` pointer). Fixed by copying byte by byte.
**Still broken / not tested:** a C++ process and a Rust process have not shared the memory yet (step 5). Scale 72/128, the yaw formula and the pitch sign are unverified guesses.
**Next:** step 3, fake Skyrim.

## 2026-10-04 Phase 0 step 3: fake Skyrim and fake Garry's Mod

**Changed:** new crate `tools/fakes` with two programs, `fake_skyrim` and `fake_gmod`; added to the workspace.
**Why:** test each game's side without the other game running.
**Tested how:** `cargo build -p gmsky-fakes --release`, then `target/release/fake_skyrim.exe --seconds 4` with `target/release/fake_gmod.exe --seconds 3` running alongside. Read `logs/fake_skyrim.log` and `logs/fake_gmod.log`.
**Result:** fake_gmod saw origin epoch 1 and anchored at (200, 0, -12700). fake_skyrim logged "gmod CONNECTED", then positions moving round the circle (e.g. `pos=(191.9, 56.3, -12700.0)` → `(-56.3, 191.9, -12700.0)`), speed 125.7, 28–31 updates per 0.5 s (Windows sleep granularity makes 66/s into ~62/s; fine for a fake). fake_gmod saw Skyrim frames counting up.
**Still broken / not tested:** nothing game-side yet.
**Next:** step 4, the Garry's Mod module and loader stubs.

## 2026-10-04 Phase 0 step 4: Garry's Mod module, stubs, and the one-time install

**Changed:** new crates `gmod/module` (real module), `gmod/stub` (loader), `gmod/harness` (out-of-game test); Lua `gmod/lua/gmsky/server.lua` and `client.lua`; `tools/install_gmod_stubs.ps1`, `tools/launch_gmod.ps1`; `.cargo/config.toml` (static C runtime) and its `.gitignore` whitelist line. DESIGN.md §10 details.
**Written to the game (the one permitted lua/bin write):** `GarrysMod\garrysmod\lua\bin\gmsv_gmsky_win64.dll` and `gmcl_gmsky_win64.dll`, both sha256 `E491088988D4232AA368CD995467D604788C741A827E35F46D3920C251C21668` (= `target\release\gmsky_stub.dll`).
**Why:** Garry's Mod side of the link.
**Tested how:** `cargo build --release`, then `target\release\gmod_harness.exe`. The harness loads Garry's Mod's real `lua_shared.dll` from the install (read-only), loads the stub under both `require` names, runs 200 fake ticks and checks shared memory and logs. Also checked (read-only) that `lua_shared.dll` builds module names from `lua/bin/` + `gmsv_`/`gmcl_` + `_win64.dll`, and that the stub only depends on Windows system DLLs (dumpbin).
**Result:** HARNESS PASSED, 21/21 checks: anchor (101, 50, -12700) on first tick, final pos (300, 50, -12700), tick 200, epoch 1, flags 0x5, yaw 45, pitch 10, eye 64, fov 75; client counted 200 frames; server asked client to `require("gmsky")`.
**Failures on the way:**
1. Module didn't compile: a parameter named `root` hid the `root()` function. Renamed.
2. Before that build, the stub ran with no module present: it logged `ERROR: copy ... gmsky_module.dll ... cannot find the file` and returned cleanly, no crash. (Good: that's the in-game failure mode too.)
3. Harness crashed (access violation) on Lua `print`. Proved with a 3-line test that Garry's Mod's `print` reads a null pointer outside the game (it writes to the game console); `io` doesn't exist either. Harness now fakes `print`. Not a module bug.
**Still broken / not tested:** nothing tested inside the real game yet. Unknown: whether `+lua_run` on the command line runs after the map loads; whether `ply:SendLua('require("gmsky")')` loads the client side in game; real tick rate.
**Next:** user runs the in-game test; then step 5 (Skyrim plugin, read-only).

## 2026-10-04 Phase 0 step 4: in-game test, and eye-height fix

**Changed:** `gmod/lua/gmsky/server.lua`: eye height now `ply:EyePos().z - pos.z` (was `GetViewOffset().z`, the standing value). Harness fake gets `EyePos`. `tools/launch_gmod.ps1` no longer passes `+lua_run` and prints the console command instead. README "what works", DESIGN.md §6 and §10.
**Why:** results of the user's first in-game test.
**Tested how:** user ran `fake_skyrim.exe` and `tools/launch_gmod.ps1`, loaded gm_flatgrass, typed `lua_run require("gmsky")` in the console, walked, jumped and crouched, then closed the game. I read the logs (copied to `build/logs-ingame-test-1/`). After the fix: `cargo build --release` + `gmod_harness.exe` → HARNESS PASSED.
**Result:**
- stub.log: both real stubs loaded from `garrysmod\lua\bin` (server side 0, then client side 1), `gmsky_open returned 0` for both.
- gmod_server.log: `skyrim=CONNECTED`, `writes/s=66`–`67` every second; anchor (321.7, 342.6, -12288.0). gm_flatgrass floor is z = -12288.
- gmod_client.log: client side loaded via SendLua; 368 and 301 fps, then 20 fps (window unfocused).
- fake_skyrim.log: `gmod CONNECTED`; positions moving (e.g. (323.0, 342.4) → (884.4, -660.6)); speed up to 267.9; flags 0x5 (ground+alive), 0x4 (in air), 0x7 (crouched); `DISCONNECTED` 0.39 s after the game closed (gmsky_close at 36127031, detected at 36127421).
**Failures / findings:**
1. `+lua_run require('gmsky')` on the command line did nothing; the console command works.
2. Eye height stayed 64.0 while crouched (flags 0x7). Bug, fixed above. Not yet re-tested in game.
3. At 20 fps (game window not focused), fake_skyrim saw only 9–11 changes per 0.5 s although the server wrote 66/s: ticks run in bursts of ~3 per frame. Recorded in DESIGN.md §6; matters for Phase 1.
**Still broken / not tested:** eye-height fix in game. Skyrim side.
**Next:** step 5, Skyrim plugin (read-only): build CommonLibSSE-NG with Visual Studio 2022.

## 2026-10-04 Phase 0 step 5: Skyrim plugin, read-only probe (built, not tested in game)

**Changed:** new `skse/` (CMakeLists, CMakePresets for Visual Studio 2022, vcpkg.json, `cmake/deploy.cmake`, `src/main.cpp`, `src/Link.*` = shared memory, `src/Probe.*` = per-frame probe). CommonLibSSE-NG added as git submodule `skse/extern/CommonLibSSE-NG` (branch ng, 10.1.0); `.gitignore` whitelists that one submodule path (git refused it otherwise: a submodule is one entry, not a folder). vcpkg cloned to `.tools/vcpkg` (ignored). `gmod/lua/gmsky/server.lua` status line adds eye height and a facing-vs-moving check. DESIGN.md §14 license note.
**What the probe does:** hooks `PlayerCharacter::Update` (function-table slot 0xAD, same as SkyCraft). Every frame: Skyrim heartbeat + `SkyrimState`. On game load: publishes a new origin at the player's feet. Once a second, to `logs/skyrim.log`: Skyrim pos / heading / pitch / height / camera-minus-feet / moving direction, a scale suggestion, and Garry's Mod's latest state converted to Skyrim units. Changes nothing in the game.
**Tested how:** `cmake --preset default` (in `skse/`), `cmake --build --preset release`; dumpbin on `gmsky.dll`; `gmod_harness.exe` after the Lua change.
**Result:** configure 102 s (vcpkg built the dependencies), build exit 0, no warnings from our code. Visual Studio 2022 builds CommonLibSSE-NG fine (open question §14.3 answered). Build copied `gmsky.dll` + `gmsky.pdb` to `Skyrim Special Edition\Data\SKSE\Plugins`. Exports `SKSEPlugin_Load/Query/Version`. Harness passed; the new check printed `facing-minus-moving=45.0` for a fake player facing 45° and moving along +X (correct).
**Still broken / not tested:** nothing tested in Skyrim. Unknown until the in-game test: whether the plugin loads, whether the hook fires, Skyrim's real eye height, and whether my heading/pitch conventions are right.
**Next:** user runs the in-game test; then pin the scale/heading/pitch numbers in `gmsky_protocol.h`.

## 2026-10-04 Code review of steps 1–5, and fixes

**Changed:** review of `cdb7e98..b13b9ea` found 8 issues; 6 fixed, 2 skipped.
1. `skse/src/Probe.cpp`: origin epoch restarted at 1 on every Skyrim launch, so a still-running Garry's Mod (already on epoch 1) never re-anchored. Now continues from the epoch already in shared memory.
2. `skse/src/Probe.cpp`: Garry's Mod's position was converted with the newest origin even when its anchor belonged to an older epoch (or none). Now converted only when `origin_epoch` matches; otherwise logs "not anchored to the current origin yet".
3. Heartbeat came only from the server Tick, which stops while single-player is paused. New `gmsky.Heartbeat()`, called every client frame (`client.lua`).
4. A failed shared-memory open was never retried (Rust `OnceLock`; Skyrim plugin disabled itself). Both now retry every 2 s and log only when the error changes.
5. `protocol/src/shm.rs`: dropping a Mapping cleared its side's pid even if another process owned it. Now only clears its own pid.
6. Nothing enforced one writer per slot. New `owns_side()` (Rust) / `OwnsSide()` (C++): newest process wins, the older one logs once and pauses, and re-claims the side when the other exits. Used by the module, both fakes and the Skyrim plugin.
Skipped: header stuck at INIT if a process dies mid-initialization (tiny window; closing both games recovers); harness duplicates the module's Lua bindings (test-only code).
**Failure on the way:** my first version of the Skyrim retry reset its 2 s timer every frame, so it would never have retried. Caught on re-read before testing; fixed.
**Tested how:** `cargo test -p gmsky-protocol` (new test: ownership + pid clearing); `gmod_harness.exe` (new checks: takeover stops writes, writes resume and side re-claimed after the other exits, client Think alone keeps the heartbeat fresh); two `fake_gmod` at once; `cmake --build --preset release` for the plugin.
**Result:** protocol 4/4 passed; HARNESS PASSED 27/27; the older fake_gmod logged "another Garry's Mod-side process took over the link; pausing" and resumed after the newer exited; plugin built with no warnings from our code and was copied to `Data/SKSE/Plugins`.
**Still broken / not tested:** all Skyrim-side changes untested in Skyrim. Whether client `Think` really runs while Garry's Mod single-player is paused is **not tested** (I believe so, not sure).
**Next:** the step 5 in-game test (unchanged procedure).

## 2026-10-04 Phase 0 step 5: in-game test, measured scale, harness log fix

**Changed:** `protocol/gmsky_protocol.h`: scale `kGmodPerSkyrim` 72/128 → **64/120.5** (eye heights), comments record which conventions are verified; `protocol_test.cpp` updated. `gmod/module/src/lib.rs`: honours `GMSKY_LOG_DIR`; `gmod/harness` sets it to `build/harness/logs`. README "what works".
**Why:** results of the user's Skyrim test; and the harness had overwritten the user's in-game `gmod_server.log`.
**Tested how:** user ran Garry's Mod (`lua_run require("gmsky")`, walked, looked down), then Skyrim via `skse64_loader.exe` with an outdoor save (stood, walked straight, looked down). I read `logs/skyrim.log` (copied to `build/logs-ingame-test-2/`). That run used the plugin built before the review fixes (loaded 01:05:45). After the changes: protocol test, `gmod_harness.exe`, plugin rebuild.
**Result:**
- Plugin: `gmsky 0.1.0 loading (runtime 1-7-104-0)`, `shared memory opened (already created by Garry's Mod)`, `hooked PlayerCharacter::Update`, `published origin epoch 1 at (12155.9, -71991.3, 6010.5) worldspace 0000003C`. Garry's Mod read live every second (`GMOD alive=true`, tick 3851 → 6684). **The C++ plugin and the Rust module share memory correctly between the two real games.**
- **Skyrim heading convention verified:** walking straight, `facing-minus-moving` = 0.0, 0.1, 0.2, -0.1, -0.2 (heading 295.4, movement 295.1–295.6).
- **Pitch sign verified:** looking down gave Skyrim `pitch=90.0` and Garry's Mod `pitch=89.0`. Both positive = down.
- **Scale measured:** standing, Skyrim camera-minus-feet = 120.5 (every standing sample); height 131.8. Garry's Mod eye 64, so eye ratio 0.5311 (hull ratio would be 0.5461). Chose eye ratio. While walking the Skyrim camera reads 124–139 (head bob); standing value used.
- Garry's Mod yaw convention: this run's Garry's Mod walk was lost (see failure). Re-analysed the first test's saved `fake_skyrim.log`: on straight stretches movement and facing agree (-18.4/-18.6, -90.6/-90.9), which a mirrored convention would not give. Supported, not cleanly isolated.
- After changes: PROTOCOL TEST PASSED; HARNESS PASSED; `logs/gmod_server.log` timestamp unchanged by the harness run; plugin rebuilt and copied.
**Failure:** my harness run at 01:12 overwrote `logs/gmod_server.log` / `gmod_client.log` from the user's in-game session (the module always logged to `logs/`). The user's Garry's Mod facing-vs-moving data is gone. Fixed with `GMSKY_LOG_DIR`. The frozen stub still appends harness lines to `logs/stub.log` (append only, pid-tagged; can't change the stub).
**Still broken / not tested:** the review fixes (`6ced355`) and the new scale are not yet run in game.
**Next:** step 6, Skyrim's player follows Garry's Mod.

## 2026-10-04 Phase 0 step 6: puppet (built, not tested in game)

**Changed:** new `skse/src/Puppet.h` / `Puppet.cpp`; `skse/src/Probe.cpp` (location-change and "Skyrim moved the player" detection → new origin; calls the puppet every frame; logs it every second); DESIGN.md §12 step 6.
**What it does:** after Skyrim's own player update, every frame: converts Garry's Mod's position to Skyrim units; finds Skyrim's ground under that spot with a downward ray cast (`TES::Pick`, from 100 above to 600 below the current feet, using the player's collision group so it ignores the player), falling back to `TES::GetLandHeight`, then to the current height; `SetPosition`; heading and pitch from Garry's Mod; zeroes the character controller's velocity, fall start height and fall time (no momentum, no fall damage). Only while Garry's Mod's heartbeat is fresh, its anchor matches the current origin epoch, its player is alive, and Skyrim's player is loaded. Every ON/OFF switch is logged with the reason. A location change (worldspace for exteriors, cell for interiors), or Skyrim moving the player more than 300 units from where the puppet put it, publishes a new origin and pauses the puppet until Garry's Mod re-anchors.
**Logs (once a second, `logs/skyrim.log`):** `PUPPET on: sets=… ground ray/land/keep=… set=(…) heading=… pitch=… max-step=… max-drift=… big-steps=…`, or `PUPPET off: <reason>`.
**Tested how:** `cmake --build --preset release`; checked the deployed `gmsky.dll` is identical to the build and contains the puppet strings.
**Result:** builds with no warnings from our code; copied to `Data/SKSE/Plugins`.
**Still broken / not tested:** nothing tested in game. Not sure: whether the ray cast ignores the player's own capsule as intended (if not, `ground` would read ~player height above the ground and the player would climb each frame: watch `set` z rising while standing still); whether `bAlwaysActive=1` keeps Skyrim fully running in the background; how the first-person camera looks when the position is set every frame.
**Next:** user's in-game test.

## 2026-10-04 Phase 0 step 6: in-game test — Phase 0 done

**Changed:** `gmsky_protocol.h` comment (Garry's Mod yaw convention now verified), README. No code changes.
**Tested how:** user set `bAlwaysActive=1`, ran Garry's Mod + `lua_run require("gmsky")`, Skyrim via SKSE with an outdoor save, walked/turned/looked in Garry's Mod, then closed Garry's Mod. Logs copied to `build/logs-ingame-test-3/`.
**Result:**
- `PUPPET ON` 0.05 s after origin epoch 1 was published; `sets=` 50–61 per second (= Skyrim's frame rate); `ground ray/land/keep` = ray every frame (land/keep 0).
- Player walked ~2,000 units and climbed Skyrim's hills (z 6010 → 7290) by the ray-cast ground height. **Standing still, z stayed constant** (6361.4, 7290.3, 6010.1): the ray does not hit the player's own capsule.
- `max-drift` 0.0–0.5: Skyrim doesn't push back against the puppet. `max-step` 11–13 per frame walking, up to 48 on steep slopes.
- Closing Garry's Mod: `PUPPET OFF: Garry's Mod not connected (no heartbeat)` within ~1 s.
- **Garry's Mod yaw convention verified:** walking straight, `facing-minus-moving=0.0` in 11/11 samples; strafing ±90 (89.6, 90.0), diagonals ±45/±135 (42.6, -46.8, 134.6). All conventions now verified.
**Findings (not code bugs, cause not known yet):**
1. Skyrim paused ~17 s while Garry's Mod had focus (Garry's Mod logged `skyrim=not connected`; Skyrim frame 319 → 330). When it resumed the puppet made one 1,819-unit `big step` to catch up. Possibly `bAlwaysActive` not yet set at that moment — asked the user.
2. Garry's Mod froze ~9 s (9.3 s hole in its logs, client frames stopped). Skyrim correctly logged `PUPPET OFF` and took back control, then `PUPPET ON` when Garry's Mod returned. Cause unknown — asked the user.
**Next:** Phase 1 plan in DESIGN.md, then step 1.1 (input from the Skyrim window).

## 2026-10-04 Phase 1 plan, and step 1.1: input bridge (built, not tested in game)

**Changed:** DESIGN.md "Phase 1 steps". Protocol **version 2**: new `GmskyInput` slot (held actions as bits, running mouse/wheel totals) in `gmsky_protocol.h` and `protocol/src/lib.rs` (`GmskyShared` 248 → 296 bytes). New `skse/src/Input.h/.cpp`; `Puppet::IsActive()`; `Probe.cpp` and `main.cpp` call it. `gmod/module`: `gmsky.ReadInput()`. `server.lua`: `StartCommand` hook and `gmsky_sensitivity` setting (default 3). `client.lua`: `engine_no_focus_sleep 0`. Harness: fakes for the new Garry's Mod functions and 6 new checks.
**What it does:** while the puppet is on, Skyrim's window is in front and no Skyrim menu has the input, the Skyrim plugin listens to Skyrim's keyboard/mouse and (a) swallows them from Skyrim's own player (`PlayerControls` hook), (b) lets only Esc, ~, J, M, F9 reach Skyrim's menus (`MenuControls` hook), (c) E activates Skyrim's crosshair target, (d) publishes W/A/S/D, Space, Left Ctrl, Left Shift, Left Alt, R, left/right mouse as held actions plus mouse totals. Garry's Mod's `StartCommand` replaces the player's command with them: Garry's Mod buttons, forward/side move, and view angles from the mouse (counts × 0.022 × `gmsky_sensitivity`, pitch clamped ±89). Ignored unless Skyrim rewrote the slot within 250 ms. The mouse wheel is sent but not used yet.
**Tested how:** `cargo test -p gmsky-protocol`; C++ `protocol_test`; `cargo build --release`; `gmod_harness.exe`; plugin `cmake --build --preset release`.
**Result:** protocol 4/4 and PROTOCOL TEST PASSED (`sizeof(GmskyShared)=296`); HARNESS PASSED 33/33, including: forward+sprint → buttons 131080 (IN_FORWARD|IN_SPEED), forwardmove 10000; mouse +100/-50 counts → yaw 45→38.40, pitch 10→6.70; `active=0` and 300 ms stale input both leave the command untouched; `engine_no_focus_sleep` set to 0. Plugin built (no warnings from our code) and copied.
**Still broken / not tested:** nothing tested in game. Not sure: whether server-side `StartCommand` changes are fully honoured in single-player (Garry's Mod's wiki says it is the hook for this); whether `engine_no_focus_sleep` can be set from Lua (the log will say); whether Skyrim's raw mouse counts feel right at sensitivity 3. Both games must be restarted (protocol version 2 refuses to talk to version 1).
**Next:** user's in-game test.

## 2026-10-04 Phase 1 step 1.1: in-game test 4 (no code changes)

**Changed:** nothing in code. Logs saved to `build/logs-ingame-test-4/`. STATUS.md updated.
**Tested how:** user played from the Skyrim window: walked, sprinted, looked around, E on flowers/barrels/a wolf/Sven, opened and closed containers, alt-tabbed.
**Result (works):**
- `INPUT -> Garry's Mod` as soon as the puppet came on; back to Skyrim on every menu (`a Skyrim menu is open`) and on alt-tab (`Skyrim window not in front`), then back to Garry's Mod.
- **Server `StartCommand` changes are honoured in single-player:** `cmds/s=66–67`; W → `buttons=0x1 speed=200 facing-minus-moving=0.0`; Shift+W → `buttons=0x41 speed=400`. Mouse turns the view (e.g. 1534 counts → yaw -101.2).
- E: `activated 000A18F9 (Mountain Flower)`, `0001F228 (Barrel)` (opened the container menu), `00013491 (Sven)` (dialogue).
**Failures found:**
1. **Choppy mouse look.** `gmod_client.log`: `engine_no_focus_sleep is now (no such setting)` and the client runs at **exactly 20 fps** whenever Skyrim is in front (168–313 fps when Garry's Mod is in front). This 64-bit Garry's Mod has no `engine_no_focus_sleep`: its `engine.dll` contains `fps_max_nofocus` instead (looked for the string, not tried). So Garry's Mod handles 3 ticks at once, 20 times a second, and the view Skyrim copies changes only 20 times a second. The user also saw a console error about "sleep" when the client side loaded (exact text not yet captured).
2. **View snaps when leaving a container (or any menu).** Every `input from Skyrim: OFF` is followed by `yaw=0.0 pitch=0.0` (later `yaw=88.6`) within a second, in 12 of 12 cases. Cause: our hook sets the view on the server only. The Garry's Mod window's own view never changes, so as soon as we stop overriding (menu open), the player's view goes back to whatever the Garry's Mod window was looking at, and on `ON` we start again from that.
**Also:** user confirmed Garry's Mod is on the normal 64-bit (x86-64) branch, not a beta.
**Next:** fix 2 (keep our view while a Skyrim menu is open, and tell the Garry's Mod window about it), and fix 1 (`fps_max_nofocus`), one at a time.

## 2026-10-04 Step 1.1 fix A: no view snap when a Skyrim menu opens or closes (not tested in game)

**Changed:** `gmod/lua/gmsky/server.lua` (`StartCommand` hook); harness fakes `ply:SetEyeAngles`, `cmd:GetViewAngles`, `CurTime` and 8 new checks.
**What it does:** (1) while input comes from Skyrim, every tick also calls `ply:SetEyeAngles(view)`, so the Garry's Mod window's own view follows ours instead of staying where it was. (2) When input stops (menu, alt-tab), it keeps putting our last view into the command until the Garry's Mod window's own view is within 1 degree of it, or 0.5 s have passed, then lets go. Logs `input from Skyrim: OFF, holding yaw=… pitch=…` and `view handed to the Garry's Mod window after N held ticks (its view yaw=… pitch=…, X degrees from ours)`.
**Tested how:** `cargo build --release`; `target/release/gmod_harness.exe`.
**Result:** HARNESS PASSED, 41 checks: the window's view is set to 38.40/6.70 with ours; on OFF the command's view is held at 38.40 and buttons are left alone; hand-over after 2 held ticks once the window catches up (0.0 degrees); gives up after 0.5 s when it never does (6.6 degrees).
**Still broken / not tested:** not tested in game. Not sure that `SetEyeAngles` every tick reaches the Garry's Mod window in single-player (the hand-over line will tell: "after 0–2 held ticks, 0.x degrees" = works; "after ~33 held ticks, big degrees" = the window never caught up and it still snaps after 0.5 s).
**Next:** fix B (Garry's Mod held at 20 fps in the background).

## 2026-10-04 Step 1.1 fix B: Garry's Mod's 20 fps background cap (not tested in game)

**Changed:** `tools/launch_gmod.ps1` adds `+fps_max_nofocus 300`; `gmod/lua/gmsky/client.lua` no longer runs `engine_no_focus_sleep 0` (that setting does not exist in this 64-bit build and caused the console error) and logs `fps_max_nofocus=<value>` in its 5-second frame-rate line; harness checks updated.
**Why:** test 4: Garry's Mod ran at exactly 20 fps whenever Skyrim was in front, so the view Skyrim copies changed only 20 times a second (choppy mouse look). `engine.dll` contains `fps_max_nofocus` (and `fps_max`, `fps_max_menu`), found by searching the file for text; its meaning is assumed from the name.
**Tested how:** `cargo build --release`; `gmod_harness.exe`; PowerShell parse check of the launch script (0 errors).
**Result:** HARNESS PASSED, 41 checks: the client side runs no console commands; the log line shows `fps_max_nofocus=300` when set and `(no such setting)` when it doesn't exist.
**Still broken / not tested:** not tested in game. Not sure that `fps_max_nofocus` is what caps it at 20, or that `+fps_max_nofocus` on the command line takes effect (`+lua_run` didn't, but that runs before the map; settings normally do). If the log still shows 20 fps with `fps_max_nofocus=300`, the cap is something else.
**Next:** user's in-game test of fixes A and B. Later idea (user's): Garry's Mod's tick rate could be set to 60 (`-tickrate 60`) to match Skyrim's 60 fps; not done, one thing at a time.

## 2026-10-04 Step 1.1 in-game test 5, and step 1b: mouse look without the round trip (1b not tested in game)

**Test 5 result (fixes A and B, logs in `logs/`):** fix B works: Garry's Mod at ~300 fps in the background (was 20); `cmds/s` 66–67. Fix A works: every menu close logged `view handed to the Garry's Mod window after 0 held ticks (… 0.0 degrees from ours)`; user: no more snap. User: looking is smooth now but "floaty", it carries on briefly after the mouse stops.
**Cause (estimated, not measured):** mouse → Garry's Mod waits up to one tick (15 ms) → Skyrim waits for its next frame (~17 ms): Skyrim shows a view 1–3 frames old.
**Changed:** DESIGN.md Phase 1 step 1b. Protocol **version 3**: `GmskyPlayerState` gains `input_frame`, `mouse_x_applied`, `mouse_y_applied`, `deg_per_count` and flag `GMSKY_PF_INPUT_APPLIED` (96 → 128 bytes, `GmskyShared` 296 → 328), C++ and Rust. Module: `ReadInput` remembers the frame it applied; `WritePlayerState` reports it with the applied totals and a 14th argument, degrees per count. `server.lua`: `gmsky_predict` setting (default 1), passes `0.022 × gmsky_sensitivity` (0 when off). Skyrim `Puppet.cpp`: draws Garry's Mod's yaw/pitch plus the mouse movement not applied yet (same formula, pitch limited to ±89); `Input::MouseTotals()`. Logs once a second: `VIEW lag (Skyrim frames, 1 = newest) avg=… max=… | prediction on/off: frames=… ahead max=… deg, last frame … deg`.
**Tested how:** `cargo build --release`; `cargo test -p gmsky-protocol`; C++ `protocol_test`; `gmod_harness.exe`; `cmake --build --preset release` (plugin, copied to Skyrim).
**Result:** protocol 4/4; PROTOCOL TEST PASSED (`sizeof(GmskyShared)=328`); HARNESS PASSED 42/42, new check: after input, the player state reports `flags=0x15 frame=2 mouse=(1100, -50) k=0.066`. Plugin builds with no warnings from our code.
**Still broken / not tested:** not tested in game; the Skyrim prediction math has no out-of-game test (only the Garry's Mod side does). Not sure: whether Skyrim's camera uses the angle we set in the same frame, or one frame later (that delay would remain); whether Skyrim's own drawing adds delay we can't remove. Both games must be restarted (version 3 refuses version 2).
**Next:** user's in-game test, with `gmsky_predict 1` and then `0` to compare.

## 2026-10-04 In-game test 6 (step 1b), and step 1c: turn Skyrim's camera directly (1c not tested in game)

**Test 6 result (logs: `build/logs-ingame-test-5/`, the run after step 1b):** user: still delayed; with both windows overlaid, Garry's Mod turns first. The logs show prediction **did** work: `VIEW lag avg=1.84–1.90 max=2–3` Skyrim frames (Garry's Mod's own view is ~2 frames old, as estimated), and `ahead max` up to 22.51 degrees while flicking (Skyrim's angle ran ahead of Garry's Mod's by the movement not applied yet). So the delay that's left comes after we set the angle, inside Skyrim. 17 seconds were logged with `prediction off (gmsky_predict 0)`; the user felt no difference.
**User's question, raise Garry's Mod's tick rate:** not the fix. It would shorten only the Garry's Mod round trip (~2 frames), which prediction already hides for the view. It may still help movement (W/A/S/D) later.
**Cause (from SkyCraft's notes, `reference/SkyCraft/skse/src/Game.cpp`, not yet measured here):** Skyrim's camera update runs *before* the player update in each frame, so the angle we set is shown one frame later; and Skyrim's first-person camera eases its rotation (~50 ms).
**Changed:** DESIGN.md step 1c. New `skse/src/Camera.h/.cpp` (adapted from SkyCraft, MIT): hooks every `call PlayerCamera::Update` site (found by scanning the game's code; `RELOCATION_ID(49852, 50784)`); learns Skyrim's camera-root axis order from 240 frames; then, after Skyrim's camera update and again after the player update, sets the camera root rotation to our predicted view and pushes it down to the render camera. First person only, not while a Skyrim menu is open. `Puppet::ViewNow()` (newest predicted view; prediction math merged into one function). `Input::SkyrimMenuOpen()`. SKSE trampoline 64 → 256 bytes. Flag `GMSKY_PF_DRIVE_CAMERA` (0x20) from the new `gmsky_camera` setting (default 1) in `server.lua`; no layout change. Harness expects flags 0x25.
**Logs (once a second):** `CAMERA updates=… turned by us after camera update=… after player update=… | Skyrim's own camera off our view avg=… max=… deg`, plus `learning n/240` until learned, and once: `CAMERA axes learned from 240 frames: col0=… -> we turn the camera from now on` (or `INCONSISTENT`). `PlayerCamera::Update: hooked N call site(s)` at start.
**Tested how:** `cmake --build --preset release`; `cargo build --release`; `cargo test -p gmsky-protocol`; `protocol_test`; `gmod_harness.exe`.
**Result:** plugin builds with no warnings from our code, copied to Skyrim; protocol 4/4; PROTOCOL TEST PASSED; HARNESS PASSED 42/42.
**Still broken / not tested:** nothing tested in game. Not sure: how many call sites the scan finds on 1.7.104 (0 = the hook does nothing; the log says); whether learning completes (needs ~4 s of fairly still looking in first person); that setting the root's `local` rotation is right (SkyCraft does it; correct only if the root's parent has no rotation). The camera *position* is still one frame late while walking (Phase 1 step 4). Skyrim's own drawing delay (vsync, frames queued on the graphics card) is not touched.
**Failure noted:** my first PowerShell edit of `Probe.cpp` added a byte-order mark and CRLF line endings; reverted to the original ASCII/LF before review (code unchanged).
**Next:** user's in-game test, `gmsky_camera 1` then `0`.

## 2026-10-04 In-game test 7 (step 1c): Skyrim's camera turned directly — works

**Changed:** README "what works". Logs saved to `build/logs-ingame-test-6/`. No code changes.
**Tested how:** user looked around in first person, flicked the mouse with both windows overlaid, compared `gmsky_camera 1` and `0`.
**Result:**
- `PlayerCamera::Update: hooked 3 call site(s)`; `CAMERA updates=60–61` per second.
- `CAMERA axes learned from 240 frames: col0=+right (240) col1=+forward (240) col2=+up (240)`: all 240 votes agree for every column.
- Turned by us 60–61 times a second at both points (after camera update and after player update).
- **Skyrim's own camera off our view:** 0.00–0.02 degrees when still, avg 1.3–7.4 / **max 5.8–45.7 degrees** during turns. This is the delay we now skip (easing + frame order), measured.
- `VIEW lag avg=1.85–1.90 max=2` (Garry's Mod's view ~2 frames old, hidden by prediction).
- User: "a very very very tiny delay" left, maybe placebo; no longer floaty.
**Still broken / not tested:** what is left is not measured; likely Skyrim's own display pipeline (vsync, queued frames), which our code doesn't control. Camera position while walking still one frame late (step 4).
**Next:** Phase 1 step 2 (smooth position) or step 3 (collision), user's choice.

## 2026-10-04 Phase 1 step 2: smooth position (not tested in game)

**Changed:** DESIGN.md step 2 revised (SkyCraft's tick blending instead of sending the client's drawn position). Protocol **version 4**: `GmskyPlayerState` gains `tick_qpc` (QueryPerformanceCounter when the tick was written) and `tick_ms` (144 bytes; `GmskyShared` 344), flag `GMSKY_PF_SMOOTH` (0x40). `protocol/src/shm.rs`: `now_qpc()`. Module stamps every player state. `server.lua`: `gmsky_smooth` setting (default 1), passes `engine.TickInterval()`. New `skse/src/Smooth.h/.cpp` (adapted from SkyCraft, MIT): keeps the last 8 ticks, locks their times to the tick rhythm (tick numbers give how many ticks passed, stamp noise filtered 1/16), draws the position at "now minus a delay" blended between the two ticks around it; the delay follows the latest of the last 2 s of tick arrivals (+1 ms, 4–30 ms). `Puppet.cpp` uses it for the position only (the view keeps no added delay). Harness: flags 0x65, tick stamp and 15 ms tick length checked.
**Logs (once a second, `skyrim.log`):** `SMOOTH on: delay=… ms late frames=… new ticks=… stamp noise=… ms | steps while moving: N frames avg=… spread=…% max=… stalls=…` (or `OFF (gmsky_smooth 0)`). Spread = how uneven the per-frame steps are (standard deviation / average); stalls = frames the player didn't move while walking.
**Tested how:** `cargo build --release`; `cargo test -p gmsky-protocol`; `protocol_test`; `gmod_harness.exe`; `cmake --build --preset release`; checked the deployed DLL is the build and contains the new log text.
**Result:** protocol 4/4; PROTOCOL TEST PASSED (`sizeof(GmskyShared)=344`); HARNESS PASSED 43/43; plugin builds with no warnings from our code.
**Still broken / not tested:** not tested in game, and the blending code itself has no out-of-game test (it only runs inside Skyrim). Adds about 10 ms delay to the position (not the view), by design. Both games must be restarted (version 4).
**Next:** user's in-game test, walking with `gmsky_smooth 1` and `0`.

## 2026-10-04 In-game test 8 (step 2): smoothing delay too short — fixed (fix not tested in game)

**Test 8 result (logs: `build/logs-ingame-test-8/`):** user: felt about the same as without. Logs: the one second that ran with `delay=15.3 ms late frames=1` had `spread=1%` (perfectly even steps). Then the delay shrank to its 4 ms floor and stayed there with `late frames=39–49` of 60: most frames the next tick hadn't arrived and the player waited. Smoothing on: spread 13–45%; off: spread 27–58%, max step exactly 6.00/12.00 (one/two ticks). `new ticks` 54–61 per second, stamp noise 0.82–0.94 ms.
**Cause (my bug):** each of our ticks carries one position, so blending toward a tick starts one tick before its time. I copied SkyCraft's "how late did the tick arrive" measure, but SkyCraft's ticks carry a start and an end position, so its measure is one tick shorter than ours needs. The delay therefore aimed at ~1 ms (clamped to 4).
**Changed:** `Smooth.cpp`: a tick counts as needed from `t.at - period`; delay range 4 to 30 + tick ms; start value 20 ms. DESIGN.md wording (delay ≈ one tick plus lateness, ~17–25 ms expected).
**Tested how:** `cmake --build --preset release`.
**Result:** builds; **not copied to Skyrim: Skyrim was still running** (file locked). The `.pdb` was copied, the `.dll` wasn't; rebuild after Skyrim closes.
**Still broken / not tested:** not tested in game. Expected in the log: `delay` settling around 17–25 ms, `late frames` near 0, `spread` clearly below the "OFF" numbers.
**Next:** user's in-game test. (Update: rebuilt and copied after the user closed both games; deployed DLL identical to the build.)

## 2026-10-04 Load gmsky automatically from the launch script (not tested in game)

**Changed:** `tools/launch_gmod.ps1` only. After starting Garry's Mod it waits for the game window, then every 5 s starts `gmod_win64.exe -insecure -hijack +lua_run if not gmsky then require('gmsky') end` until `logs/gmod_server.log` (rewritten each session) contains `server.lua loaded`, up to 3 minutes. `-hijack` is a Source engine option that hands console commands to the copy already running (`launcher.dll` of this build contains the text `-hijack`; found by searching the file). Nothing is written to the game folder. `-NoAutoLoad` = old behaviour. Refuses to start if Garry's Mod is already running.
**Safety:** if a `-hijack` call hasn't exited after 10 s, or a new Garry's Mod process appears, it is treated as a second game starting: exactly the new processes are stopped and the script prints the manual command.
**Tested how:** PowerShell parse check (0 errors).
**Result:** parses. **Not run at all yet.**
**Not sure:** that `-hijack` still works in this 64-bit build (the text being in the file doesn't prove it); that `lua_run` receives the Lua unquoted as one line; whether starting the exe a second time goes through Steam. Alternative if it doesn't work: one small autorun file in `garrysmod/lua/autorun/server/` (needs the user's permission: game folder).
**Next:** user runs the script.

**Update (user's first run):** `-hijack` **works** in this 64-bit build: the running Garry's Mod received the command and printed `> if...` / `[ERROR] lua_run:1: unexpected symbol near '<eof>'`. Failure: `lua_run` gets only the first word after `+lua_run` from a command line, so the Lua arrived as just `if`. Fixed: the Lua is now one word, `return(gmsky)or(require('gmsky'))` (plain Lua; loads only once). Not yet re-run.

**Update 2 (user's second run):** the one-word Lua via `-hijack` gave a server error in Garry's Mod (exact text not captured). User decided: drop `-hijack`, use an autorun file. `-hijack` code removed from `launch_gmod.ps1`.

## 2026-10-04 Auto-load gmsky with an autorun file (user's decision; not tested in game)

**Changed:** new `gmod/autorun/gmsky_autoload.lua` (master copy) and `tools/install_gmod_autoload.ps1` (one-time copy, refuses if the file exists). **Installed** to `B:\SteamLibrary\steamapps\common\GarrysMod\garrysmod\lua\autorun\server\gmsky_autoload.lua` (sha256 `F1835A88…FB81C`), with the user's permission. `launch_gmod.ps1` back to just starting the game; says whether the auto-loader is installed. Harness: 4 checks on the auto-loader.
**What it does:** single-player only. Waits for the player's first real movement command (`SetupMove` with `not cmd:IsForced()`, the documented way to tell the player's game is fully loaded; `PlayerInitialSpawn` is earlier, and gmsky sends its client side with `SendLua`), removes its hook, then `require("gmsky")` once (next frame, outside the hook call). It never needs changing: gmsky's code stays in the project. Delete that file to stop auto-loading.
**Tested how:** `gmod_harness.exe` (real Garry's Mod Lua, fake game API); PowerShell parse check of the launch script.
**Result:** HARNESS PASSED 47/47: a forced command loads nothing; the first real command requires gmsky exactly once and removes the hook; outside single-player it adds no hook.
**Still broken / not tested:** not tested in game.
**Next:** user starts Garry's Mod with the launch script, without typing anything.

## 2026-10-04 In-game test 9: auto-load works; smooth position works

**Changed:** README, STATUS. Logs in `build/logs-ingame-test-9/`. No code changes.
**Result:**
- **Auto-load:** user typed nothing. `gmod_server.log`: `server.lua loaded`; `gmod_client.log`: `client.lua loaded` 16 ms later; no errors.
- **Smoothing on:** `delay` settled at 14.2–16.7 ms; `late frames` 0–6 per second; walking or sprinting at a steady speed, `spread` **1–6%** (e.g. avg 6.67 / max 6.86). Seconds with speed changes (starting, stopping, walk↔sprint) read 14–63%, as expected: the steps really change size then.
- **Smoothing off:** steady walking `spread` 27–44%, max step 12–18 (two or three ticks in one frame), `stalls` 0–2 per second.
- User: "I think I even noticed it with my own eyes."
**Not an issue, noted:** the delay settles ~15 ms, a little under one tick + lateness, so 0–6 frames a second blend slightly short; it doesn't show in the spread. If it ever does, raise the +1 ms margin in `Smooth.cpp`.
**Next:** commit; Phase 1 step 3 (collision).

## 2026-10-04 Phase 1 step 3 plan; step 3a: Garry's Mod-only collision test (not tested in game)

**Changed:** DESIGN.md: step 3 split into 3a–3d; §6 no longer mentions `engine_no_focus_sleep`. Module: `gmsky.Include(name)` runs `gmod/lua/gmsky/<name>.lua` from the project on that side (names: lowercase letters, digits, `_`). New `gmod/lua/gmsky/terrain.lua`, included by `server.lua` and `client.lua`: an invisible, frozen collision entity class `gmsky_terrain` (`PhysicsFromMesh`, `SOLID_VPHYSICS`, custom collisions, always sent to the client); the client builds its own copy for movement prediction. Console command `gmsky_test_terrain` (server): measures the map's room around the player (traces against the map only), builds a 65×65-point test hill (8,192 triangles, 25.5 units apart = Skyrim's planned grid, two waves up to ~35°, 0–400 units high) just above the floor under the player, and puts the player on top; `gmsky_test_terrain 0` removes it. On load the server also logs the room around the player. Harness: fakes for `Vector`, `util`, `game`, `scripted_ents`, `concommand`, `ents`, `LocalPlayer`; checks terrain.lua runs on both sides.
**Logs:** `terrain: map … room from (…): down … up … +x … -x … +y … -y …`; `terrain: built test N: 8192 triangles in X ms, PhysicsFromMesh=… physics object=…` (server and client); once a second on the server line and every 5 s on the client line: `terrain test N: over it/off it, feet minus hill …, standing on the hill/the map/nothing`.
**Tested how:** `cargo build --release`; `gmod_harness.exe`.
**Result:** HARNESS PASSED 49/49. The hill-building code only runs in the real game: not tested at all.
**Not sure:** whether the player collides with a `PhysicsFromMesh` entity in movement (the main question); whether the client builds its copy (needs its network data first); build time for 8,192 triangles.
**Next:** user's test, Garry's Mod only.

## 2026-10-04 In-game test 10 (step 3a): player walks on PhysicsFromMesh — works

**Result (logs: `build/logs-ingame-test-10/`):** user: "worked flawlessly", except being placed inside the hill (noclipped out) and falling off the invisible edges a few times. Logs:
- `built test 1–3: 8192 triangles in 18.5–20.4 ms` (server), 17.6 and 25.4 ms (client): `PhysicsFromMesh=true physics object=true` on both sides.
- On the hill: `standing on the hill` with feet 2–14 units above the formula surface (the square hull rests on its uphill corner on slopes; ~16 × slope, as expected). Off the edge: `standing on nothing`, falling.
- Room (gm_flatgrass): from (640, 384, −12246): down 42, up 27,606, +x 14,720, −x 16,000, +y 14,976, −y 15,744. User's guess of "about 20× the hill" is right (map ≈ 30,700 wide, hill 1,632).
**Failure:** player placed 4 units above the hill's centre point, but the hull is 32 wide, so its edges were inside the slope (stuck). **Fixed** (not re-tested): placed 4 above the highest point under the whole footprint.
**Changed:** `terrain.lua` placement; DESIGN.md 3c gets the measured room, anchor near (0, 0, 1,500), the placement rule, and "hold the player if there's no ground yet".
**Next:** commit; step 3b (Skyrim height grid).

## 2026-10-04 Phase 1 step 3b: Skyrim height grid (not tested in game)

**Changed:** Protocol **version 5**: new `GmskyTerrain` slot (65×65 heights, centre, spacing, origin epoch, misses; 16,952 bytes; `GmskyShared` 17,304 of the 65,536-byte map), C++ and Rust. New `skse/src/Terrain.h/.cpp`: while the puppet is on, casts downward rays (`TES::Pick`, player's collision group, LOS layer, from 4,000 above to 4,000 below the player's feet) on a 65×65 grid 48 Skyrim units apart centred on the player (snapped to the spacing), 1.5 ms of casting per frame (at least 16 rays); when complete, converts to Garry's Mod units (`SkyrimToGmod`, so conversion stays on the Skyrim side) and publishes. New grid when the player is > 768 units from the last centre or the origin epoch changes; reset on game load. Counts misses and rays that hit an actor (`FindCollidableRef` → `Actor`). Module: `gmsky.ReadTerrain(last_id)` (+ `lua_rawseti` binding). `terrain.lua`: server and client poll it every frame and log each new grid; the status line adds the player's distance from the grid centre. Harness: a grid written by the fake Skyrim reaches Lua (id, size, NaN kept).
**Logs:** `skyrim.log`: `TERRAIN published grid N at (x, y) epoch E: 4225 rays in X ms casting (Y us each) over F frames, Z ms from start; misses=… actor hits=…; ground z a..b`, and each second `TERRAIN this second: … | grid height under the player: G (feet F, grid minus feet D)`. `gmod_server.log`: `terrain: grid N from Skyrim …`.
**Tested how:** `cargo build --release`; `cargo test -p gmsky-protocol`; `protocol_test`; `gmod_harness.exe`; `cmake --build --preset release`.
**Result:** protocol 4/4; PROTOCOL TEST PASSED (`sizeof(GmskyShared)=17304`); HARNESS PASSED 51/51 (the harness caught `GridStatus` used before it was defined; moved). Plugin builds with no warnings from our code, copied.
**Not sure:** cost per ray (decides how long a grid takes; if slow, a sprinting player could outrun it); whether LOS-layer rays hit NPCs (the "actor hits" count will say); misses over water or outside loaded cells.
**Next:** user's test, both games.

## 2026-10-04 Garry's Mod map: gm_largeemptyplane (user's choice; not yet run)

**Changed:** `tools/launch_gmod.ps1` starts `gm_largeemptyplane` by default (`-Map <name>` to choose; `-Map gm_flatgrass` for the old one). DESIGN.md §6.
**Checked:** the user subscribed to it on the Workshop: item 1314012230 (`919169528587740911_legacy.bin`, read in memory: contains `maps/gm_largeemptyplane.bsp`). Nothing written.
**Not sure:** how much bigger it really is. Source maps are limited to about ±16,384 units, and gm_flatgrass already measured ~30,700 wide and ~27,600 tall; the room is logged on every load (`terrain: map gm_largeemptyplane room from …`), so the next run tells. The bigger gain may be no obstructions.
**Update:** `+map gm_largeemptyplane` failed at start-up: `CModelLoader::Map_IsValid: No such map 'maps/gm_largeemptyplane.bsp'` / `map load failed`. Likely cause (not confirmed): Workshop content is mounted only after the main menu loads, after command-line commands run. The launch script now stops at the main menu by default (the user picks the map there); `-Map <name>` still starts a built-in map directly.

## 2026-10-04 In-game test 11 (step 3b): Skyrim height grid works; gm_largeemptyplane is too low

**Result (logs: `build/logs-ingame-test-11/`):** user walked, sprinted up a mountain, passed NPCs; frame rate "felt similar".
- 57 grids published (`skyrim.log`) and all received (`gmod_server.log`). Each: 4,225 rays in 13.8–33.7 ms of casting (**3.3–8.0 µs per ray**) over 10–23 frames, **150–384 ms** from start to publish. **0 misses.** Skyrim frames per second: 60 in 76 of 103 seconds, 61 in 21, 54–58 in 6.
- Grid height under the player vs the puppet's feet: within 1 unit in 65 of 105 seconds, 1–5 in 19, 5–20 in 10.
- The large differences (+981 to +2,676 at 02:56:57–59) are the puppet's fault, not the grid's: sprinting up a ~55° mountainside (feet z 1,290 → 5,712 in 4 s), the Phase 0 ground ray (starts only 100 units above the feet) fell behind and the Skyrim player went *inside* the mountain. Step 3d (height from Garry's Mod's collision) replaces that ray.
- **Actor hits: up to 8 rays per grid** near NPCs/animals (0 away from them). These would become bumps in the ground. To fix.
- **gm_largeemptyplane room:** from (−13150, 7438, 106): down 42, **up 3,734**, +x 28,190, −x 2,018, +y 7,218, −y 22,478. As wide as gm_flatgrass (~30,000) but only **~3,776 units tall** (≈7,100 Skyrim units), against 27,600 (≈52,000) for gm_flatgrass. This walk alone covered Skyrim z −351 … 18,928.
