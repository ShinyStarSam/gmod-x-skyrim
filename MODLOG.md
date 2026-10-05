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

## 2026-10-04 Step 3b fix: rays that hit an NPC or animal are cast again (not tested in game)

**Changed:** `Terrain.cpp`: when a ray hits an actor, cast again from 1 unit below the hit (inside the actor) down to the same bottom, up to 3 times; the log line now says `actor hits=N (still on an actor after re-casting: M)`.
**Tested how:** `cmake --build --preset release` (no warnings from our code; copied to Skyrim).
**Not sure:** that a Havok ray starting inside an actor's capsule passes out of it (it should: rays report entering a surface, not leaving one). `M` above 0 in the log would mean it doesn't.
**Next:** tested together with step 3c.

## 2026-10-04 Map: gm_flatmap (user's find)

**Measured** (user loaded it with gmsky running; `gmod_server.log`): from (−32, 32, −16215): down 73, up 32,535, +x 16,352, −x 16,288, +y 16,288, −y 16,352. So x and y −16,320…16,320 and z −16,288…~16,320: the engine's whole ±16,384 box (≈61,000 Skyrim units each way). Workshop item 3669476334. Chosen over gm_largeemptyplane (only ~3,776 tall) and gm_flatgrass (~27,600 tall, a building on the floor).
**Changed:** launch script's main-menu hint; DESIGN.md §6.

## 2026-10-04 Phase 1 step 3c: Garry's Mod walks on Skyrim's ground (not tested in game)

**Changed:** `terrain.lua`: grids cached by id on both sides (last 4); `T.GridHeight` (bilinear, nil off the grid or on a miss), `T.FootprintTop` (highest ground under the 32×32 hull, 3a's rule), `T.GridMesh` (8,192 triangles, squares touching a miss left out); the `gmsky_terrain` entity also builds from a Skyrim grid (`GridId`), the client as soon as it has that grid; `MeasureRoom` returns "home" = the middle of the map's open space. Server: `T.BeforeAnchor` moves the player home when Skyrim publishes a new origin, so the anchor is there; `T.UpdateGround` builds each new grid for the anchored origin (then removes the old entity), **holds** the player (SetupMove: no velocity, fixed origin) while there's no Skyrim ground under it and Skyrim is connected, releases it 4 above the highest ground under its footprint, and **lifts** it out if it is more than 12 units inside the ground. Status line: `ground: grid N (X ms to build), feet minus ground …, standing on Skyrim's ground/…, lifts, held ticks`. `server.lua` calls these every tick. Module: `gmsky.OriginEpoch()`. Harness: grid height lookup (centre point −11,978.88), 8,190 triangles with one missed corner, entity spawned for grid 7, held then released on top.
**Skyrim side unchanged** in 3c: Skyrim's player still takes its height from its own ground ray. The PUPPET line's `set=(…, z)` (ray) vs `gmod-as-skyrim z=` (Garry's Mod's height, converted) shows whether Garry's Mod now walks at Skyrim's ground height; that is 3c's test. 3d then switches Skyrim to Garry's Mod's height.
**Tested how:** `cargo build --release`; `gmod_harness.exe`.
**Result:** HARNESS PASSED 54/54.
**Not sure:** entity build time on real grids (3a: 18–25 ms per side; a ~20 ms hitch in Garry's Mod about once a second while moving); whether replacing the ground entity under a moving player causes a stumble; interiors (rays from 4,000 above hit ceilings and roofs: stage A limit, avoid for now).
**Next:** user's test on gm_flatmap.

## 2026-10-04 In-game test 12 (step 3c): Garry's Mod crashed building the second grid — fix attempt 1 (not tested in game)

**Result (logs: `build/logs-ingame-test-12/`):** user: Garry's Mod crashed when they tried to jump. Before that, 3c worked: `new origin epoch 1: player moved home (0, 0, 16)`, grid 1 arrived 344 ms later, `built Skyrim grid 1: 8192 triangles in 24.6 ms` (client 24.7 ms), `released at z 21.1`, then walking/sprinting from the Skyrim window `standing on Skyrim's ground` with feet 1.6–4.4 above it (Garry's Mod z rose 17.8 → 76.1 going uphill). Then `terrain: grid 2 from Skyrim …` on both sides, and nothing more: no `built Skyrim grid 2`.
**Crash dump** `GarrysMod/crashes/gmod_win64_20261004_031529_1_accessviolation.mdmp` (read with a small script in the scratchpad: exception, module list, stack words): `0xc0000005` **write** at `0x20ba15f5000` (a page boundary: writing past the end of a block) in **`vphysics.dll+0xaaff0`**; stack, top first: vphysics (98 frames) ← `server.dll+0x2eb909` ← `lua_shared.dll` ← … So our Lua called into Garry's Mod's physics, which crashed: building grid 2's `PhysicsFromMesh` (8,192 triangles in one entity). Grid 1 and three test hills of the same size built fine, so it depends on the data; why is unknown (inside Garry's Mod).
**Changed (attempt 1):** each grid is built as **16 pieces** (4×4 entities of 16×16 squares = 512 triangles), one per tick, nearest the player first; the old grid's pieces are removed once all 16 exist. Each piece is logged *before* `PhysicsFromMesh` (`terrain: building Skyrim grid N piece K: 512 triangles...`), so a repeat names the piece. The module records each grid the server receives to `logs/terrain/grid_<id>.bin` (raw `GmskyTerrain`, last 20 kept) for replay. A new origin removes all ground. Client status no longer mistakes ground pieces for the test hill. Harness: one piece = 512 triangles; 16 pieces built over 16 ticks, then grid 7 swapped in and the player released; `SysTime` fake.
**Tested how:** `cargo build --release`; `gmod_harness.exe`.
**Result:** HARNESS PASSED 56/56; `build/harness/logs/terrain/grid_7.bin` written (16,952 bytes).
**Not sure:** that smaller pieces avoid the crash. If it crashes again, the log's last `building … piece K` line and the recorded grid file point at the exact data; that would be attempt 2, and per AGENTS.md I stop and write STATUS.md if that fails too.

## 2026-10-04 In-game test 13: crashed again, on the first piece of grid 2 — cause narrowed down

**Result (logs and grids: `build/logs-ingame-test-13/`):** user: crashed "the moment I got close to a fence". Grid 1 built as 16 pieces (23.3 ms in all, slowest 2.6 ms), player released and walking on it (feet 4.3 above). Grid 2 arrived; the last line is `building Skyrim grid 2 piece 10: 512 triangles...`. Crash dump `gmod_win64_20261004_032047_1_accessviolation.mdmp`: the **same** place, `vphysics.dll+0xaaff0`, a write at a page boundary.
**The data rules out size and shape** (scratchpad `grid.py` on the recorded grids): grid 2 piece 10 is one of the gentlest pieces (largest step between neighbouring points 79 units, z 67–154). Grid 1 built fine with far harsher pieces (steps up to 1,409 units). Pieces of 512 triangles crash just like 8,192.
**What differs, every time:** grid 2's pieces are created while grid 1's pieces still exist in the same place (the same triangles, overlapping). Grid 1 is built with no ground around it; 3a's test hill was removed *before* the next one was built. Hypothesis: Garry's Mod's physics crashes when a new mesh object is inserted overlapping an existing mesh object (mesh-against-mesh collision). Fits all 5 builds without a crash and both crashes; not yet proven.
**Changed (fix attempt 2, not tested in game):** pieces never overlap. `Terrain.cpp`: grid centres snap to a 768-unit lattice (one piece = 16 squares × 48); a new grid starts when the lattice point nearest the player changes (so the player is at most ~384 units + travel from the centre; the grid reaches 1,536). `terrain.lua`: pieces are kept by their place on the lattice; each new grid builds only the pieces not yet there (nearest first, one per tick), then removes pieces outside it. A new origin removes everything. Log: `terrain: grid N is the ground now (X new pieces, Y kept, Z removed, … ms)`. Experiment command `gmsky_test_terrain overlap`: builds a second test hill exactly on top of the first without removing it (expect a crash if the hypothesis is right; logs `OVERLAP TEST: building…` / `survived`). Harness: a grid shifted by one piece builds only 4 new pieces (12 kept, 4 removed). HARNESS PASSED 58/58; plugin builds with no warnings from our code, copied.
**Per AGENTS.md:** this is the second real attempt at the crash; if it still crashes, stop and write STATUS.md.

## 2026-10-04 Experiment: overlapping meshes alone do NOT crash — hypothesis of test 13 wrong

**Tested how:** user, Garry's Mod only (gm_flatmap): `gmsky_test_terrain`, then `gmsky_test_terrain overlap`.
**Result:** no crash. `gmod_server.log`: `OVERLAP TEST: building test 2 on top of test 1...`, `built test 2: 8192 triangles in 21.8 ms`, `OVERLAP TEST: survived (hypothesis wrong)`. Caveat: the player was **not** touching the hill then (`off it … standing on the map`), so "overlap while the player stands on the old mesh" is still untested.
**Consequence:** two real attempts at the crash have not held up (smaller pieces: crashed again; overlap: disproved). Per AGENTS.md, stopped and wrote STATUS.md instead of trying more variations. Fix attempt 2 (non-overlapping pieces) stays built but untested in game.

## 2026-10-04 Crash investigation 1–4: the crash is a full 16-bit list inside the physics; replay tool (replay not tested in game)

**Changed:** nothing in how the ground is built. Investigation only:
- `gmod/module/src/lib.rs`: `gmsky.ReadGridFile(path)` reads a recorded `grid_<id>.bin` (project-relative path, no `..`) and returns the same values as `ReadTerrain`, or `false, error`.
- `gmod/lua/gmsky/terrain.lua`: console command `gmsky_replay` (`<id> [dir]`, `order near|far`, `piece <id> <k> here|away`, `repeat <id> <k> <count>`, `clear`); replay mode acts as if Skyrim were connected. Every `building …` log line now adds the build number and triangle total on that side, and whether the player is over that mesh (distance from its middle, on the ground or in the air). In replay mode each build also logs `built …` after it returns. Pieces can be built farthest-first (`T.buildOrder`).
- Harness: fake `concommand` keeps its function, fake `hook.Remove`, velocity as a Vector; 3 new checks.
**Why:** STATUS.md troubleshooting steps 1–4.
**Step 1 result (crash dumps, read-only, scratchpad `stackcheck.py`, `regs.py`, `stackwords.py`; vphysics.dll disassembled with `dumpbin`):**
- **Not a stack overflow.** The crashing thread (the main thread) used ~36 KB of its 1 MB stack; the write target (`0x20ba15f5000`, `0x25b8f017000`) is a heap address, not the stack.
- The real call chain is 13 vphysics frames under one call from `server.dll+0x2eb909` (our `PhysicsFromMesh`); the "98 vphysics frames" in STATUS.md were stale values further up the stack, not real calls. Nothing in the chain is about the player or other objects; it is all inside the mesh-to-collision conversion (not certain without symbols).
- The crashing function (`vphysics.dll+0xaaed0`) adds an item to a sorted list (float key + pointer, 24 bytes per item) that numbers its items with **16-bit numbers**. When full it doubles, capped at 65,532 items. Registers at the crash, identical in both dumps: `rbp=0xfffd`, `r12=0xffff`, `r13=0xfffe`, `rcx=0x2fff7` (= 3 × 0xfffd). So the list was already at its cap, could not grow, and the item was written just past the end of its memory block: the page-boundary write. **Something put ~65,533 items in this list while converting a 512-triangle piece**, or the list keeps items from earlier builds.
- Recorded grids (scratchpad `flat.py`): no misses, NaN or infinities; grid 2 piece 10 is unremarkable (6 of 256 squares perfectly flat, 216 distinct heights; grid 1's pieces have 0–61 flat squares and 159–289 distinct heights). Nothing in the data explains it yet.
**Tested how:** `cargo build --release`; `gmod_harness.exe`.
**Result:** HARNESS PASSED 61/61. The test 13 grid files are 16,952 bytes, the size the reader expects.
**Not tested:** `gmsky_replay` in the real game; whether test 13's crash repeats with the replay.
**Next:** the user runs the replay tests (STATUS.md).

## 2026-10-04 Replay test (user, one session, several tests in a row): crashed — same crash, piece under the player

**Result (logs still in `logs/`, not yet copied; dump `gmod_win64_20261004_034822_1_accessviolation.mdmp`):** same crash, same registers (`0xfffd/0xfffe/0xffff`), same call chain. In the session:
- `replay 1` (16 pieces, player held over 4 of them, 40 above): OK. `replay 2` (7 new pieces, one with the player OVER it, feet exactly on its surface): OK.
- **Test C passed:** `piece 2 10 away` (test 13's crash piece, nobody near it): built, "extra copy survived". So that piece's data alone does not crash.
- `order far`, `replay 1`, `replay 2`: crashed on **grid 2 piece 15, the piece under the player** (10 units from its middle; the 7 pieces before it built fine; 38th build on the server). Player held in the air at z 58.0; grid 2 ground under its footprint 56.0–58.0: feet on the new surface (0.0). The no-crash piece 5 above had the same (0.0). So "player on the new mesh" happens in both the crash and a non-crash; not the whole answer yet.
- **Replay is not exact for test 13:** test 13's grids are not on the 768-unit lattice (recorded before fix attempt 2), and `PieceKey` rounding treated 9 of grid 2's pieces as "kept" from grid 1 although they are in a different place. Grid 2 piece 10 was never rebuilt in place. Fix attempt 2's reuse must only match pieces that line up exactly (not rounding). Not fixed yet.
- Build count is not the trigger alone: a crash on the 38th server build here, on the 17th (first of grid 2) in test 13.
**Next:** see STATUS.md.

## 2026-10-04 Fix: piece reuse only for pieces exactly on the lattice (replay now matches test 13)

**Changed:** `terrain.lua` `PieceKey`: a piece more than 1 unit off the lattice gets a key of its own grid (`g<id>:ci:cj`) instead of being rounded onto a lattice place, so it is never "kept" in place of a piece somewhere else. Harness: grid 9, half a piece off the lattice, builds all 16 pieces (0 kept, 16 removed).
**Why:** the replay showed test 13's grids (recorded before Skyrim snapped grid centres) being rounded: 9 of grid 2's pieces were treated as already built.
**Tested how:** `cargo build --release`; `gmod_harness.exe`. **Result:** HARNESS PASSED 63/63.
**Also:** the replay session's logs and dump saved to `build/logs-ingame-test-14/`.
**Not tested in game.**

## 2026-10-04 Replay tests A, B, E (user): no crash. The crash is in "collision rules changed", not PhysicsFromMesh

**Result (logs: `TEMP LOGS TEST A/B/E/`, each a fresh Garry's Mod):**
- **A** (`replay 1`, walk, `replay 2`, now with the lattice fix): grid 2 built in full (16 new, 16 removed, 18.9 ms), player standing still on the ground over pieces 4 and 8. No crash. Test 13's player was sprinting (speed 400) over piece 10's corner.
- **B** (`piece 1 10 here`, player standing on it): survived.
- **E** (`repeat 1 10 60`): all 60 copies built, 76 builds and ~39,000 triangles per side; survived. **Builds piling up is ruled out** (test 13 crashed on the 17th build).
**Crash call site read (server.dll, read-only, dumpbin):** the return address `server.dll+0x2eb909` is in the function at `0x2eb870`, which checks the entity's physics object, warns "Changing collision rules within a callback is likely to cause crashes!" if inside a physics callback, gets up to 1,024 physics objects, and calls one method on each. That is Source's `CBaseEntity::CollisionRulesChanged()` calling `RecheckCollisionFilter()` (the method slot matches the SDK's order; the warning text confirms the function). So the crash is not inside `PhysicsFromMesh`, but in a call after it that changes collision rules: `SetSolid` or `EnableCustomCollisions` (both normally do). The physics then recomputes the new object's collision pairs, which involve the player's physics body when it is on the spot; the overflowing 16-bit list is in that.
**Changed:** in replay mode, `BuildCollision` logs a line before `SetSolid`, `SetMoveType`, `EnableCustomCollisions` and the rest, so a crash names the call. `gmsky_replay hold <x> <y> <z>` / `hold off` puts and holds the player at an exact spot (to recreate test 14's crash position).
**Tested how:** `gmod_harness.exe`: HARNESS PASSED 63/63 (the harness does not run `BuildCollision`). **Not tested in game.**
**Next:** recreate test 14's crash exactly; if it repeats, try creating the entity solid and with its flags set *before* `PhysicsFromMesh`, so no collision-rules re-check happens on a live mesh.

## 2026-10-04 Crash fix candidate: make the ground solid before its mesh exists (not tested in game)

**Changed:** `terrain.lua` `BuildCollision`: with `gmsky_solid_first 1` (new default, saved, replicated to the client side) `SetSolid`, `SetMoveType` and `EnableCustomCollisions` run *before* `PhysicsFromMesh`, so Garry's Mod's collision-rules re-check never runs on a live mesh; `0` = the old order. A log line before each call (always, for now), and `built …` is always logged with the final `solid=` and `flags=` (expect solid=6). `gmsky_reuse_pieces 0` (not saved, back to 1 on every start) rebuilds all 16 pieces of every grid, as in tests 12/13, so fix attempt 2's reuse can't hide the crash. Harness: fake `FCVAR_REPLICATED`.
**Why:** the crash is in `CollisionRulesChanged` → `RecheckCollisionFilter` (previous entry). User prefers testing with Skyrim running (tests 12/13 crashed at grid 2, 2 of 2).
**Tested how:** `gmod_harness.exe`: HARNESS PASSED 63/63 (does not run `BuildCollision`). **Not tested in game.**
**Not sure:** that `PhysicsFromMesh` keeps the solid type and flags set before it (the `built` line's `solid=`/`flags=` and "standing on Skyrim's ground" will show it).

## 2026-10-04 In-game test 15 (Skyrim + Garry's Mod, `gmsky_reuse_pieces 0`, solid first): crashed the same way — solid-first reverted

**Result (logs, grids and dump: `build/logs-ingame-test-15/`):** grid 1 built and walked on; grid 2 (one piece west, on the lattice) arrived; first piece built = piece 11, player OVER it (72 units from its middle), on the ground, walking (speed ~208). Last log line: `Skyrim grid 2 piece 11: PhysicsFromMesh...`. Dump `gmod_win64_20261004_170725_1_accessviolation.mdmp`: same instruction, same full 16-bit list (`rbp=0xfffd`, `r12=0xffff`, `r13=0xfffe`). The inner chain (`0xa0728` … `0x97c9b` → list add) is the same as before; it is now reached from object creation inside `PhysicsFromMesh` (`server.dll` → `vphysics.dll+0x1b687` … `+0x9736c`) instead of from `CollisionRulesChanged` (`+0x98cfc`). So the overflowing list belongs to the physics step that works out the new object's collision pairs with what touches it, which both paths run. Making the entity solid first only moved the crash.
**Changed:** solid-first and `gmsky_solid_first` removed (back to the original order); the step log lines stay. `gmsky_reuse_pieces` stays (default 1, not saved).
**Pattern over all runs** (from logs): a piece built under the player crashed 4 times (tests 12, 13, 15: player walking/sprinting/jumping on the old ground; replay test 14: player held after noclip). It did not crash with the player held still at "home" (grid 1, every run), standing still (replay A, B), or held still (replay 14 piece 5). Pieces away from the player never crashed (hundreds). Leading explanation, **not proven**: the crash needs a new ground mesh created where the player's physics body is touching it while that body is moving.
**Tested how:** `gmod_harness.exe`: HARNESS PASSED 63/63.

## 2026-10-04 In-game test 16: piece reuse on (fix attempt 2) — no crash in 65 grids; cleanup

**Result (logs: `build/logs-ingame-test-16/`):** Skyrim + Garry's Mod, defaults (`gmsky_reuse_pieces 1`). User walked/sprinted ~2 minutes: grids 1–65, no crash. Grid 1: 16 pieces (22.7 ms); every later grid: `4 new pieces, 12 kept, 4 removed`, 4.6–6.7 ms, 44–81 ms from arrival. 272 builds on each side (client built all of them too). The only piece built with the player over it was grid 1's, with the player held at home (as in every run). Every piece reported `solid=6 flags=3`. One `LIFT: player was 125.8 below the ground` at (−1923, −825) after falling at speed ~493 (sprinting downhill, input in the other window): put back on top, then walking normally.
**Client error:** the user saw a client-side Lua error before closing. Our client status timer stopped logging at ~1,907 s while the game ran to ~1,932 s and Skyrim was still connected, so it was probably that timer failing (Garry's Mod deletes a timer whose function errors), not Skyrim closing. Cause not found in the code. **Changed:** the client status timer catches errors and logs `ERROR in client status: …` (and keeps running).
**Changed (cleanup):** the per-call debug lines in `BuildCollision` removed; `built …` is logged only on failure, for test hills and in replay mode (it now includes `solid=`/`flags=`). README: step 3c works; known gaps listed.
**Tested how:** `gmod_harness.exe`: HARNESS PASSED 63/63.
**Not handled:** running off the built ground faster than new grids arrive would build a piece under a moving player (the crash case). Not seen in test 16.

## 2026-10-04 Phase 1 step 3d: Skyrim's player takes its height from Garry's Mod (not tested in game)

**Changed:** `skse/src/Puppet.cpp`: the player's z is Garry's Mod's (blended with x/y, step 2) instead of Skyrim's own ground ray. The ray stays only for the log: once a second `PUPPET height (step 3d): … Garry's Mod feet minus Skyrim's ground min=… max=…`. The `PUPPET on:` line now gives Skyrim's own drift in xy and z separately, and how many frames Garry's Mod was in noclip. `docs/DESIGN.md`: 3d and the new 3e (noclip) planned.
**Why:** next planned step; jumping and noclip can't show in Skyrim without it; test 11 (sprinting up a steep slope put Skyrim's player inside the mountain) came from mixing the two games' heights.
**Tested how:** `cmake --build --preset release` (compiles, copied to Skyrim's Plugins folder).
**Not tested in game.** Expected: walking, "feet minus ground" within a few units of 0; in a jump, max goes up (~100 Skyrim units).
**Next:** step 3e, noclip.

## 2026-10-04 Phase 1 step 3e: noclip on V, and the crash guard for building ground under the player (not tested in game)

**Changed:**
- Protocol version 6: `GmskyInput.reserved0` is now `noclip_presses`, a running count of V presses (`protocol/gmsky_protocol.h`, `protocol/src/lib.rs`; offset 36 asserted on both sides).
- `skse/src/Input.cpp`: V (scan code 0x2F) in Skyrim's window adds 1 and logs `V: noclip toggle #N sent to Garry's Mod`; it never reaches Skyrim.
- `gmod/module/src/lib.rs`: `gmsky.ReadInput()` returns a 6th value, the new presses since the last tick (presses made while input went to Skyrim are not counted; a jump of more than 8 counts as a restart and is ignored).
- `gmod/lua/gmsky/server.lua`: each new press does what Garry's Mod's own `noclip` command does: asks the `PlayerNoClip` hook, then switches the move type. Logs `noclip ON/OFF (V in Skyrim) at (…): done` (or "refused"). The 1-second status line shows `NOCLIP`.
- `gmod/lua/gmsky/terrain.lua`, **crash guard**: a ground piece whose square (+48 units) contains the player is never built while the player moves. In noclip it is left in the queue (`GUARD: … under the player in noclip: left for later`) and built once the player isn't over it or noclip ends. Walking, the player is held still with its feet 40 above the ground near it for 3 ticks, then the piece is built and the player let go (`GUARD: … held still at z …`, `GUARD: released …`). This also covers the old "outrunning the grid" gap. Noclip ends any hold. All holds go through one `T.Hold` with a reason (origin / no ground / guard / replay), shown in the status line. A piece built from a grid with misses is rebuilt when a newer grid has fewer misses there (`… replaces a piece with more misses`).
- `skse/src/Terrain.cpp`: rays reach 20,000 units below the player's feet (was 4,000), and a new grid is cast when the player's height moved more than 2,000 units from the last grid's (`TERRAIN player height … casting a new grid`), so a player flying high in noclip still gets ground.
- `skse/src/Puppet.cpp`: logs `PUPPET Garry's Mod noclip ON/OFF` and Skyrim's own drift (xy and z) every second, to see whether Skyrim's physics pushes the flying player out of rocks.
**Why:** user request: noclip (V) must work. Noclip flies much faster than walking, so without the guard it would build ground under the moving player, which crashes Garry's Mod (STATUS.md "Known engine bug").
**Tested how:** `cargo build --release`; `cargo test --release`; `gmod_harness.exe` → HARNESS PASSED, 75/75 checks (new: V on/off/two presses in one tick through the `PlayerNoClip` hook; guard hold 40 above the ground before building under the player for grids 7 and 9; grid 10's piece under a noclip player left for later, then built with a guard hold after noclip ends; grid 11 rebuilds only the piece that had a miss). `cmake --build build/protocol` → PROTOCOL TEST PASSED. Skyrim plugin built and copied.
**Result:** all automated checks pass. The harness does not run Garry's Mod's physics or movement.
**Still broken / not tested:** nothing here has run in game. Not known: whether Skyrim's own physics fights a player flying through rocks (watch `max-drift` in noclip); whether a guard hold raising the player 40 units is noticeable; how much the longer rays cost (the per-frame budget caps it; the grid may take more frames). Known limit: ending noclip more than ~2,100 Garry's Mod units under the ground holds the player in place; press V again.
**Next:** in-game test 17 (procedure in STATUS.md).

## 2026-10-04 Phase 1 step 3f: Skyrim's player body handled as SkyCraft does (not tested in game)

**Why:** user reminder before test 17: always check SkyCraft's implementation first. Reading SkyCraft's puppet (`Game.cpp`) and `DigPhysics.cpp` showed three things our 3d/3e missed:
1. SkyCraft tells Skyrim's character controller "on the ground" (with support) or "in the air" from the other game's on-ground flag, and keeps Skyrim's gravity on: Skyrim's AI won't melee a target it thinks is in the air (SkyCraft had tried zero gravity and undid it).
2. Skyrim's character body pushes itself out of whatever it touches; SkyCraft drops the puppet body's contacts above its feet so Skyrim can't shove it away from where the other game put it (and then "pull the other game along").
3. Skyrim lifts any actor more than 30 units under the land back on top ("land rescue"). For a noclip player under the ground that lift would also be bigger than our 300-unit "Skyrim moved the player" threshold, which resets the origin and teleports the Garry's Mod player home. Our own plan only said "measure it"; this would have failed in test 17.
**Changed:** new `skse/src/Body.cpp/.h` (adapted from SkyCraft, MIT): contact hook (`bhkCharacterPointCollector::AddCdPoint`) drops our body's contacts above its feet (+0.3 Havok units), all of them in noclip; land-rescue hook at the call SkyCraft found on 1.7.104 (`REL::ID(37507) + 0xCEA`, patched only if a 5-byte call is there; else a warning in the log) skips the rescue for an actor within 64 units of our player while the puppet is on. `Puppet.cpp`: on-ground/in-air controller state from `GMSKY_PF_ON_GROUND` (in air while in noclip), gravity untouched; passes the body to `Body`; `Body::Release` when the puppet goes off. `main.cpp`: installs the hooks; trampoline 256 → 512 bytes. Log: `BODY puppet: contacts dropped above the feet=… in noclip=…, land rescues skipped=…` every second. New `THIRD-PARTY-NOTICES.md` with SkyCraft's MIT license and the list of files adapted from it; README credits section.
**Tested how:** `cmake --build --preset release` (compiles, links, copied to Skyrim). No Garry's Mod changes; harness unchanged (75/75 earlier).
**Not tested in game.** Not sure: that the land-rescue spot holds for our exact game build (same 1.7.104 as SkyCraft; the plugin checks the byte before patching and logs either way).
**Next:** in-game test 17 (procedure in STATUS.md).

## 2026-10-04 In-game test 17 (steps 3d, 3e, 3f): noclip works, then Garry's Mod crashed (same engine bug)

**Result (logs, grids, dump: `build/logs-ingame-test-17/`):** ~2 minutes, grids 1–87, 404 piece builds on the server. User: "great around regular terrain until CTD", four problems:
1. **Crash** (Garry's Mod, 17:44:27): same signature as tests 12–15 (`vphysics.dll+0xaaff0`, `rbp=0xfffd r12=0xffff r13=0xfffe`, via `server.dll+0x2eb909` = `CollisionRulesChanged`). Last line: `building Skyrim grid 87 piece 15 … player not over it, 328 units from its middle, in the air`, 0.3 s after `noclip OFF` while flying fast (noclip sprint speed ~1,470). So the guard's rule ("only a piece under the player is dangerous", margin 48) was wrong: a fast player just beside a piece crashes too. **Fifth crash from this bug.**
2. **"Wonky around objects":** at grids 21–23 the lift rule fired 7–23 times a second with the player 126–164 units *above* the ground under its centre. The rule compared the feet with the highest point under the 32×32 footprint; next to an object the 48-unit ray grid is a steep spike, so the player was popped onto it. (Inferred from the status lines; those grids weren't kept on disk.) The ray grid itself also turns objects into steep pyramids, which can't walk smoothly; that part is a limit of stage A.
3. **Stopping mid-air after a noclip launch:** after `noclip OFF` at speed, the player outran the built ground: `HOLD at … no Skyrim ground` (5 times) and guard holds (2 times). Both freeze the player and zero its speed.
4. **Map edge:** the player reached x −16,929 Garry's Mod units (−16,416 is gm_flatmap's wall) ≈ 31,000 Skyrim units from the start. Needs step 5 (moving the origin).
**Worked (verified from logs):** V toggles (`noclip ON/OFF (V in Skyrim)`, 17 toggles), skipped-in-noclip pieces (`left for later`, 14), guard holds built and released, holds released on noclip. Skyrim: `body hooks installed (contacts, land rescue)`; contacts dropped every second while moving (up to 145 above the feet, 56 in noclip); 6 land rescues skipped; Skyrim's own drift ≤ 2.6 units; no origin reset (epoch 1 all session). Step 3d: Skyrim z followed Garry's Mod (two logged "big steps" of 1,015 and 665 units up were Garry's Mod lifting a player that ended noclip 850 units under the ground).
**Client error found:** `terrain.lua:300: attempt to call method 'GetTestId' (a nil value)` in the client status timer: a ground entity that had only just arrived on the client had no data functions yet. Same timer stopped in test 16.

## 2026-10-04 Fixes from test 17: client status error, lift judged at the player's centre (not tested in game)

**Changed:** `terrain.lua`: `T.Status` skips entities without `GetTestId` yet. The lift rule ("player is inside the ground") now uses the ground under the player's centre (bilinear) minus 12, not the highest point under the footprint; where it lifts, it still lifts to 4 above the footprint's highest point.
**Why:** test 17 problems 2 (partly) and the client error.
**Tested how:** `gmod_harness.exe`: HARNESS PASSED (75/75). The harness has no spike next to the player, so the new rule is only covered for the plain "fell through" case.
**Not tested in game.** Problems 1, 3, 4 not addressed: see STATUS.md.

## 2026-10-04 In-game test 18 (lift fix) and a re-plan (proposal only, no code)

**Result (logs: `build/logs-ingame-test-18/`):** user: "tested, doesn't matter anyway". No crash; `lifts` 0 in every status line (fix works for that session; no numbers on bumpiness).
**User's direction:** don't fix the ray grid; step 5 next, then Skyrim's real collision shapes ASAP, checked against SkyCraft.
**Finding from SkyCraft:** SkyCraft never creates physics objects for Skyrim's world in Minecraft; it adds shapes to Minecraft's collision query. Our crash comes from Garry's Mod creating physics objects near the moving player, so it would follow us into step 5 and into real Havok shapes if they were still built with `PhysicsFromMesh`.
**Changed:** `docs/DESIGN.md`: "Phase 1 re-plan after test 17": A collision-field experiment (`ENT:TestCollision`, no physics objects), B ray grid into the field, C step 5, D SkyCraft's stage C ported. Waiting for the user's go-ahead (AGENTS.md: ask before large refactors).

## 2026-10-04 Re-plan step A: collision field experiment, triangles answered from Lua (not tested in game)

**Why:** user's go-ahead for step A; user worried boxes would feel bad around trees and fences. Checked SkyCraft: its final player collision (`fabric/src/main/java/dev/skycraft/world/TriCollider.java`) uses Skyrim's exact triangles because boxes stair-stepped slopes; boxes only for mobs/items. So step A uses triangles.
**Changed:** new `gmod/lua/gmsky/field.lua` (both sides; included by `server.lua`, `client.lua`): a triangle store with a 64-unit lookup grid; a swept box-against-triangle test (separating axes: 3 box axes, face normal, 9 edge crosses; stops 1/32 unit short like Source; a box already overlapping a triangle may move out or along it but not further in); entity `gmsky_field` (`SOLID_OBB`, custom collisions, **no physics object**) whose `ENT:TestCollision` answers from the field. `gmsky_test_field` builds a test scene under the player on both sides: the 3a hill (8,192 triangles) to the west, and a 900×900 floor with a post, a fence, a 12-sided trunk, stairs (6 × 12 units) and a 50° ramp; `gmsky_test_field 0` removes it. `gmsky_field_feet` (default 1) says whether `startpos` is the bottom of the moving box (Garry's Mod wiki example) or its centre; the first 6 box calls are logged with their arguments to settle it. Status lines (server each second, client every 5 s): tests, hits, triangles tested, Lua ms, what the player stands on, feet minus hill.
**Tested how:** `gmod_harness.exe`: HARNESS PASSED, 84 checks (new: box lands on a floor 1/32 above it; ray hits floor; no touch 1 unit above; post stops a box at its face; box inside the floor may leave upward but not go down; 50° ramp normal; scene = 8,352 triangles). Speed in Garry's Mod's Lua: 0.0175 ms per player-sized 300-unit sweep over the hill.
**Not tested in game. Not known:** whether Garry's Mod's player movement traces call `TestCollision` for this entity at all (the experiment's main question), the `startpos` convention, client prediction.

## 2026-10-04 In-game test 19 (step A, Garry's Mod alone): triangle collision from Lua works

**Result (logs: `build/logs-ingame-test-19/`):** user: "the collision feels great … this is definitely the future"; couldn't find the stairs.
- Player movement calls `TestCollision`: 300–820 box tests a second on the server while moving, 0.03–0.3 ms of Lua per tick (1.2–20 ms per second), slowest call 0.84 ms.
- `startpos` is the **bottom** of the moving box (`gmsky_field_feet 1` correct): scene 1, the player stood on `gmsky_field` at z −16,279.9, i.e. 0.03 above the floor (−16,280) = the 1/32 gap we leave. Seen extents: (16, 16, 18) and (8, 8, 36) (not the 16×16×72 standing hull; unexplained, works anyway).
- Scene 2 (the user probably set `gmsky_field_feet 0`): the box started inside the floor ("hit at 0.000") and the player stood 4 above it; then left.
- "Standing on the map" lines: the player walked off the floor's west edge (8 units down to the map) and under the hill, which is a sheet 100–265 units above the floor there ("feet minus hill −339"). **Scene layout mistake**, not a collision fault. Stairs were 450–700 units east of the start.
- Client: 0 box tests (single-player: the client doesn't predict movement), ~950 ray tests a second (view traces), ~1 ms per frame of Lua: long rays scan every cell of their box. To speed up later (walk only the cells along the ray).
- Not verifiable from these logs: hits on the post/fence/trunk specifically (no per-object log); the user's report is the evidence.
**Next:** step B (ray grid onto the field).

## 2026-10-04 Re-plan step B: Skyrim's ground grid becomes Lua triangle collision; physics-object ground removed (not tested in game)

**Changed:**
- `gmod/lua/gmsky/terrain.lua` rewritten. Each new grid from Skyrim becomes the triangles of one persistent `gmsky_field` entity at once (8,190 triangles for a full grid). **Removed:** the `PhysicsFromMesh` ground (`gmsky_terrain` entity, 4×4 pieces, piece reuse and `gmsky_reuse_pieces`, the crash guard and its holds, rebuilding pieces with misses), the 3a test hill (`gmsky_test_terrain`; `gmsky_test_field` replaces it) and the crash-investigation replay tool (`gmsky_replay`). Kept: grid polling, "home" on a new origin, the "no ground" hold (now gives the player its speed back when it lets go: test 17 problem 3), noclip ending holds, the centre-based lift rule.
- `gmod/lua/gmsky/field.lua`: `F.FromGrid(g)` (squares touching a miss left out) and `F.UseGrid(ent, g)` (server); the entity's networked `GridId` makes the client build the same field from its own copy of the grid (logs `field: grid N built on this side`).
- Harness: fake entities store `SetX` values and run the scripted entity's own methods; the ground tests now check: grid → ground in one tick, the real `ENT:TestCollision` stopping a falling box on the grid, the next grid replacing it, a no-ground hold, noclip releasing it, and the speed given back.
- Unchanged: the Skyrim plugin, the protocol, the module (`ReadGridFile` and grid recording stay).
**Why:** user's go-ahead for the re-plan; test 19 showed Lua triangle collision works and feels good. No physics objects means the "known engine bug" crash can't happen any more.
**Tested how:** `cargo build --release`; `gmod_harness.exe` → HARNESS PASSED, 77 checks (the old piece/guard/replay checks are gone with the code). Field speed 0.021 ms per sweep.
**Not tested in game.** Not known: the hitch from building 8,190 triangles in Lua on each new grid (6–10 ms per side in test 19's scene), the client's cost for long view rays over the 1,632-unit field.

## 2026-10-04 In-game test 20 (step B): feels great, hitches when launched horizontally

**Result (logs: `build/logs-ingame-test-20/`):** 73 grids, no crash. User: "feels great, strange hitches when launching myself horizontally".
- **Cause 1 (the hitch):** after noclip launches (~1,300 Garry's Mod units/s ≈ 2,450 Skyrim units/s) the player outran the ground 14 times (`HOLD at … no Skyrim ground`, 3–25 held ticks = 45–375 ms frozen, then `speed … given back`). Each grid took 320–517 ms (20–32 frames) to cast and was centred where the player was when casting *started*: the player had covered 800–1,300 Skyrim units of the grid's 1,536 half-width by the time it arrived.
- **Cause 2:** turning a grid into 8,192 triangles took 7.4–20.0 ms of Lua per grid (server; the client too): a small stall at every new grid.
- Speed given back worked (e.g. `speed (-1385, -383, -137) given back`).

## 2026-10-04 Fixes for test 20's hitches (not tested in game)

**Changed:**
- `skse/src/Terrain.cpp`: a new grid is centred where the player *will be*: position + Garry's Mod's velocity × (recent cast time + 0.1 s), snapped to the lattice. The "published" line says how far ahead it was centred. Above 1,000 Skyrim units/s the ray budget is 4 ms a frame (was always 1.5).
- `gmod/lua/gmsky/field.lua`: a grid field keeps only the heights; a square's two triangles are made the first time a trace touches it (grid build now < 1 ms). Long traces are tested in 128-unit sections in order. Traces exactly along a square's edge also test the square on the other side (found by a new harness check: a ray on a grid line slipped between two squares), with a 0.0001-unit tolerance in the edge test.
- `terrain.lua`: logs squares instead of triangles; the status line shows how many triangles have been made so far.
- Harness: 78 checks (new: long slanted ray hits the surface and makes < 300 triangles; ray along a square edge hits).
**Why:** test 20 hitches; user wants them fixed before committing.
**Tested how:** `cargo build --release`; `gmod_harness.exe` → HARNESS PASSED 78/78; `cmake --build --preset release` (Skyrim plugin built and copied).
**Not tested in game.** If the player still outruns the ground at noclip launch speed, the next step is SkyCraft's streaming (small regions nearest-first in a wider radius), planned for step D.

## 2026-10-04 In-game test 21 (test 20 fixes): "absolutely perfect"

**Result (logs: `build/logs-ingame-test-21/`):** Skyrim + Garry's Mod, 164 grids, 17 noclip launches, no crash (no new dump), no Lua errors. **0 `HOLD at` lines** (test 20: 14). Grids centred up to 1,205 Skyrim units ahead of the player; slowest grid 36 frames / 584 ms from start. Grid build in Garry's Mod 0.1 ms (test 20: 7.4–20.0 ms). Slowest single `TestCollision` 1.26 ms. User: "absolutely perfect".
**Next:** commit the steps; then step C (moving the origin, step 5), then step D (Skyrim's real collision shapes, SkyCraft's stage C).

## 2026-10-04 Re-plan step C: re-centring the origin (not tested in game)

**Changed:**
- Plan first: `docs/DESIGN.md` re-plan step C (and §15's risk line). Checked SkyCraft first: it has no floating origin; its teleport number + acknowledgement (`Game.cpp` `teleportSeq`/`teleportAck`) is the idea reused: never mix numbers from two frames.
- **Who decides:** Garry's Mod. When the player is more than `gmsky_recentre_at` (default 8,000 Garry's Mod units ≈ 15,000 Skyrim units) from home along x, y or z, `terrain.lua` `T.Recentre` moves the player and the anchor by the same whole-unit amount in one tick, before the player state is written. Skyrim's origin and epoch don't change, so Skyrim's position of the player doesn't change. Speed, noclip and a hold are kept. Works in the air and in noclip too.
- Module (`gmod/module/src/lib.rs`): new `gmsky.Anchor()` and `gmsky.ShiftAnchor(dx, dy, dz)` (logs every shift).
- **Protocol 7** (same layout): Skyrim's ground grids are now relative to the anchor, `(skyrim − origin) × scale` (`Terrain.cpp`). A grid cast before a re-centre and arriving after it is still right. Garry's Mod keeps the ground entity at the anchor; `field.lua` `TestCollision` works relative to its entity, so a re-centre moves one entity (no physics object). The step A test scene is built relative to its entity too.
- Skyrim `Smooth.cpp`/`Puppet.cpp`: blending uses player-minus-anchor, so a re-centre between two ticks isn't drawn as a jump.
- Logs: `origin: RE-CENTRE #n` (player before/after, shift, anchor before/after, player minus anchor before/after: must match, speed); `origin: 2 ticks after re-centre #n` (speed kept?); module `anchor shifted by …`; the server status line counts `re-centres`.
- Harness: 85 checks (was 78). New: re-centre moves the player state and anchor together (player minus anchor unchanged), the ground entity follows the anchor, the player isn't held and the ground is still under it, only one re-centre, log lines. Test grids are anchor-relative now. **Found:** the "long slanted ray" check from the test 20 fixes was computed but never checked (an unused variable); it is checked now and passes.
**Why:** test 17 reached the map's wall (gm_flatmap is ±16,300 units); next step in the re-plan.
**Tested how:** `cargo build --release`; `gmod_harness.exe` → HARNESS PASSED (85); `protocol_test.exe` → PROTOCOL TEST PASSED; `cmake --build --preset release` (Skyrim plugin built and copied).
**Not tested in game.** Not known: whether `SetPos` on the player keeps its speed in the real game (logged 2 ticks later), and whether anything in Garry's Mod's own view smoothing shows a blip in the Garry's Mod window (only the Skyrim window matters). Other Garry's Mod entities (props, NPCs) are not moved with a re-centre; none are used yet.
**Next:** in-game test 22 (STATUS.md).

## 2026-10-04 In-game test 22 (step C, re-centring): works

**Result (logs: `build/logs-ingame-test-22/`):** user: "works flawlessly". **42 re-centres** (37 with `gmsky_recentre_at 500` while walking, jumping and in short noclip flights; 5 at 8,000 in long noclip flights). Furthest point: player minus anchor y = 58,024 Garry's Mod units ≈ 109,000 Skyrim units from the start (test 17 hit the wall at ~31,000). "Player minus anchor" identical before and after in all 42 (checked by script); speed kept (e.g. #1: (218, 336, 0) before, (221, 334, 0) two ticks later); 0 `HOLD at`; no Lua errors; no new crash dump; one origin epoch all session.
**Seen and checked, not caused by re-centring (as far as the logs show):** Skyrim logged 6 `PUPPET big step` lines (300–491 units in one frame; tests 20–21: 0), and 9 seconds where one frame's step was > 3× the average (tests 20–21: 0). By lining up both logs' clocks (noclip toggles; the one `LIFT` line matched Skyrim's 491-unit step within 5 ms):
- 28 of the 31 seconds that contain re-centres are perfectly even (largest step ≤ 1.3× the average, up to 4 re-centres a second at ~1,460 units/s).
- 6 of the 9 odd seconds contain no re-centre. One is Garry's Mod's existing lift rule (`LIFT: player was 324.8 below the ground`, after noclip ended underground).
- Most odd seconds are within ~1 s of the Skyrim window losing focus. Test 22 had 11 window switches (tests 20/21: 0/2), probably from typing `gmsky_recentre_at` into Garry's Mod's console.
**Not proven** what moves the player in those frames; no per-tick Garry's Mod positions were logged. Next change adds that log.

## 2026-10-04 Jump log in Garry's Mod (not tested in game)

**Changed:** `gmod/lua/gmsky/server.lua`: every tick, the player's move (relative to the anchor, so a re-centre doesn't count) is compared with what its speed explains; more than 40 units off is logged as `JUMP tick N: moved …, speed explains …, … off; speed before -> after; noclip/walking, on ground/in air, on <entity>, input from Skyrim on/off/switched, terrain holding / lifts / re-centres` (at most 5 lines a second; the status line adds `jumps N`). Harness: fake `game.GetWorld`; 87 checks (new: the harness's own 800-unit teleport is logged as exactly one JUMP).
**Why:** test 22's unexplained single-frame jumps (previous entry). The next test shows what Garry's Mod was doing when they happen (or that Garry's Mod didn't jump and the cause is on Skyrim's side).
**Tested how:** `cargo build --release`; `gmod_harness.exe` → HARNESS PASSED (87).
**Not tested in game.**

## 2026-10-04 Test 22's jumps: explained by the user

User: they alt-tabbed several times during test 22, gameplay was "buttery smooth"; don't pursue the single-frame jumps further. (Consistent with the logs: most odd seconds were within ~1 s of a window switch.) The JUMP log stays in for now.

## 2026-10-04 Re-plan step D: Skyrim's real collision shapes, streamed and answered in Rust (not tested in game)

**Plan:** `docs/DESIGN.md` re-plan step D (written first). **Read in SkyCraft first:** `skse/src/Collision.cpp/.h`, `Link.cpp` `WriteCollision`, the collision ring in `protocol/skycraft_protocol.h`, `fabric/.../world/SkyCollision.java`, `SkyTri.java`, `TriCollider.java`. Taken: region streaming, layers, shape decoding with SkyCraft's offsets, guarded reads, triangles from boxes/capsules/hulls, byte ring with pad messages, clear epoch, 1 s refresh near the player, per-frame limits. Not taken: voxels, digging, materials, Minecraft's axis swap.
**Changed:**
- **Protocol 8:** shared memory renamed `Local\GmodSkyrim_v2` and grown to 64 KB + a 16 MB collision ring (messages: pad, clear {collision epoch, origin epoch}, region {rx, ry, rz, epoch, count, lo, size} + triangles {9 floats, flags}); flag `GMSKY_PF_SHAPES` (Garry's Mod asks for shapes instead of the ray grid). Ring writer in C++ (`Link::RingWrite`) and Rust (`shm.rs` `ring_write`, for tests), reader in Rust (`ring_read`).
- **Skyrim** `skse/src/Collision.cpp/.h` (new, adapted from SkyCraft): regions of 512 Skyrim units, ±5 around the player, 3 below, 2 above (726); the 27 around the player first, then nearest the point the player reaches in 0.5 s; 3 regions / 2.5 ms per frame (6 / 5 ms above 1,000 units/s); worker thread makes triangles and writes the ring in Garry's Mod units relative to the anchor; regions forgotten beyond 6 (Garry's Mod drops at 8); dropped (ring full) regions are sent again. Starts with "clear" when Garry's Mod asks for shapes / on a new origin; stops when the puppet is off or Garry's Mod asks for the grid. `Terrain.cpp` skips the ray grid while shapes are asked for. `Probe.cpp` calls it every frame, logs each second.
- **Garry's Mod module** `gmod/module/src/shapes.rs` (new): the triangle store (per region, 64-unit lookup cells) and the swept test, `field.lua`'s ported line by line. Lua functions `gmsky.PollShapes`, `Sweep`, `ShapesKnown`, `ShapesBounds`, `ShapesStatus`; `lua_toboolean` binding.
- **Lua:** `gmsky_ground shapes` (default) / `grid`. Shapes mode: the ground entity's `TestCollision` calls `gmsky.Sweep`; its bounds cover the stored regions; the player is held ("waiting for Skyrim's collision") until the region it stands in and the one below have arrived, never in noclip. The server polls the ring every frame; status lines show regions, triangles, sweeps, Rust ms.
- `THIRD-PARTY-NOTICES.md`: the new files adapted from SkyCraft.
**Tested how:** `cargo test --release` (protocol: new ring test, 1 MB messages, wrap-around, full ring refuses; module: Rust sweep matches field.lua's numbers, region known/evict/epochs) all pass; `gmod_harness.exe` → HARNESS PASSED, **97 checks** (new: switch to shapes holds the player, flag sent, clear + 4 regions arrive and release it, the entity's `TestCollision` lands a falling box on Skyrim's floor at the right height, bounds, a new clear drops everything; and **3,000 random sweeps over 137 regions (8,592 triangles): Rust and field.lua give the same fraction every time**, 6 ties (same fraction, another triangle's normal, box starting inside a corner), Rust 0.0013 ms vs Lua 0.020 ms per sweep); `protocol_test.exe` PASSED; Skyrim plugin built (no warnings) and copied.
**Not tested in game.** Not known: whether SkyCraft's Havok offsets hold for every shape we meet (same game build 1.7.104; every read is guarded and a fault is counted in the log); harvest cost in cities; ring throughput; how stair helpers feel in Garry's Mod's movement; doors (refreshed every second near the player).
**Known limit (by design for now):** in shapes mode nothing lifts a player who ends noclip under the ground (the grid mode's lift rule needs the grid's heights); it falls until V is pressed. Not fixed: Garry's Mod alone can't tell "under the land" from "in a cave"; Skyrim's land height would.
**Next:** in-game test 23 (STATUS.md).

## 2026-10-04 In-game test 23 (step D, Skyrim's real shapes): "perfect and superior to grid"

**Result (logs: `build/logs-ingame-test-23/`):** user: shapes are "perfect and superior to grid". Skyrim: 9 × `COLLISION streaming starts` (loads, doors, origin changes), **0 faults** and **0 dropped** in every second; slowest harvesting frame 35.9 ms (a one-frame hitch; the budget is 2.5/5 ms per frame plus gathering bodies, worth watching in cities). Garry's Mod: up to 1,199 regions / 90,116 triangles stored, slowest Rust sweep 99 µs, no Lua errors, 5 short `waiting for Skyrim's collision` holds.
**Noclip under the ground (decided):** the user likes it as it is: you fall "under the earth" like in Source, and Skyrim eventually moves the player back up (seen as "Skyrim moved the player" → new origin). Not to be changed.

## 2026-10-04 Phase 1 step 4: the camera (eye height, crouch, view punch, FOV) (not tested in game)

**Plan:** `docs/DESIGN.md` Phase 1 step 4 (written first). **Read in SkyCraft first:** `Game.cpp` `PinCameraAndSky`, `ApplyMcFov`/`RestoreFov`, eye height blended between ticks, walk bob. Taken: camera root + camera position + sky put at the eye in the same two spots as the rotation; FOV saved, set, restored. Not taken: the walk bob (Garry's Mod/Sandbox doesn't bob the camera, only the weapon model; Garry's Mod's **view punch** is sent instead). Also fixed in the design doc: Phase 5 no longer mentions "time/weather sync" (Garry's Mod has neither; pointed out by the user).
**Changed:**
- **Protocol 9:** `GmskyPlayerState.punch_deg[3]` (size 144 → 160; slots after it moved, both layout checks updated).
- Garry's Mod (`server.lua`, module): sends `GetViewPunchAngles()`; FOV = `GetFOV()`, or the player's `fov_desired` when that is 0.
- Skyrim: `Smooth.cpp` blends the eye height together with the position (`Smooth::EyeHeight`); `Puppet.cpp` `CameraNow`: view + punch (pitch, yaw, roll), eye = this frame's feet + eye height / scale (64 → 120.5, 28 crouched → 52.7), FOV; `Camera.cpp` `Drive`: rotation (with roll), camera root / camera / sky at the eye, `worldFOV` = Garry's Mod's FOV (Skyrim's own saved and restored when the puppet is off or `gmsky_camera 0`). Log each second: `CAMERA eye (step 4): …` (placed count, eye height in both units, how far we moved Skyrim's own camera, view punch max, FOV in use and Skyrim's own).
**Tested how:** `cargo build --release`; `gmod_harness.exe` → HARNESS PASSED (98: new check, the view punch reaches the player state); `cargo test --release` passes; `protocol_test.exe` PASSED; Skyrim plugin built (no warnings) and copied.
**Not tested in game.** Not sure: the sign of the view punch's roll (small; the log shows it); whether `GetFOV()` on the server returns 0 or the real FOV in this Garry's Mod build (either way the log shows the FOV in use).
**Next:** in-game test 24 (STATUS.md).

## 2026-10-04 Phase 1 step 4b: crouch = Skyrim's sneak (not tested in game)

**Plan:** `docs/DESIGN.md` Phase 1 step 4b (user request). **Adapted from SkyCraft** (`Game.cpp` `SyncSneak`, MIT).
**Changed:** `skse/src/Puppet.cpp`: while the puppet is on, Garry's Mod's crouch (`GMSKY_PF_CROUCHING`, not in noclip) makes Skyrim's player sneak through Skyrim's own sneak action (`PlayerControls::DoAction(ActionSneak)`, `REL::ID(42350)`), whenever the two disagree, at most every 0.3 s; not while dead, mounted or swimming. Logs: `SNEAK … toggled (taken/refused)` for the first 20 toggles, `SNEAK Garry's Mod crouching/standing, Skyrim sneaking/not sneaking; toggles …` each second.
**Tested how:** Skyrim plugin built (no warnings) and copied. No Garry's Mod change (the crouch flag was already sent; the harness checks it).
**Not tested in game.** Not sure: that ID 42350 is right for our exact game build (it is SkyCraft's, same 1.7.104; not checked before calling, unlike the land-rescue patch).

## 2026-10-04 Launcher loads gm_flatmap directly again (not tested)

**Changed:** `tools/launch_gmod.ps1`: default `-Map gm_flatmap`; it loads the map straight into single-player (`+maxplayers 1 +map gm_flatmap`) when `garrysmod\maps\gm_flatmap.bsp` exists, otherwise it says so and stops at the main menu as before; new `-Menu` switch.
**Why:** the user is moving gm_flatmap from a Workshop addon to a plain map file, which (unlike Workshop maps) can be loaded from the command line, as gm_flatgrass can.
**Tested how:** the script parses (PowerShell parser). Not run: the file isn't in `garrysmod\maps` yet (checked, read-only).
**Not tested.**

## 2026-10-04 In-game test 24 (camera, sneak, map autoload): "perfect, all tests done"

**Result (logs: `build/logs-ingame-test-24/`):** camera 120.5 Skyrim units above the feet standing, 52.7 crouched, values between during the duck transition (e.g. 63.3, 80.7); view punch up to 7.97°; `CAMERA FOV: Skyrim's 80.0 -> Garry's Mod's 100.0`. Sneak: 20 logged toggles, all `taken`, 0 refused. The launcher loaded gm_flatmap directly (`map gm_flatmap room`), gmsky loaded by itself.
**Not exercised:** `gmsky_camera 0` (no `FOV back to Skyrim's own` line), a second FOV value.

## 2026-10-04 Phase 2 planned; step 2a: overlay experiment in Garry's Mod (not tested in game)

**Plan:** `docs/DESIGN.md` "Phase 2 steps: the overlay" (written first). **Read in SkyCraft first:** `skse/src/Overlay.cpp`, `fabric/.../client/FrameExporter.java`, the overlay slots in `protocol/skycraft_protocol.h`: SkyCraft ships the "copy through normal memory" path (Minecraft draws hand + HUD on a transparent background, copies it back from the GPU asynchronously into one of three picture slots in shared memory; Skyrim's `Present` hook uploads the newest one and draws it full screen with premultiplied alpha, skipped in menus, Skyrim's own first-person arms hidden). Unknowns for Garry's Mod, measured first: whether Source's alpha can be trusted (fallback: matting from a black and a white background), whether the view model and HUD can be drawn alone into a render target, and which Direct3D 9 Garry's Mod uses.
**Changed:**
- Module: `gmsky.GraphicsInfo()` (lists the loaded graphics DLLs with path and size, and whether `vulkan-1.dll` is loaded, which DXVK would need; read only, nothing hooked); `gmsky.WriteLogFile(name, data)` (saves binary data to `logs/overlay/<name>`; plain file names only); `lua::to_bytes` (a Lua string's raw bytes).
- New `gmod/lua/gmsky/overlay.lua` (client, included by `client.lua`): logs `gmsky.GraphicsInfo()` at load; console command `gmsky_test_overlay [name]` saves, on the next frame, `<name>_screen.png` (the screen) and the overlay drawn alone (view models with the weapon's own view-model hooks, hands, then `render.RenderHUD`) into a render target cleared to transparent black and to transparent white (`<name>_black.png`, `<name>_white.png`), and logs draw and capture times.
- Harness: 101 checks (new: overlay.lua runs on the client and logs the graphics line; `WriteLogFile` refuses `../escape.png` and keeps binary data byte for byte).
**Tested how:** `cargo build --release`; `gmod_harness.exe` → HARNESS PASSED (101). The drawing itself can't run in the harness (no renderer).
**Not tested in game.** Not known: whether `render.RenderHUD` draws the engine HUD (health/ammo) into a render target; whether `render.Capture` keeps alpha (`alpha = true`); whether the view model drawn this way is lit like Garry's Mod's own.

## 2026-10-04 In-game test 25 (step 2a, first try): pictures empty; new approach (not tested in game)

**Result (logs: `build/logs-ingame-test-25/`):** user: "they're all black".
- `graphics:` Garry's Mod draws with Windows' own `C:\Windows\SYSTEM32\d3d9.dll`; **no Vulkan / DXVK**; also loaded: `shaderapidx9.dll`, `materialsystem.dll`, `d3d11.dll`, `dxgi.dll` (the latter two are loaded by something else; Garry's Mod's renderer is `shaderapidx9`).
- Measured from the PNGs (Pillow): every `_black.png` pixel is exactly (0, 0, 0, 0) and every `_white.png` pixel (255, 255, 255, 0): the render target and `render.Capture` with `alpha = true` work (the transparent clear colour survives), but **nothing was drawn**: `vm:DrawModel()`, `hands:DrawModel()` and `render.RenderHUD` from the `PostRender` hook drew nothing (0.1-0.3 ms). Guess (not proven): Garry's Mod only draws view models and the HUD inside its own view-model / HUD passes. `render.Capture` of the screen in `PostRender` returned nothing.
- Capture + PNG of 1280×720: ~20 ms (only for the experiment; the real path copies raw pixels).
**Changed:** `overlay.lua`: Garry's Mod draws a whole second view into the render target (`render.RenderView`, view model and HUD on, no post-processing); at the first `PreDrawViewModel` / `PreDrawPlayerHands` of that view the render target is cleared to transparent black or white, so what remains is what Garry's Mod draws after the world, drawn by its own code. The same view without the clear is saved as `<name>_scene.png`. Logged per view: time, whether the clear happened, which hooks ran (`PreDrawViewModel`, `PreDrawPlayerHands`, `HUDPaint`).
**Tested how:** harness PASSED (101). **Not tested in game.** If this works, 2b could do the same clear in Garry's Mod's normal frame instead (no second view: the Garry's Mod window would show only hands and HUD, and it is going out of sight in 2d anyway).

## 2026-10-04 In-game test 26 (step 2a, second try): only the sky; third try (not tested in game)

**Result (logs: `build/logs-ingame-test-26/`):** user: "the scene image is just the skybox, no hud, viewmodel or world". Measured: in every view `PreDrawViewModel` and `PreDrawPlayerHands` ran once and the clear happened (`cleared at the view model: true`), yet `_black`/`_white` are again exactly the clear colour, and the three `_scene.png` are pixel-identical for three different weapons (26,155 colours, only sky). Each `render.RenderView` took 0.4-0.9 ms: the world wasn't drawn. So a second view started from `PostRender` draws only the sky. Cause not known (not investigated further: the plan for 2b never needed a second view).
**Changed:** `overlay.lua`, third try: Garry's Mod's own normal frame, three frames in a row: (1) as it is, (2) the screen cleared to black at the frame's first `PreDrawViewModel` / `PreDrawPlayerHands`, (3) the same with white; each captured in `PostDrawHUD` (after the whole HUD, still inside the frame). Logged per frame: hooks that ran, whether the clear happened, capture time.
**Tested how:** harness PASSED (101). **Not tested in game.**

## 2026-10-04 In-game test 27 (step 2a, third try): hands and weapon captured; HUD missing (fix not tested)

**Result (logs and pictures: `build/logs-ingame-test-27/`), measured with Pillow/NumPy:**
- **The normal-frame method works for the view model:** clearing at the frame's first `PreDrawViewModel` and capturing in `PostDrawHUD` gives the hands and weapon alone: crowbar 17,304 px, physics gun 82,857, tool gun 58,467 (box x 618-1188, y 347-719: lower right, where the view model is).
- **Source's own alpha can't be used:** on fully solid weapon pixels it is 13-90 (of 255) and the background clear's 0 survives elsewhere; it is something Source stores there (probably depth for fog), not transparency.
- **Two-background matting is exact:** where something is drawn, the black and white pictures agree to the last bit (crowbar, physics gun: 0 difference on every drawn pixel, all solid); empty pixels differ by exactly 255. Tool gun: 335 partly see-through pixels (its screen / glow), channel spread ≤ 17.
- **The HUD was missing**, also in the plain frame (no HUD colours in the health/ammo corners, nothing at the crosshair), and `HUDPaint` didn't run in any captured frame. Most likely cause: the capture ran on the very next frames, while the console the command was typed in was still open (Garry's Mod doesn't draw the HUD then).
- Times: capture + PNG 21-27 ms per frame (experiment only).
**Changed:** `overlay.lua`: the capture waits until the console / menu is closed and 0.5 s have passed (`gui.IsGameUIVisible`, `gui.IsConsoleVisible`), logs `console closed, capturing`.
**Tested how:** harness PASSED (101). **Not tested in game.**

## 2026-10-04 In-game test 28 (step 2a, console fix): HUD captured, crosshair not; preview tool (not tested in game)

**Result (logs and pictures: `build/logs-ingame-test-28/`):** user: "still no hud that I can see, only on the scene", and "darkening around the bottom half" of the scene. Measured:
- `HUDPaint` now runs in every captured frame (the console fix works).
- **The HUD is in the overlay:** the health panel at x 24-176, y 648-701 (its background 18% opaque, the digits 64-86%: 661 yellow pixels against 689 on screen, the rest edge pixels); ammo corner 4,094-30,168 drawn pixels depending on the weapon. It wasn't visible to the user because the PNGs carry Source's meaningless alpha channel (0 on most of those pixels), which an image viewer applies. **Not a capture problem.**
- **The "darkening" is the ground:** scene brightness by eighths, top to bottom: ~180-195 (sky), then ~80 (gm_flatmap's floor) from the horizon down, even from left to right (no vignette).
- **No crosshair:** nothing drawn in the centre 25×25 of the overlay, and the scene's centre is flat (75-85) for the physics gun and the tool gun. Either the crosshair is drawn after `PostDrawHUD`, or it is off.
**Changed:**
- `overlay.lua`: each frame is also captured in `DrawOverlay` (the last 2D hook): `<name>_scene_late.png`, `_black_late.png`, `_white_late.png`.
- New `tools/overlay_preview.py`: turns each black/white pair into `<name>_preview.png` (the overlay over a grey checkerboard, with the real transparency from the pair, composited as Skyrim will: black + (1 - alpha) × background) and prints drawn pixels per area (health, ammo, crosshair). Run on test 28: crowbar 25,553 drawn px, physics gun 91,124, tool gun 158,753; health corner 8,250 px each; crosshair 0.
**Tested how:** harness PASSED (101); the preview tool run on test 28's pictures. **Not tested in game.**

## 2026-10-04 In-game test 29 (step 2a, late capture, crosshair on): crosshair captured; capture point = DrawOverlay; one-frame alpha test (not tested in game)

**Result (logs and pictures: `build/logs-ingame-test-29/`, `tools/overlay_preview.py`):** the crosshair was off in the user's settings ("I had my crosshair disabled"); with it on, it is captured: 5 pixels, Half-Life 2's dots at (639, 352), (629, 360), (639, 360), (649, 360), (639, 368). Health corner 8,250 px, ammo corner 4,102-33,808 px. **Panels drawn after `PostDrawHUD`** show only in the `DrawOverlay` captures: physics gun +14,451 px (x 1006-1279, y 545-644), tool gun +16,310 px (x 956-1259, y 514-572), dark and see-through (alpha median 0.35): Garry's Mod's on-screen panels (notifications, hints, the tool's info). Chat and menus are panels too. **So the real capture point is `DrawOverlay`.**
**Open problem:** the black/white pair comes from two consecutive frames; when the hands move between them (a swing, weapon sway), the matte is wrong at the edges.
**Changed:** `overlay.lua`: a fourth frame, cleared to transparent black with `render.SetWriteDepthToDestAlpha(false)` (Source stores depth in the alpha channel by default; set back in `PostRender`): `<name>_alpha.png`. `tools/overlay_preview.py` compares that picture's own alpha with the matte (mean difference, pixels off by more than 0.1) and writes `<name>_preview1.png` from it. If it matches, one frame is enough (no black/white flicker, no motion error).
**Tested how:** harness PASSED (101). **Not tested in game.**

## 2026-10-04 In-game test 30 (one-frame alpha) and a re-check against SkyCraft; test 31 built (not tested in game)

**User:** "double check that we're doing something at least similar to what SkyCraft is doing. Refocus."
**Re-checked SkyCraft** (`fabric/.../client/mixin/LevelRendererMixin.java`, `FrameExporter.java`, `skse/src/Overlay.cpp`): Minecraft doesn't draw its world while linked, so hand + HUD land on a transparent background **in one frame with Minecraft's own alpha**; async readback through staging buffers; three picture slots; Skyrim's `Present` hook draws it. **Our black/white matting (tests 27-29) was a drift from that** (two frames, motion errors, flicker): now only a measuring tool. `docs/DESIGN.md` Phase 2 has the side-by-side table.
**Test 30 result (logs and pictures: `build/logs-ingame-test-30/`), one frame with `render.SetWriteDepthToDestAlpha(false)`, compared with the matte per area (pixels off by > 0.1 / drawn):** view model mostly right (physics gun: 490 edge pixels of 27,099; solid pixels read 1, empty 0), but the tool gun's screen writes 0 (6,836 of 28,318); ammo numbers right (0-4 off); **2D parts write no alpha:** health panel 8,222 of 8,250 off (0 instead of 0.18), notification panels 6,756-16,307 off, crosshair 5 of 5 off (0 instead of 1). Frames lined up (colour difference to the black frame 0.01-0.25 of 255), so motion wasn't a factor in this test.
**Changed:** `overlay.lua`: two more frames: `alpha2d` (from `PreDrawHUD` on: `render.OverrideAlphaWriteEnable(true, true)` and `render.OverrideBlend` with the alpha channel blended as "over": src + dest × (1 − src alpha)) and `alphaall` (the same from the view model on); set back in `PostRender`. `tools/overlay_preview.py`: per-area comparison for `alpha`, `alpha2d`, `alphaall`, previews `<name>_preview_<variant>.png`.
**Tested how:** harness PASSED (101); preview tool run on test 30. **Not tested in game.**

## 2026-10-04 In-game test 31 (forced 2D alpha): no fix; stopped and wrote STATUS.md

**Result (logs and pictures: `build/logs-ingame-test-31/`; `tools/overlay_preview.py`):** forcing alpha writes with "over" blending of the alpha channel (`render.OverrideAlphaWriteEnable` + `render.OverrideBlend`) from `PreDrawHUD` (`alpha2d`) or from the view model (`alphaall`) does **not** give the 2D parts their alpha: health panel 7,174-7,956 of 8,250 pixels still off by > 0.1 (default 8,222), notification panels unchanged (16,304 of 26,175), crosshair now 324 pixels off around it (alpha written where nothing is visible), and the view model gets worse (540 → 1,815-1,870 of 20,417 off). Ammo numbers right in all variants.
**Two real attempts failed** (tests 30, 31) at getting Garry's Mod's 2D drawing to write transparency from Lua, so per AGENTS.md I stopped and wrote up `STATUS.md` ("The current problem", "Ideas not tried yet": a native Direct3D 9 render-state fix below VGUI (recommended, SkyCraft's one-frame model), a hybrid with matting for 2D only, or our own Lua HUD). Waiting for the user's decision.


## 2026-10-04 Troubleshooting idea 3: read how VRMod gets Garry's Mod's UI with transparency (research only)

**Changed:** `docs/DESIGN.md` Phase 2, "2a after test 31": what VRMod does, and the plan for ideas 2 and 1.
**Why:** after test 31 I listed seven ways to troubleshoot (1 read Direct3D's settings, 2 Lua test patches, 3 read VRMod, 4 `PaintManual` into our own render target, 5 the native fix below VGUI = STATUS's first idea, 6 our own Lua HUD, 7 hybrid). The user chose 1-3 first, and likes 5 if it also covers Lua-drawn UI.
**Tested how:** read VRMod's maintained x64 fork (https://github.com/Abyss-c0re/vrmod-x64, `lua/vrmod/ui/cl_hud.lua`, `cl_ui.lua`; downloaded to the session scratchpad, not into the project; no licence stated, so nothing copied). SkyCraft re-checked first: it has no 2D-alpha trick (Minecraft's GUI writes alpha itself, by its exporter's comment; not checked in Minecraft's code).
**Result:**
- VRMod paints **Derma menus (Lua UI) into its own render target**: `PushRenderTarget`, `cam.Start2D`, `render.OverrideAlphaWriteEnable(true, true)`, `render.Clear(0, 0, 0, 0)`, `render.SetWriteDepthToDestAlpha(false)`, `panel:PaintManual()`. **No `OverrideBlend`.** We tried neither "alpha writes without the blend override" nor "our own render target + `PaintManual`".
- VRMod does **not** capture Garry's Mod's engine HUD. It draws its own health / armour / ammo in Lua (= troubleshooting idea 6). `render.RenderHUD` is optional and called "often empty / noisy" (like our test 25).
- Possible reason (from memory of Source SDK 2013 shaders, **not verified for Garry's Mod**): Source turns alpha writes off for translucent materials. The HUD and panels are translucent; the view model is opaque.
**Still broken / not tested:** nothing built in this step.
**Next:** idea 2 (Lua test patches) and idea 1 (read Direct3D's settings), both in one in-game test (32).

## 2026-10-04 Troubleshooting idea 2: Lua test patches for 2D alpha, `gmsky_test_alpha2d` (not tested in game)

**Changed:** new `gmod/lua/gmsky/overlay2d.lua` (included from `client.lua`); new `tools/alpha2d_report.py`; harness check that it loads.
**Why:** find out which kinds of 2D drawing write alpha and which don't, with shapes whose right alpha is known (no black/white reference).
**What it does:** console `gmsky_test_alpha2d [name]`, then close the console. 3 consecutive frames (`plain`: nothing forced; `write`: alpha writes forced, VRMod's way; `over`: alpha writes + "over" blending, test 31's way). Each frame is cleared to transparent black at `PreDrawHUD` (depth-in-alpha off), then white 40 × 40 patches with alpha 255 / 128 / 46 are drawn with `surface.DrawRect`, `surface.DrawTexturedRect`, `draw.RoundedBox`, `render.DrawQuad` (in `HUDPaint`) and in a Derma panel (Lua UI, painted by the engine's VGUI pass), and the frame is captured in `DrawOverlay`. Once, VRMod's way: a second Derma panel painted manually into our own render target (`<name>_rt.png`). Saves `<name>_plain/write/over/rt.png` and `<name>_patches.txt` in `logs/overlay/`. The report prints, per patch, the measured alpha against the right one and against alpha × alpha (alpha written but blended like the colour), plus the engine HUD's health digits and the crosshair.
**Tested how:** harness PASSED (103 checks); report tool run on made-up pictures (right / alpha × alpha / none recognised). **Not tested in game.**
**Still broken / not tested:** whether `render.DrawQuad` draws at screen coordinates inside `HUDPaint` (the report says "not drawn" if not); whether `render.Capture` reads our own render target while it is pushed. The window shows 3 black frames during the test.

## 2026-10-04 Troubleshooting idea 1: read Direct3D 9's blending settings from the module, `gmsky.D3DState` (not tested in game)

**Changed:** new `gmod/module/src/d3d.rs`; `lib.rs` registers `gmsky.D3DState()`; `overlay2d.lua` calls it at `PreDrawHUD`, in `HUDPaint` (after the first patch, at the end), in the Derma panel's paint, and in `DrawOverlay`; harness check (without the game it answers "not found: shaderapidx9.dll is not loaded"); unit test in `d3d.rs`.
**Why:** see directly whether alpha writes (`COLORWRITEENABLE`) and a separate alpha blending rule are on while the 2D parts draw. That decides whether idea 5 (forcing them below VGUI, STATUS's first idea) can work.
**How it finds the device (read only, nothing hooked or changed):** a throwaway null Direct3D device on a hidden window gives the addresses of a device's functions; `shaderapidx9.dll`'s data is scanned (with `ReadProcessMemory`, so bad pointers can't crash) for an object with those functions, confirmed with `QueryInterface`. Re-checked on every call. Logs creation flags, colour write mask, blending on/off and rules, separate alpha blending and rules, alpha test, sRGB write, render target format and size.
**Only with `mat_queue_mode 0`:** Source draws on a second thread by default, so Lua's hooks wouldn't match the real drawing. The test logs "Direct3D reads skipped" otherwise.
**Tested how:** `cargo test --release` (unit test on this PC, without the game): a null device gave flags 0x922 (not pure, not multithreaded) and Direct3D's defaults (colour write RGBA, blending off ONE/ZERO); a real hidden 16 × 16 hardware device has the same functions and gave "render target X8R8G8B8 16x16". Harness PASSED (103). **Not tested in game** (the scan of `shaderapidx9.dll` can only run there).
**Still broken / not tested:** Source applies settings at the next draw, so a read shows what the previous draw used (close, not exact per patch). If the scan finds no device, the log says how many objects it looked at.
**Next:** in-game test 32 (both ideas together).

## 2026-10-05 In-game test 32 (2D alpha diagnostics): forcing alpha writes from Lua works for the HUD and Lua 2D; not for VGUI panels; device not found; tests 28-31's reference was skewed

**Result (logs and pictures: `build/logs-ingame-test-32/`; `python tools\alpha2d_report.py build\logs-ingame-test-32\overlay a2d`; physics gun, 1280×720, `mat_queue_mode 0`):**
- **`plain` (nothing forced):** every patch, the HUD digits and the crosshair have alpha 0 (as tests 30-31).
- **`write` (`render.OverrideAlphaWriteEnable(true, true)` from `PreDrawHUD`, VRMod's way): all 12 Lua 2D patches have exactly the right alpha** (`surface.DrawRect`, `DrawTexturedRect`, `draw.RoundedBox`, `render.DrawQuad`: 1.00 / 0.50 / 0.18). **The engine HUD too:** health digits 0.91, panel background 0.35 (8,258 of 8,262 pixels with alpha); crosshair dots 1.00.
- **`over` (also `OverrideBlend`):** identical to `write`, pixel for pixel. The blend override isn't needed.
- **Derma panel painted by the engine's VGUI pass: alpha 0 in every variant**, like Garry's Mod's own right-side panels (15,920 drawn pixels, alpha 0). **VRMod's way (painted manually into our own render target): exactly right** (1.00 / 0.50 / 0.18).
- **Side effect of forcing:** an 18×18 square (324 px) around the crosshair has alpha 0.13 and no colour. In the 61×61 area around it, 1,029 px read 0.13. Probably something drawn additively (adds no colour but writes alpha). **Not explained yet.**
- **Colours are blended in linear light and stored as sRGB** (gamma-encoded): alpha 0.50 white gives colour 188, not 128; 0.18 gives 118 (both exactly the sRGB curve). **So the black/white matte of tests 28-31 reads partly see-through pixels wrongly:** test 31's forced `alpha2d` pictures have health-panel alpha 0.353; the matte said 0.176, exactly what 0.353 becomes through the sRGB curve. **Test 31's "no fix" for the health panel was a measuring error.** What was really missing there: the VGUI panels (alpha 0), plus the crosshair square above.
- **Idea 1 failed:** `gmsky.D3DState` found no device. Each call: ~640 pointers, 10 objects with a function table in `d3d9.dll`, none with the null device's functions; 27-35 ms per call, searched again on every call (15 times: 3 frames stalled ~150 ms each). Likely cause (**not verified**): Garry's Mod made a Direct3D 9**Ex** device, whose functions differ from the plain null device used as the reference.
**Changed:** nothing (results only). `STATUS.md` updated.
**Next:** proposed to the user: (A) a Lua test forcing alpha writes *inside* the VGUI pass (in a panel's own `Paint`, and from an invisible panel painted first); (B) fixing the device search (a 9Ex reference device, log the 10 objects, don't search again after a failure) only if A fails; (C) the crosshair square; (D) an sRGB-correct matte in `overlay_preview.py`, and remembering linear-light blending for Skyrim's drawing (step 2c).

## 2026-10-05 Own Lua HUD started and cancelled; HOLOHUD 2 chosen instead

**Changed:** a Lua HUD of our own (`gmod/lua/gmsky/hud.lua`: HL2-style health, suit, ammo, crosshair; engine elements hidden) was started after the user said "a full Lua UI", then **cancelled** by the user ("I can just download a UI"). Removed again: `hud.lua`, its `client.lua` line, its harness checks. Never tested in game. One of its harness checks was wrong (`tostring` of an empty `hook.Run` result); it went with the file.
**Why:** the user installed **HOLOHUD 2** (D/GL4, Workshop 3459525275, compass and radar; later Skyrim's enemies and quest markers on the radar).
**Tested how:** read its `.gma` in the session scratchpad (never in the repo or the game). It draws the HUD in `HUDPaintBackground` / `HUDPaint` (forced alpha worked there in test 32); `vgui.Create` only in its settings menu. It renders into its own render targets in `PreDrawHUD`, then draws a translucent background layer and an **additive** glowing layer. hl2hud (Workshop 2954934766) was also downloaded, then removed by the user.
**Next:** test 33: does the additive layer write alpha it shouldn't?

## 2026-10-05 Test 33 built: grey frame to check an additive HUD's alpha (not tested in game)

**Changed:** `overlay2d.lua`: a fourth frame `grey` (alpha forced, cleared to sRGB 124 ≈ 0.2 linear instead of black); option `gmsky_test_alpha2d <name> nopatches` (no test patches or panels, so they don't cover HUD elements). `tools/alpha2d_report.py`: "grey check". From the `write` frame (colour + alpha over black) it predicts the picture over grey in linear light, and compares that with the real `grey` frame. It prints the error in sRGB levels, whether alpha is too high (dark halos in Skyrim) or too low, additive-only pixels, a 3 × 3 screen grid of where errors are, and writes `<name>_grey_check.png` (real | predicted, error × 8 | alpha). `docs/DESIGN.md`: the "full Lua UI" plan replaced by "HUD decision after test 32".
**Tested how:** harness PASSED (103); the grey check run on made-up pictures: a correct translucent panel and a correct additive glow gave error ≤ 0.13 levels mean; a glow that wrongly wrote alpha 0.3 was flagged (8,000 of 8,000 px, "too high", top right). **Not tested in game.**
**Still broken / not tested:** the two frames are consecutive, so HOLOHUD's animations (flicker, scanlines) also show up as error; to judge from the picture.

## 2026-10-05 In-game test 33 (HOLOHUD 2, grey check): its "3D visor" writes alpha 1 over almost the whole screen

**Result (logs and pictures: `build/logs-ingame-test-33/`; `python tools\alpha2d_report.py build\logs-ingame-test-33\overlay hh`; physics gun, `mat_queue_mode -1`, `nopatches`):**
- All 4 frames captured, `HUDPaint` ran in each.
- **In every frame, including `plain` (nothing forced), 862,905 of 921,600 pixels (93.6%) have alpha 255**, with almost no colour (colour > 2 on only 45,787 px). The alpha is identical in all 4 frames. In the picture, the solid area is a curved, screen-sized shape (bulging top and bottom edges): HOLOHUD 2's **"3D visor" effect** (`holohud2_r_3d`, default 4 = high; `render.lua`). It bends the whole HUD onto a curved surface over the screen, and that surface writes alpha 1 by itself, not because of our forcing.
- So the grey check reads "everything wrong" (mean error 120 levels, alpha too high on 860,710 px). Over Skyrim this would cover the whole view with black. **Not usable with the 3D visor on.**
- The HUD itself looks right in colour over black and over grey: radar, compass, clock, HOLOHUD's welcome panel, and Garry's Mod's own hint boxes ("Hold 'C'…").
**Changed:** nothing.
**Next:** test 34, the same with `holohud2_r_3d 0` (flat HUD, drawn with `surface.DrawTexturedRect` instead of the curved surface).

## 2026-10-05 In-game test 34 (HOLOHUD 2, 3D visor off): its flat layer writes alpha 1 everywhere too; the black/grey matte is right

**Result (logs and pictures: `build/logs-ingame-test-34/`; `holohud2_r_3d 0`; the log has 3 runs, the pictures are from the last):**
- The colours are right (user: "the images themselves are perfect").
- **Alpha is 255 on 921,599 of 921,600 pixels in all 4 frames, also `plain`** (nothing forced). HOLOHUD's flat full-screen layers mark the whole screen solid by themselves. So the report's grey check found "no empty pixels". **HOLOHUD's own alpha can't be used, with or without forcing.**
- **Alpha worked out from the black and grey frames (in linear light) is right** (checked by eye on a composite over a sky colour, and by numbers): 80,228 px with alpha > 0.02; HOLOHUD's dark plates 0.37 (median); its glow ~0 (adds light only, as it should); 99th percentile 0.96. Garry's Mod's own hint box (a VGUI panel) comes out right this way too. Unreliable pixels: 4,647 at full brightness in the grey frame (clipped), 4,419 where colour channels disagree (motion or flicker between the two frames).
**Changed:** nothing.
**Next:** a design decision for the user: the black/grey matte (any HUD, any panel) needs two pictures; how to get them (DESIGN.md / chat).

## 2026-10-05 HOLOHUD 2, our patched personal copy (not tested in game)

**Changed:**
- New `gmod/holohud2_patch/render_append.lua` (our code): replacement `HOLOHUD2.render.RenderHUDBackground` / `RenderHUD`. They draw HOLOHUD's own two layers on its own quad or curved "3D visor" polygons, so **the curve is kept**. Only the alpha writes change: the plate layer (`MAT_FRAME`) writes alpha with "over" blending; the glow layer (`MAT_COMPOSITE`, additive) writes none. Afterwards alpha writes go back to what gmsky wants (`gmsky.alphaForced`). With HOLOHUD's post-processing or its 3D debug view on, the originals run.
- New `tools/build_holohud.py`: extracts the user's Workshop `holohud2.gma` (read only) into `build/holohud2_gmsky/` (never tracked; HOLOHUD has no licence file, so none of its files go into git). It checks that `render.lua` still has the 10 things the patch uses, appends the patch, and writes `gmsky_patch.txt`.
- New `tools/install_holohud.ps1` (**for the user to run**: it writes into the game folder): copies that build to `garrysmod\addons\holohud2_gmsky`, replacing only a folder of ours.
- `overlay2d.lua`: sets `gmsky.alphaForced` while it forces alpha. Harness: if the copy is built, its patched `render.lua` must compile in Garry's Mod's Lua (compiled only, not run). `docs/DESIGN.md`: "HOLOHUD 2, patched".
**Why:** tests 33-34: HOLOHUD's own drawing writes alpha 1 everywhere. The user asked for our own copy to patch ("no need to worry about updates"), keeping the curve if possible.
**Tested how:** `python tools\build_holohud.py`: 518 files, patch appended (2,895 bytes); git status shows only our 3 new files, not the copy. Harness PASSED (104). **Not tested in game.**
**Still broken / not tested:** whether HOLOHUD's plate layer texture carries a right alpha (it looked right on screen, matte 0.37 in test 34); whether HOLOHUD's direct-paint elements (`PaintOver`) write a right alpha; Garry's Mod's own hint boxes (VGUI) are still not covered.

## 2026-10-05 In-game test 35 (patched HOLOHUD): patch loads and changes the alpha, but not right yet; diagnostics added for test 36 (not tested in game)

**Result (logs and pictures: `build/logs-ingame-test-35/`; `flat` = `holohud2_r_3d 0`, `curved` = 4; AR2):**
- **The patch is loaded:** in the unforced frame (`plain`) the alpha is now 0 on 817,288 px and 255 on 104,312 (before the patch: 255 everywhere). The solid shapes are exactly HOLOHUD's plates.
- **But the plates get alpha 1 where they are really 0.368 see-through** (from the grey frame: plate colour 100 over grey 124, black over black). So the alpha written for the plate layer isn't the see-through amount its colour is blended with.
- **In the frames where alpha is forced (`write`, `grey`), something again writes alpha 1 over the whole screen** (flat: 921,599 px; curved: the visor shape, 862,905 px). Not the damage-indicator overlay (it only draws while hurt). Not found by reading the code.
- Colours unchanged and right in all frames.
**Changed (for test 36, diagnostics only):** the patch saves pictures while `gmsky.holohudProbe` is set: HOLOHUD's two layer textures (`_rtframe`, `_rtcomposite`) and the screen before the plates, after the plates, before and after the glow layer, after its direct-paint elements (`_1before` … `_5afterpaintover`). `overlay2d.lua` sets it in the `plain` and `write` frames. Copy rebuilt (patch 4,034 bytes); harness PASSED (104). **Not tested in game.**
**Next:** one run; the pictures show which step writes the whole-screen alpha and what alpha the plate texture really holds. This is the second attempt at the HOLOHUD alpha. If the next fix fails too, stop and write STATUS.md (AGENTS.md).

## 2026-10-05 In-game test 36 (HOLOHUD diagnostics): the glow layer causes the full-screen alpha; the plates lose their 94; glow fix + plate experiment built (not tested in game)

**Result (pictures: `build/logs-ingame-test-36/overlay/probe_*`; flat, AR2):**
- Screen alpha, step by step (unforced / forced frame): before HOLOHUD 0 everywhere / same; after the plates 255 on 104,312 px (the plates), 0 elsewhere / same; **after the glow layer unchanged / 255 on 921,599 px**. HOLOHUD's glow texture (`RT_COMPOSITE`) has alpha 255 on 921,599 px. So **the glow layer writes the full-screen alpha whenever alpha writes are forced**. The patch's `render.OverrideAlphaWriteEnable(true, false)` doesn't stop it once gmsky has forced writes on. With the override off (unforced frame) it writes none.
- **Plates:** HOLOHUD's plate texture (`RT_FRAME`) is (0, 0, 0, 255) on every plate pixel. On screen every plate is exactly 0.368 = 94/255 see-through (test 35 grey frame: 1st-99th percentile all 0.368). That is HOLOHUD's plate colour setting `Color(0, 0, 0, 94)`. So the 94 is applied while drawing, but never reaches the alpha channel, which ends up at 255. **Why: not known.**
- The user: the pictures' colours look "genuinely perfect" (they are; the problem is only the alpha channel).
**Changed:** glow layer drawn with the alpha override **off** (`OverrideAlphaWriteEnable(false)`), then forcing restored for HOLOHUD's direct-paint elements. Plate experiment (diagnostics, once in the `plain` frame): the plate layer drawn into our own RGBA8888 picture with 3 blend set-ups (`over` = the patch's, `material` = the material's own, `holohud` = HOLOHUD's original), each over black and over grey: `<name>_plain_plate_<setup>_<black|grey>.png`. Copy rebuilt (patch 5,710 bytes); harness PASSED (104). **Not tested in game.**
**Next:** test 37. The glow fix is the second fix attempt for the HOLOHUD alpha. If it fails, or the plate experiment shows no set-up writing 0.369, stop and write STATUS.md.

## 2026-10-05 In-game test 37 (glow fix, plate experiment): both failed; stopped and rewrote STATUS.md

**Result (pictures: `build/logs-ingame-test-37/overlay/probe_*`; flat, AR2):**
- **Glow fix failed:** with the alpha override off (`OverrideAlphaWriteEnable(false)`), the glow layer now makes 921,599 px alpha 255 in **both** frames (test 36: only in the forced one). The override's effect depends on what was set before, in a way not understood.
- **Plate experiment:** drawn into our own RGBA8888 picture, HOLOHUD's plate layer is **fully opaque** with all 3 set-ups (`over`, `material`, `holohud`): written alpha 1.000 on all 104,312 plate px, 0 elsewhere; plate colour 0 over grey 124 (implied alpha 1.000). On the real screen the same plates are 0.368 see-through (test 35). **Where the 0.368 comes from is not known.**
**Changed:** nothing in code. `STATUS.md` rewritten ("The current problem", evidence table for tests 33-37, everything tried, ideas not tried yet).
**Why stop:** second failed fix attempt on HOLOHUD's alpha (tests 35, 37). AGENTS.md: stop and write STATUS.md instead of trying variations.
**Next:** the user decides. Recommended: alpha from a black and a grey background on alternating frames (right on HOLOHUD in test 34, independent of how any addon draws).

## 2026-10-05 HOLOHUD's plates: the see-through look is a blur trick; blur turned off (not tested in game)

**Found (by reading HOLOHUD's code, no game run):** the plates are not transparent. `components/panel.lua` `PaintFrame` first pastes a blurred copy of the screen, fully opaque (`modules/render.lua` `BlurRect`, material `pp/blurscreen`, snapshot taken by `ComputeBlur` earlier in the frame), then draws `Color(0, 0, 0, 94)` on top. That explains tests 35-37: the plate layer holds alpha 255 because the blur patch is solid; on the real screen over flat grey it measured 0.368 because a blur of flat grey is the same grey; in our own picture it was opaque because the snapshot wasn't our background. A blur of Skyrim's world can't come out of Garry's Mod, so it can't be in our picture.
**Also checked:** VRMod's HUD (semi-official fork `Pescorr/Gmod-VRMod-Semioffcial`, `lua/vrmodunoffcial/vrmod_hud.lua`; the original repo is gone) captures with `PushRenderTarget`, `OverrideAlphaWriteEnable(true, true)`, `Clear(0,0,0,0)`, `render.RenderHUD`, from its RenderScene-time hook. Same idea as our test 32; it does nothing about HOLOHUD's blur either.
**Changed:** `gmod/holohud2_patch/render_append.lua`: runs `holohud2_r_blur 0` (HOLOHUD's own "Blurs the panels' backgrounds" setting) when the patched copy loads. The setting is saved, so it stays off for HOLOHUD outside this project too until set back to 1. Copy rebuilt (`python tools\build_holohud.py`, patch 6,318 bytes); harness PASSED (it doesn't run HOLOHUD, so it says nothing about this).
**Still broken / not tested:** not tested in game. Expected plate alpha 0.369 (94/255); it may come out about 0.136 (0.369 x 0.369) if HOLOHUD's drawing into its plate layer squares the alpha. The glow layer's full-screen alpha 1 (tests 36-37) is not touched by this.
**Next:** test 38, the same alpha test as 35-37, and read the plate alpha.

## 2026-10-05 In-game test 38 (blur off): plates fixed, glow still alpha 1 everywhere

**Tested how:** user installed the rebuilt copy and ran `gmsky_test_alpha2d flat` (flat visor, AR2); logs in `build/logs-ingame-test-38/`. Measured HOLOHUD's step pictures directly (the report's test patches are hidden under the glow's full-screen alpha).
**Result:**
- **Plates right:** plate layer (`flat_plain_rtframe`) alpha **94** on all 68,244 plate px, colour 0. Screen after the plates (`_2afterframe`), plain and forced frames: alpha 94 on the same 68,244 px, 0 elsewhere. In our own picture over grey 124: colour 100, alpha 94 (= black at 94/255 over grey in linear light). Test 36-37 with blur: 255. So the 255 was the blur patch. (Fewer plate px than test 35's 104,312: not checked why; maybe panels shown differ.)
- Since the patch's "over" draw wrote 94 and not 94 x 94 / 255 = 35, Garry's Mod's 7-argument `render.OverrideBlend` (separate alpha factors) does take effect here.
- **Glow still wrong:** after the glow layer alpha 255 on 921,599 px in both frames; `RT_COMPOSITE` itself has alpha 255 everywhere except the one bottom-left pixel (719, 0). That matches HOLOHUD's chromatic-aberration quads drawn 1 px off in each direction, so those quads are what fill the glow layer's alpha (not proven). 95.5% of that layer's pixels are black (colour ≤ 2).
**Still broken / not tested:** glow alpha. Two fixes of the glow's alpha write switch failed (tests 36, 37).
**Next:** user decides: one try with a different mechanism (draw the glow with separate alpha factors ZERO, ONE = leave the picture's alpha as it is; shown to work for the plates), or switch to another HUD.

## 2026-10-05 Glow drawn with "keep alpha" blending; post blur planned (not tested in game)

**Changed:** `gmod/holohud2_patch/render_append.lua` `RenderHUD`: the glow layer is drawn with alpha writes as gmsky wants them and `render.OverrideBlend(true, ONE, ONE, ADD, ZERO, ONE, ADD)`: colour added (as HOLOHUD's `$additive` material), alpha = what was already there. Third and last try at the glow (user's go-ahead: if it fails, switch HUD). Copy rebuilt (patch 6,702 bytes); harness PASSED (doesn't run HOLOHUD). `docs/DESIGN.md`: plan for blurring Skyrim behind the plates in Skyrim's Present hook, from a plate mask sent with the picture (after steps 2b and 2c exist).
**Why:** test 38: glow layer texture alpha 255 everywhere; the separate alpha factors were shown to work for the plates.
**Still broken / not tested:** not tested in game. Curved visor (`holohud2_r_3d 4`) not tested with either fix. Post blur: plan only.
**Next:** test 39: after the glow, screen alpha should stay 94 on plates and 0 elsewhere except where HOLOHUD's direct-paint elements draw.

## 2026-10-05 In-game test 39 (glow "keep alpha"): HOLOHUD's alpha is right, flat and curved

**Tested how:** user installed the rebuilt copy; `gmsky_test_alpha2d flat` (`holohud2_r_3d 0`) and `gmsky_test_alpha2d curved` (`holohud2_r_3d 4`), AR2. Logs: `build/logs-ingame-test-39/`. Measured HOLOHUD's step pictures and ran `tools/alpha2d_report.py`.
**Result:**
- **The glow no longer touches alpha.** Screen alpha after the plates and after the glow are identical in both frames (plain and forced): flat 817,288 px alpha 0, 104,312 px alpha 94, none 255 (test 38: 921,599 px 255 after the glow). Curved: 819,556 px 0, 98,827 px 94, 3,217 px in between (the visor polygons' edges). The glow adds colour with alpha 0 (flat 2,188 px, curved 1,693 px outside the plates).
- **Grey check, curved** (our one picture predicting Garry's Mod's real frame over grey): mean error 1.63 levels, 95% under 3.4 levels (0-255).
- **Grey check, flat:** mean 3.51, 95% 43.4; the error sits bottom-right (7,692 px). In the same area 3,427 px change colour between the two consecutive frames themselves (plain → write), so most likely a HUD animation between frames, not an alpha error (not proven; the curved run had 0 there).
- Test patches over a plate read 0.69 = 0.5 + 0.5 × 0.37, which is right for a 50% patch over a 37% plate.
**Still broken / not tested:** HUD not yet sent to Skyrim (2b, 2c). Blur behind the plates: planned in Skyrim (DESIGN.md), not built. HOLOHUD's saved `holohud2_r_blur` stays 0.
**Next:** step 2b (picture to shared memory), then 2c (Skyrim draws it), then the post blur.

## 2026-10-05 Step 2b planned; 2b-1: find Garry's Mod's Direct3D device and hook its Present (count only)

**Changed:** `docs/DESIGN.md`: "Step 2b in detail" (sub-steps 2b-1 to 2b-4). The picture will be copied in a hook on the device's `Present` (runs on the thread that really draws, once per finished frame, whatever `mat_queue_mode` is), not from a Lua hook (with the default `mat_queue_mode -1` Lua runs before the real drawing, on another thread). `gmod/module/src/d3d.rs`: the device search also makes a throwaway Direct3D 9Ex device as a second reference, matches candidates on `QueryInterface` only (test 32 also required `GetRenderState` to match), logs up to 16 candidates when it fails, and doesn't search again for 10 s after a failure (test 32: 15 searches, ~150 ms stalls). New `gmod/module/src/present.rs`: replaces `Present` (and `PresentEx` on an Ex device) in the device's function table; it only counts: frames, drawing thread vs Lua's thread, back buffer format / size / multisampling, hook cost. Lua functions `gmsky.OverlayHook()`, `gmsky.OverlayStatus()`. New `gmod/lua/gmsky/liveoverlay.lua` (included by `client.lua`): console command `gmsky_overlay_probe`, status line every 5 s. Harness: fakes `ScrW`/`ScrH`, 3 new checks.
**Why:** step 2b needs Garry's Mod's device (to copy the picture) and the right moment to copy it. Both were unknown.
**Tested how:** `cargo test --release` (module: 5 tests): on this PC, a plain and an Ex device are made on a hidden 16 × 16 window; the hook is installed on each, `Present` / `PresentEx` called through the function table, counted, the back buffer read (X8R8G8B8 16 × 16), the original run (it returns "occluded", a success code, because the window is hidden), and the table put back. `target\release\gmod_harness.exe`: HARNESS PASSED (107 checks; without the game the probe says "device not found: shaderapidx9.dll is not loaded" and the status stays nil).
**Result:** on this PC **an Ex device has the same `QueryInterface` as a plain one** (both `d3d9+0x5e770`). So test 32's guess ("probably 9Ex") does not explain its failure by itself; its stricter `GetRenderState` match may have been what failed. Not known until the in-game test logs the candidates.
**Still broken / not tested:** **not tested in game.** Nothing is copied yet (2b-2, 2b-3). Unknowns the probe answers: is the device found; does Source call `Present` on the device (it might present through a swap chain object, then the counts stay 0); the back buffer's format (`X8R8G8B8` would mean no alpha in it).
**Next:** in-game test 40: `gmsky_overlay_probe`, read `present hook:` lines. Then 2b-2 (picture block + triple buffer).
## 2026-10-05 In-game test 40 (`gmsky_overlay_probe`): device still not found (second failed attempt)

**Tested how:** user ran `gmsky_overlay_probe` in Garry's Mod (gm_flatmap, 1280×720, `mat_queue_mode -1`). Log: `logs/gmod_client.log`.
**Result:** `hook NOT installed`. 640 pointers in shaderapidx9.dll's data tried, 10 point at objects with a function table in d3d9.dll, **none of them has a device's `QueryInterface`** (`d3d9+0x5e770`, the same for plain and Ex devices), so none was asked. Their `QueryInterface`s: `+0xf1100`, `+0x1145a0`, `+0x118f50` (×2), `+0x11d470` (×2), `+0xeb650`, `+0x5e540` (×2), `+0x114490`: 7 different kinds of object. Search took 61.6 ms, once (no repeats).
**What this shows:** shaderapidx9.dll's data holds **no direct pointer to the device**. The 10 objects are most likely other Direct3D objects Source keeps (textures, surfaces, queries; `+0x5e540`, next to the device's `+0x5e770`, is probably the `IDirect3D9` factory object). So test 32's failure wasn't the 9Ex question. Either the device pointer is kept one step further away (inside an object on the heap), or it doesn't point into d3d9.dll at all (some layer wrapping the device). Not known which. The "119 functions in d3d9" count isn't a useful filter: it reads past the end of short function tables into the next ones.
**Still broken / not tested:** step 2b-1 blocked: no device, no hook. Nothing crashed.
**Next:** stopped after two failed attempts (AGENTS.md); `STATUS.md` updated with the ideas; the user chooses.
## 2026-10-05 Device search, third try (user chose "ask an object"): the cause of tests 32 and 40 found offline

**Changed:** `gmod/module/src/d3d.rs`: (b) the user's choice: if no direct pointer to the device is found, each Direct3D object Source keeps (up to 16) is asked whether it is a resource (texture, surface, buffer: `QueryInterface`), and if so for the device that made it (`GetDevice`); the device it names is checked (`QueryInterface` for device / Ex) and used. All references taken are given back. (a) the direct scan also accepts a function table outside `d3d9.dll` if its `QueryInterface` is a real device's. The log names the DLL holding the device's table and its `Present`. `present.rs`, `docs/DESIGN.md`: comments corrected.
**Why:** test 40, second failed attempt; user chose idea 1 from `STATUS.md`.
**Tested how:** `cargo test --release` (module: 6 tests). New test on real hidden plain and Ex devices: a texture's `GetDevice` gives back exactly that device; the `IDirect3D9` factory (not a resource) answers "no" without harm. Harness PASSED (107 checks).
**Result:** the new test printed **`device table 0x253043e0af0 (in no DLL), Present d3d9.dll+0x61c40`**: on this PC, `d3d9.dll` gives **each device its own copy of its function table in ordinary memory**. Tests 32 and 40 only accepted objects whose table is inside `d3d9.dll`, so they skipped the device. That explains both failures (not yet confirmed in game). It also means the `Present` hook patches only Garry's Mod's device's own table.
**Still broken / not tested:** **not tested in game.** If route (a) finds the device, (b) isn't used.
**Next:** in-game test 41: `gmsky_overlay_probe` again.
## 2026-10-05 In-game test 41 (`gmsky_overlay_probe`): device found, Present hook works (step 2b-1 done)

**Tested how:** user ran `gmsky_overlay_probe` in Garry's Mod (gm_flatmap, 1280×720, `mat_queue_mode -1`), played ~45 s, quit normally. Logs: `build/logs-ingame-test-41/`.
**Result:**
- **Device found** by the fixed direct scan (route a; the `GetDevice` route wasn't needed): `shaderapidx9+0xa7d10`, its own function table in ordinary memory (as the unit test predicted), `Present` = `d3d9.dll+0x61c40`. 436 pointers tried, 65.3 ms, once. So tests 32 and 40 failed only because they required the table to be inside `d3d9.dll`.
- **Ex device: yes.** Creation flags `0x56`: hardware vertex processing, **pure** (Direct3D won't report its settings, so `gmsky.D3DState`'s reads can't work on it), **multithreaded** (safe to call from other threads).
- **The hook counts every frame:** `Present` calls per 5 s = the client's frame count exactly (1212 / 1212, 1059 / 1059, 978 / 978, 1080 / 1080, 1269 / 1269, 1121 / 1121; the first reading covers 6 s). `PresentEx` 0: Source uses plain `Present`. 0 presents from other devices.
- **Drawing thread = Lua's thread** (26144) even with `mat_queue_mode -1`: on this PC Source draws on the main thread, so Lua's hooks run at the real drawing time. (Why: not checked; the hook works either way.)
- **Back buffer `A8R8G8B8` 1280×720, no multisampling:** it has an alpha channel, and `GetRenderTargetData` / `StretchRect` need no resolve step. 0 read failures.
- Hook cost 0.001 ms per frame on average, 0.052 ms at most. No crash (newest crash dump is from 2026-10-04).
**Still broken / not tested:** nothing copied yet. Device reset / resolution change with the hook installed not tested.
**Next:** step 2b-2 (picture block + triple buffer, no game needed).
## 2026-10-05 Step 2b-2: the overlay picture block and its triple buffer (no game needed)

**Changed:** `protocol/gmsky_protocol.h`: "Overlay picture block": a second shared memory block `Local\GmodSkyrim_overlay_v1` (own magic `MSOV`, version 1; the link block and `GMSKY_VERSION` 9 unchanged), 101,257,216 bytes: control part (triple-buffer word, frames published / taken, Skyrim's size), three slot headers (size, format BGRA8, flags premultiplied / sRGB, frame, capture time, row bytes, plate-mask size), three slots of 3840×2160×4 bytes + room for a quarter-size plate mask. Windows only uses memory for pages actually written. C++ helpers `gmsky::ov` (attach, publish, take, detach; Skyrim will use take). New `protocol/src/overlay.rs`: the Rust copy (same layout checks) and `OverlayMapping`; `shm.rs`: `map_named` / `unmap`. `protocol/test/protocol_test.cpp`: triple-buffer rules. New `tools/fakes/src/fake_overlay.rs` (plays Garry's Mod's picture writer: a test picture 60 times a second). New `tools/overlay_read.py` (looks into the block without taking Skyrim's place; saves the newest picture).
**Different from SkyCraft (on purpose):** SkyCraft keeps the middle slot in a shared word but each side's own slot in its own memory, so a restarted Skyrim resets the word and can end up sharing a slot with the writer. Here all three slot numbers live in one 32-bit word changed only by compare-and-swap, so a restarted or crashed side gets its old slot back (or the free one) and the slots never overlap.
**Tested how:** `cargo test --release` (protocol: 8 tests). (1) Every sequence of attach / publish / take / restart up to 7 steps from an empty block: the three slots are always 0, 1, 2 in some order, and a take always gets the newest picture. (2) A writer thread and a reader thread, 20,000 pictures of changing sizes, each filled with its frame number in two halves: **7,163 taken, every one whole**, frames only ever newer, the last one taken is the last one written. **This test was checked to fail** with a deliberately broken rule (writer keeps writing into the slot it just published: "picture 1 not whole" at once); rule restored. (3) Restarts: Garry's Mod quits and comes back while Skyrim holds a slot; a block of another version is refused. C++: `build\protocol\Release\protocol_test.exe` PROTOCOL TEST PASSED (same rule sequence, same numbers). Cross-check: `fake_overlay` (Rust) + `tools\overlay_read.py` (Python): 55 pictures/s seen, newest frame 190, 1280×720, 3.6 ms old, 0 retries; alpha 7,424 px 255 (80×80 + 32×32), 12,800 px between (two 80×80), 6,400 px colour with alpha 0 (the "glow" square): exactly the test picture. Harness PASSED.
**Still broken / not tested:** nothing in either game uses the block yet (2b-3: Garry's Mod writes; 2c: Skyrim reads). The block costs ~100 MB of the page file's commit limit while open (pages used only when written: ~3.7 MB per slot at 1280×720).
**Next:** step 2b-3: the `Present` hook copies each frame into the block; Lua's live overlay mode (`gmsky_overlay 1`).
## 2026-10-05 Step 2b-3: every prepared frame copied into the overlay block (`gmsky_overlay 1`)

**Changed:** new `gmod/module/src/capture.rs`, called from the `Present` hook: in frame N the back buffer is copied on the graphics card into one of three pictures of our own (`StretchRect`) and an event query is issued; in a later frame, when the card says it is done, it comes to normal memory (`GetRenderTargetData` into a system-memory surface), is copied into Garry's Mod's slot of the overlay block and published (only the newest finished copy; older ones dropped). Only frames Lua marked are copied; only `A8R8G8B8` back buffers (otherwise an error is logged: no alpha to send). `present.rs` also hooks `Reset` / `ResetEx`: our pictures are released before a reset and made again at the new size. If the device changes, old pictures are forgotten, not released. Lua functions `gmsky.OverlayEnable(on)`, `gmsky.OverlayPrepared()`. `liveoverlay.lua`: console variable `gmsky_overlay` (client, off at every start): each frame cleared to transparent at the first `PreDrawViewModel` / `PreDrawPlayerHands` (or at `PreDrawHUD` without a view model), depth-in-alpha off, alpha writes forced from `PreDrawHUD` (+ `gmsky.alphaForced` for the HOLOHUD patch), all set back in `PostRender`, frame marked. Logs once per switch-on where it cleared and the order of `PostRender` / `DrawOverlay`. The 5 s status line now also has the copy numbers. `docs/DESIGN.md`: these details, written before building. Harness: fake `render` library; 2 new checks (109).
**Tested how:** `cargo test --release`. New module test on real hidden plain and Ex devices with a 64×48 `A8R8G8B8` back buffer filled with a known half-transparent colour (`0x80FF2010`, `0x40102030`): a frame not prepared is not copied; then the picture arrives in the block, read as Skyrim would (take), **every pixel exactly the colour**, header 64×48, 256 bytes per row, BGRA8, premultiplied + sRGB; 1 frame from copy to publish; start 0.2 ms, finish 0.47 ms; 0 failures. A reset releases our pictures and the next frame makes them again. Harness PASSED (109): with `gmsky_overlay 1`, each frame exactly `depthAlpha(false) Clear(0,0,0,0) alphaWrite(true,true) … depthAlpha(true) alphaWrite(false)`, with or without a view model; with 0, no render calls.
**Still broken / not tested:** **not tested in game.** Copy cost at 1280×720 unknown (expected 1-3 ms on the drawing thread). Order of `PostRender` and `DrawOverlay` in Garry's Mod not known (logged). Resolution change while copying not tested. Skyrim doesn't read the block yet (2c), and doesn't publish its size yet (2b-4).
**Next:** in-game test 42: `gmsky_overlay 1`, then `python tools\overlay_read.py 2` while Garry's Mod runs.
## 2026-10-05 In-game test 42 (`gmsky_overlay 1`): every frame reaches the overlay block (step 2b-3 works); logs now saved by Claude

**Tested how:** user ran `gmsky_overlay 1` in Garry's Mod (gm_flatmap, 1280×720, `mat_queue_mode -1`, RPG), `python tools\overlay_read.py 2` meanwhile, then `gmsky_overlay 0`. Logs saved by Claude: `build/logs-ingame-test-42/`.
**Result:**
- Device found and hooked again (`Present`, `Reset`, `PresentEx`, `ResetEx`). First frame cleared at the view model; **hook order `PostRender`, then `DrawOverlay`** (so `DrawOverlay`'s panels are drawn after our `PostRender`, but before `Present`: they are in the picture).
- **Every frame published:** e.g. 918 / 918, 976 / 976, 1025 / 1025, 1100 / 1100, 1125 / 1125 per 5 s; 0 not prepared, 0 skipped, 0 dropped, **0 failures**; 1.00 frame from copy to publish; picture made once (no resets).
- **Cost:** copy start 0.007 ms, finish 1.07-1.19 ms per frame on average (max 13.0 ms once, in the first 5 s; otherwise ≤ 4.3 ms). Frame rate ~185-225 fps with the overlay on, ~285-290 after switching it off (2210 frames in 7.6 s).
- **Picture** (`overlay/live_4950.png`): hands and RPG solid (143,800 px alpha 255), HOLOHUD's plates at alpha 94 (25,079 px, as tests 38-39), its glow as colour at alpha 0 (1,232 px), 751,743 px transparent: the world is gone. The tool hint ("Press and hold 'Q'…") is in it.
**Changed:** new `tools/save_logs.ps1`: copies `logs\` to `build\logs-ingame-test-<next N>` (user's request: Claude saves the logs after each test, the user no longer copies them). `tools/launch_gmod.ps1` saves the last run's logs first if they weren't saved (safety net). `liveoverlay.lua`: no "live overlay OFF" line at start-up. Harness PASSED (109).
**Still broken / not tested:** Skyrim doesn't read the block yet (2c) or publish its size (2b-4). Resolution change while copying not tested.
**Next:** step 2b-4 (Skyrim's window size), then 2c.
## 2026-10-05 Step 2b-4: Skyrim publishes its back buffer size into the overlay block

**Changed:** new `skse/src/Overlay.cpp` / `.h`, called every frame from `Probe.cpp`: opens the overlay picture block (same open rules as `overlay.rs`; retried every 2 s), reads Skyrim's back buffer size from its swap chain (`Renderer::GetCurrentRenderWindow()->swapChain->GetDesc`, the size SkyCraft's Present hook reads) and writes it with the time into the block. Once a second `skyrim.log` gets: that size, Skyrim's screen setting (`Renderer::GetScreenSize`), and Garry's Mod's newest picture as Skyrim sees it (frame, size, age in ms, writer attached; only the middle slot's header is read, nothing taken: that is 2c). `gmod/module/src/capture.rs`: the status line says whether Skyrim's size equals Garry's Mod's ("= ours" / "DIFFERS … set Garry's Mod's resolution to WxH" / "written N s ago: Skyrim not running?"). `tools/launch_gmod.ps1`: `-Width` / `-Height` (default 1280 × 720 as before). Skyrim plugin rebuilt and copied into `Data\SKSE\Plugins` (no compiler warnings).
**Tested how:** `cargo test --release` (module: 8 tests; new: the size line for unknown / equal / different / stale sizes). Harness PASSED (109). Plugin build: compiles; **not run in Skyrim**.
**Still broken / not tested:** **not tested in game** (needs both games). Garry's Mod's window size isn't changed automatically (left to 2d).
**Next:** in-game test 43 with both games.
## 2026-10-05 In-game test 43 (both games, `gmsky_overlay 1`): Skyrim sees Garry's Mod's pictures; sizes match (1280×720)

**Tested how:** user started Garry's Mod (`launch_gmod.ps1`, 1280×720), Skyrim (SKSE) with a save, `gmsky_overlay 1`, played ~1.5 min, quit both. Logs saved by Claude: `build/logs-ingame-test-43/`.
**Result:**
- **Skyrim side (step 2b-4) works:** `overlay block opened (already created by Garry's Mod)`; Skyrim's back buffer **1280×720** (from its swap chain), the same as its screen setting, 1 change (0×0 → 1280×720). 90 once-a-second lines: Garry's Mod's newest picture as Skyrim sees it, frame number = pictures published (2857, 3036, 3201, 3381, …: ~180 per second), **6.1-8.7 ms old**, writer attached. After Garry's Mod stopped, the age grows by 1 s per line (4078, 5092, 6108 ms): Skyrim notices a stopped writer.
- **Garry's Mod side:** every frame published (761/762 first, then e.g. 1034/1034, 976/976, 882/882, 0 failures), copy 1.3-1.75 ms per frame (a bit more than test 42's 1.1 with Skyrim running too); ~160-210 pictures/s. Skyrim's size arrives in the block: `(1280, 720)` from the third status line on.
- **Failure (mine):** Garry's Mod ran the **previous module build** (DLL from 02:50; the size comparison was added at 03:00). After the change I ran `cargo test` (builds only test programs) and the harness, not `cargo build --release`. So the new "= ours / DIFFERS" line was not in this test. Rebuilt now (03:04; the DLL contains it; harness PASSED). The numbers above are unaffected: Skyrim's size was visible in the old line.
**Still broken / not tested:** the "= ours / DIFFERS" status text not seen in game yet (rebuilt; will show in the next test).
**Next:** step 2c: Skyrim draws the picture.
## 2026-10-05 Step 2c-1 built: Skyrim draws Garry's Mod's picture (Present hook), in linear light; snapshot check

**Changed:** `docs/DESIGN.md`: "Step 2c in detail" (2c-1 draw, 2c-2 hide Skyrim's arms), written first. New `skse/src/Draw.cpp` / `.h` (adapted from SkyCraft's `Overlay.cpp`, MIT): replaces entry 8 (`Present`) of Skyrim's swap chain function table at `kDataLoaded`; in it: take the newest picture (Skyrim attaches as the block's reader), copy it into a texture (`Map` `WRITE_DISCARD`), draw one full-screen triangle, premultiplied (ONE, INV_SRC_ALPHA; Skyrim's alpha kept), every setting saved and put back. **Different from SkyCraft:** blended in linear light (texture `B8G8R8A8_UNORM_SRGB`, back buffer drawn through an sRGB view; falls back to SkyCraft's sRGB-number blending, logged, if the view can't be made); not drawn while a Skyrim menu is open, while the puppet is off, or when the newest picture is more than 0.5 s old. Once-a-second log: presents, taken, drawn, not drawn by reason, upload / draw ms, picture age, blending mode, back buffer format, Present thread vs game thread. `skse/src/Overlay.cpp`: reader side (`Take`, `Header`, `Pixels`). `skse/CMakeLists.txt`: links `d3d11`, `d3dcompiler`, `dxgi`. **Snapshot check:** `gmsky_overlay_shot` (Garry's Mod console, `gmsky.OverlayShot`) adds 1 to a new field `shot_request` in the block's header (was `reserved0`: same layout, version unchanged); Skyrim then saves its next drawn frame before and after drawing, and the picture, to `logs/overlay/skyrim_shot_<n>_*.bin`. New `tools/skyrim_shot_report.py` predicts "after" from "before" + picture, linearly and SkyCraft's way, and gives the error per region (opaque, see-through, glow, empty). Harness: 1 new check (110).
**Tested how:** both builds: no warnings; `cargo test --release` (module 8, protocol 8), harness PASSED (110), PROTOCOL TEST PASSED. The report on made-up files blended linearly: linear prediction error ≤ 1 level (rounding) everywhere; SkyCraft's way would be 37-41 levels off on average in see-through and glow parts; 0 changed pixels where the picture is empty. **The drawing itself: not run** (needs both games).
**Still broken / not tested:** not tested in game. Skyrim's own first-person arms still drawn (2c-2). A snapshot requested before Skyrim's first drawn frame is ignored.
**Next:** in-game test 44 (both games).
## 2026-10-05 In-game test 44 (both games): Skyrim draws Garry's Mod's picture in linear light; the physgun blacks out the screen (halo)

**Tested how:** user ran both games, `gmsky_overlay 1`, played in Skyrim's window, menus, weapons, `gmsky_overlay_shot` (typed in Garry's Mod's console). User: "Flawless except using the physgun turns my entire screen black." Logs saved by Claude: `build/logs-ingame-test-44/`. `python tools\skyrim_shot_report.py build\logs-ingame-test-44\overlay 1`.
**Result:**
- `Present hooked`; `renderer ready`; Skyrim's back buffer 1280×720 format 28 (`R8G8B8A8_UNORM`): **sRGB view made, blending in linear light**. Present runs on the game thread (24992 = 24992).
- Once a second: ~56-61 presents, every new picture taken and drawn; upload 0.28 ms average (2.9 ms max, first frame); draw 0.01 ms; **picture age 7-9 ms average** (max ~17); "not drawn: menu" counted while Skyrim menus were open (e.g. 57 of 113 presents in one second), 0 stale, 0 errors.
- **Snapshot 1 (by numbers):** Skyrim's real frame = the linear prediction: error 0 levels on opaque pixels, ≤ 1 (mean 0.22) on 420,973 "glow" pixels (colour with alpha 0), 0 on empty pixels (none changed). SkyCraft's sRGB-number blending would have been 31 levels off on average there (max 80). The picture happened to be Garry's Mod's **main menu and console** (the command was typed there, so they were open): Garry's Mod's menus and console are in the overlay and appear in Skyrim.
- **Physgun → whole Skyrim screen black.** Cause (read in Garry's Mod's own `lua/includes/modules/halo.lua`, read only, not copied): Sandbox draws a halo around a physgun-held prop (`physgun_halo`, default 1; also freeze / unfreeze effects and Sandbox entity outlines). `halo.Render` clears the whole screen to black with **alpha 255**, then pastes the scene back: afterwards the frame is opaque everywhere (black, hands on top). Not measured in this test (no snapshot while holding a prop).
**Changed (fix, not tested in game):** `liveoverlay.lua`: while the overlay is on, `halo.Render` is wrapped: before it the frame (with alpha) is copied into an RGBA picture of ours (`render.CopyRenderTargetToTexture`), after it only the alpha is written back from that copy (blend: colour ZERO, ONE; alpha ONE, ZERO), so the halo glow becomes added light (alpha 0). `gmsky_overlay_halofix 0` turns it off to compare; halos counted in the 5 s status line. `gmsky_overlay_shot [seconds]`: delayed snapshot (to close the console and hold a prop first). `capture.rs`: share of fully opaque pixels in each published picture (every 8th pixel each way), average and max in the status line. Tests: module 9 (new: opaque count; the capture test checks 0% opaque), harness PASSED (111; new: the halo wrapper's call order, and halofix 0 / overlay off leave halos untouched).
**Still broken / not tested:** the halo fix in game. Garry's Mod's menus / console in Skyrim (keep or hide: for the user to decide, step 2d). Skyrim's own arms (2c-2).
**Next:** in-game test 45: hold a prop with the physgun.
## 2026-10-05 In-game test 45 (physgun halo fix): no black screen; Garry's Mod's menus and console stay in Skyrim (user's choice)

**Tested how:** user held props with the physgun in Skyrim's window, with `gmsky_overlay_halofix 1` and `0`. User: "personally tested it, flawless". Logs: `build/logs-ingame-test-45/` (saved before Claude's save; Claude's duplicate copy `-46` removed).
**Result (5 s status lines, `gmod_client.log`):** halo fix on, halos drawn (4,042 halos kept in 8 lines): pictures at most **10% opaque** (8.7% average), keeping the alpha cost 0.21 ms per halo. `halofix 0` (390 halos): up to **100% opaque**, the black screen: so the halo was the cause. Without halos: up to 8.7% opaque.
**Decided (user):** keep Garry's Mod's menus and console in the overlay ("I like that"). Nothing to change: they are already drawn (test 44's snapshot).
**Changed:** nothing in code since the fix. `docs/DESIGN.md` / `STATUS.md`: the decision, 2c-1 done.
**Next:** step 2c-2: hide Skyrim's own first-person arms while the overlay is drawn.
## 2026-10-05 Step 2c-2 built: Skyrim's first-person arms hidden while the overlay is live

**Changed:** new `skse/src/Arms.cpp` / `.h` (adapted from SkyCraft's `HideFirstPersonMeshes`, MIT), called every frame from `Probe.cpp` (game thread): while the puppet drives the player and Garry's Mod's newest picture is under 0.5 s old, every mesh of the player's first-person model is marked culled (never the nodes; the model is walked every frame, since Skyrim rebuilds it when a weapon is drawn or changed); otherwise exactly the meshes hidden are shown again (also on game load). Once a second: hidden / shown, meshes hidden now, newly hidden, shown again, changes, last reason. `Overlay.cpp`: `NewestAgeMs()` (age of the newest published picture, from the middle slot's header). `docs/DESIGN.md`: the rule refined before building ("live", not "drawn this frame", so Skyrim's arms don't flash back during Skyrim menus).
**Tested how:** plugin build: no warnings, copied into `Data\SKSE\Plugins`. **Not run in game.**
**Still broken / not tested:** in game (test 46).
**Next:** in-game test 46.
## 2026-10-05 In-game test 46 (Skyrim's arms hidden): works, but Skyrim's weapon flashed for single frames around Skyrim menus

**Tested how:** user played with both games, overlay on. User: Skyrim's weapons "flash on screen for a split second", in odd positions for that frame; normal otherwise. Logs: `build/logs-ingame-test-46/`.
**Result:** hiding works (4 meshes hidden, shown again exactly). But Garry's Mod's overlay was switched on **once and never off**, and Skyrim showed its arms 5 times, each for **one frame** (16-18 ms, e.g. `03:30:15.354` shown, `.370` hidden), reason "Garry's Mod's pictures stopped". Garry's Mod never stopped: 0 stale pictures, picture age ≤ 14 ms in those seconds, publishing steady. **Every flash is in a second when a Skyrim menu opened or closed** (menu counts 61, 40, 37, 38, 13, 2 next to them). The odd positions: probably Skyrim doesn't update hidden meshes, so for that one frame they are drawn where they last were (inferred, not measured).
**Cause, part known:** "newest picture's age" read only the middle slot's header. When Skyrim takes a picture, its previous slot becomes the middle; after a pause in taking (a Skyrim menu) that slot holds a picture from before the pause, so the newest picture looked seconds old until Garry's Mod's next publish. That explains flashes right after a menu closes; the ones just before a menu opens are **not explained**.
**Changed:** `Overlay.cpp` `NewestAgeMs`: the newer of the middle slot and Skyrim's own slot. `Arms.cpp`: the arms are shown again only after 3 frames in a row (~50 ms) say the overlay isn't live; shorter blips are counted and logged ("overlay looked stopped for N frame(s), arms kept hidden") instead of acted on; the age is written into the "shown again" line; once a second, the oldest "newest picture" age seen while live. Plugin rebuilt (no warnings).
**Still broken / not tested:** not tested in game. Why some blips happen just before a menu opens: unknown; the new log lines will show their ages.
**Next:** next play session: check `arms:` lines for blips and ages; then commit 2c-2.
## 2026-10-05 In-game test 47/48 (arms flash fix): no flashes; step 2c done

**Tested how:** user played with both games, overlay on, Skyrim menus opened and closed. User: "Perfect". Logs: `build/logs-ingame-test-48/` (`-47` is a shorter earlier run, saved by the launch script).
**Result:** arms hidden once at 03:36:19 and shown again once at 03:37:59 when Garry's Mod stopped ("newest 538 ms old"); **0 blips** across 127 seconds, 21 of them with a Skyrim menu open. The newest picture's age while live stayed low until shutdown (73.7 ms and 488.1 ms only in the last two seconds, as Garry's Mod closed). The unexplained "just before a menu opens" blips of test 46 did not appear (0 logged), so their cause stays unknown but harmless behind the 3-frame margin.
**Next:** commit; next step: blur behind HOLOHUD's plates, done in Skyrim (DESIGN.md "HOLOHUD 2, patched").
## 2026-10-05 Blur behind see-through parts (HOLOHUD's plates), done in Skyrim: built

**Changed:** `docs/DESIGN.md` "Blur behind the plates": plan revised before building: no separate plate mask; the blur weight comes from the picture's own alpha, w = min(1, a / 0.35) (plates at 94/255 get the full blur; under opaque hands it is covered; side effect: Garry's Mod's console / menu backgrounds are frosted too). `skse/src/Draw.cpp`: before drawing, Skyrim's frame is copied (`CopyResource` into a typeless texture read through an sRGB view: linear light), shrunk to half and quarter size, blurred at quarter size (9-tap Gaussian, horizontal + vertical, twice, 16-bit float); the overlay's pixel shader outputs colour P + (1 − a)·w·B and alpha 1 − (1 − a)(1 − w), so the unchanged premultiplied blend gives P + (1 − a)(w·B + (1 − w)·S). All graphics settings saved and restored as before (now also shader slot 1 and constant buffer 0). Log: frames blurred, CPU ms, state ("asked for" / off / UNAVAILABLE). Protocol: flag `GMSKY_OVF_BLUR` (0x4) in each picture's existing `flags` field (layout and version unchanged). Garry's Mod: `gmsky_overlay_blur` (saved, default 1) → `gmsky.OverlayBlur` → the flag. `tools/skyrim_shot_report.py`: fine-detail check of see-through parts (blurred: ratio well below 1).
**Tested how:** all four shaders compiled offline with Windows' `d3dcompiler_47.dll` and the in-game settings (OK, 700-1,968 bytes each); plugin builds without warnings; module 9 tests (capture test now expects the blur flag), protocol 8, harness PASSED (111), PROTOCOL TEST PASSED; the report's blur check on made-up data: ratio 0.09 for a blurred plate, opaque / empty pixels exact. **The blur itself: not run in game.**
**Still broken / not tested:** in game. Blur strength fixed (quarter size, 2 × 9 taps).
**Next:** in-game test 49.
## 2026-10-05 In-game test 49 (blur behind plates): works

**Tested how:** user played with both games, overlay and blur on. User: "Looks good". Logs: `build/logs-ingame-test-49/`.
**Result:** `blur textures for 1280x720 (quarter size 320x180, 6 passes)`; no errors. 98 one-second lines with the blur asked for: **5,196 frames drawn, 5,196 blurred**; CPU time of the blur passes 0.007 ms average (0.009 max). Graphics card time not measured. No snapshot taken in this run.
**Next:** commit; step 2d (the Garry's Mod window out of the way; which HUD each game shows), decided with the user.
## 2026-10-05 Step 2d built: Skyrim's HUD parts hidden, Garry's Mod's window off-screen (user's choices)

**Decided (user):** while the overlay is live, hide Skyrim's crosshair, health / magicka / stamina, compass, charge meters and arrows; move Garry's Mod's window off-screen. **Future (user):** Garry's Mod's HUD will show stamina / magicka bars, and quest markers, locations and enemies on HOLOHUD's compass and minimap (noted in DESIGN.md "Step 2d in detail"; until then quest markers aren't visible while the overlay is live).
**Changed:** `docs/DESIGN.md` "Step 2d in detail". **2d-1:** new `skse/src/Hud.cpp` / `.h` (adapted from SkyCraft's `HideHud` / `HideCrosshair`, MIT): every frame while the overlay is live (same decision as the arms, `Arms::Live()`), `_visible` of `Health`, `Magica`, `Stamina`, `LeftChargeMeter`, `RightChargeMeter`, `ArrowInfoInstance`, `CompassShoutMeterHolder` (also holds the shout meter) and the crosshair (`_visible` and `_alpha`) is set; the sneak eye stays while sneaking; all shown again when the overlay stops. Logs once which of these paths exist in Skyrim's HUD movie. **2d-2:** new `gmod/module/src/window.rs`: `gmsky.OverlayWindow(off)` moves this process's visible window of Source's class `Valve001` just past the right edge of all screens (no resize, no activation, no Z-order change; nothing else is ever moved) and back to where it was. `liveoverlay.lua`: `gmsky_overlay_hidewindow` (saved, default 1) follows the overlay; logged with old and new positions.
**Tested how:** module 10 tests (new: a window of our own moved past all screens without resizing and put back exactly); harness PASSED (112; the harness never asks to move a window: `gmsky_overlay_hidewindow 0`, and the new check shows no window is found without Source's); plugin builds without warnings. **Not tested in game:** whether the HUD paths are right, and whether Source keeps drawing off-screen.
**Next:** in-game test 50.
## 2026-10-05 In-game test 50 (step 2d): works; Phase 2's planned steps done

**Tested how:** user ran both games, `gmsky_overlay 1` and `0`. User: "Everything working as intended". Logs: `build/logs-ingame-test-50/`.
**Result:** Skyrim's HUD: **all 10 element paths found** (Health, Magica, Stamina, LeftChargeMeter, RightChargeMeter, ArrowInfoInstance, CompassShoutMeterHolder, CrosshairInstance, CrosshairAlert, StealthMeterInstance); hidden at 03:55:04, shown again at 03:55:46. Garry's Mod's window (1280×720) moved from (43, 24) to (1430, 24) (the screens end at x = 1366) and put back at (43, 24). **Source keeps drawing off-screen:** 11 status lines with the overlay on, every frame published (e.g. 1007/1007, 940/940, 834/834 per 5 s, ~170-200 fps).
**Changed:** `liveoverlay.lua`: no "window wasn't moved" line at start-up (cosmetic; harness PASSED).
**Next:** Phase 2's planned steps (2a-2d, blur) are done; what's left is listed in STATUS.md.
## 2026-10-05 README: Phase 2 in "What works"

**Changed:** `README.md`: Garry's Mod's overlay in Skyrim (tests 42-50) moved to "What works"; "What doesn't work yet" now lists quest markers / stamina-magicka display, the window size not matched automatically, and the mouse wheel / weapon switching from Skyrim's window (not tested). Removed three items made out of date by tests 23-24 (bumpy walking and no interiors: fixed by Skyrim's real collision; camera one frame late / no crouch height or view bob: fixed by the camera step).
**Tested how:** documentation only.
## 2026-10-05 Phase 2 closed; Phase 3 planned; step 3a built: weapon choice from Skyrim's window

**Changed:** `docs/DESIGN.md`: "Phase 2 closed" (its "done when" met in test 50; left open on purpose: automatic window size, HOLOHUD quest markers / stamina / magicka) and "Phase 3 steps: combat" (3a-3f, read from SkyCraft's `Combat.cpp` first). **3a:** `protocol/gmsky_protocol.h` + `protocol/src/lib.rs`: `GMSKY_IN_LASTINV` (0x800, Q) and the slot key held in `buttons` bits 16-19 (no layout change, version stays 9). `skse/src/Input.cpp`: keys 1-6 and Q sent to Garry's Mod (the wheel already was). `gmod/lua/gmsky/server.lua`: the wheel (previously read and thrown away) and those keys pick Garry's Mod's weapon in its own order (slot, then position), with `cmd:SelectWeapon`; pressing a slot key again goes to the next weapon in that slot; Q = the weapon before; while the physics gun is held with attack down the wheel goes to it (`cmd:SetMouseWheel`). Logs every switch ("weapon: wheel -1: weapon_pistol -> weapon_smg1"), a per-second count, and the held weapon in the status line.
**Why:** Phase 3 (fight with a crowbar) needs choosing the crowbar from Skyrim's window; README listed the wheel / weapon switching as not tested, and reading the code showed it couldn't work.
**Tested how:** harness PASSED (123 checks; 11 new: wheel down / up 2 notches, slot key, held key, key again, Q, slot 3, physgun wheel, empty slot, log lines); module 10 + protocol 8 unit tests; PROTOCOL TEST PASSED; Skyrim plugin built without warnings and copied. **Not tested in game.**
**Still broken / not tested:** in game (test 51). Garry's Mod's weapon menu doesn't appear (its window has no focus); the switch shows as the new weapon in the overlay. Whether `cmd:SelectWeapon` from the server's `StartCommand` switches in real Garry's Mod: not tested (the harness fakes it).
**Next:** in-game test 51; then step 3b (nearby NPCs, log only).
## 2026-10-05 Step 3a rebuilt: Garry's Mod's own weapon menu and Q = spawn menu (user's feedback)

**Why:** the first 3a build (entry above, never run in game) was rejected by the user on reading it: the server switched weapons directly ("quick switch"), so Garry's Mod's weapon menu (HOLOHUD 2's) never showed, and Q was "last weapon" when in Sandbox **Q is the spawn menu**. Recorded as a dead end: picking weapons on the server skips every addon's weapon menu.
**Changed:** new `gmod/lua/gmsky/binds.lua` (client, included by `client.lua`): Skyrim's wheel, 1-6, mouse buttons and Q become Garry's Mod's binds the way a real key does: the `PlayerBindPress` hook first (HOLOHUD 2's weapon menu listens there: `invnext` / `invprev` / `slotN` open and move it, `+attack` picks; read in its `elements/weaponselection.lua`), else `ply:ConCommand(bind)`; "+binds" get their "-" on release, and all are released when input goes back to Skyrim. Q = `+menu` while held. While a menu cursor is up (`vgui.CursorVisible()`), Skyrim's mouse moves it (`gui.InternalCursorMoved`), its buttons / wheel click and scroll (`gui.InternalMousePressed` / `Released` / `MouseWheeled`), and an arrow is drawn in `DrawOverlay` (the Windows cursor isn't in the picture); `gmsky_cursor_speed` (default 1 pixel per Skyrim count). The physics gun holding something keeps the wheel. `server.lua`: the quick switch removed; while the client reports (module `gmsky.SetClientInput` / `gmsky.ClientInput`, fresh for 0.5 s) attack comes from the player's own command (so a click the weapon menu took doesn't fire) and the view doesn't turn while a cursor is up; otherwise Skyrim's buttons as before; logs "weapon now X (was Y)". Module `lib.rs`: the input reader keeps one baseline per side. Protocol: `GMSKY_IN_LASTINV` renamed `GMSKY_IN_MENU` (same bit 0x800); `Input.cpp` comment. `docs/DESIGN.md` 3a rewritten.
**Tested how:** harness PASSED (140 checks; the 3a checks: wheel = invnext / invprev per notch, slot keys once per press, +attack / -attack, a hook taking +attack (no command, no -attack), Q = +menu / -menu, cursor start and move, menu click / release / wheel, cursor arrow drawn only with a cursor, physgun wheel on both sides, server attack from the command / fallback, no view turn with a cursor and a turn without); module 10 + protocol 8 unit tests; PROTOCOL TEST PASSED; plugin built without warnings. **Not tested in game.**
**Still broken / not tested:** all of it in game (test 51). Unknowns: whether `ConCommand("+attack")` etc. are allowed from Lua and reach the command while Garry's Mod's window has no focus; whether HOLOHUD's `input.SelectWeapon` survives our `StartCommand` (we don't clear the weapon choice); whether VGUI keeps our cursor position or takes the real one; Sandbox's spawn menu may move the real Windows cursor when it opens (harmless to Skyrim's mouse, expected, not checked). Typing in the spawn menu's search box and the C context menu: not done.
**Next:** in-game test 51.
## 2026-10-05 In-game test 53 (step 3a rebuilt): weapon menu, spawn menu and physgun wheel work; camera twitched with the menu cursor

**Tested how:** user played with both games, overlay on (runs 51 and 52 were saved by the launch script; the 3a run is `build/logs-ingame-test-53/`). User: "Solid", but with Q held, moving the cursor turned the camera slightly and it was yanked back; there must be no camera input at all while the cursor is out.
**Result:** `gmod_client.log`: 48 binds; HOLOHUD's weapon menu **took** `invnext` (8), `invprev` (2), `slot1` (4) and `+attack` (5, the picks); `+attack` run 16 times, `+attack2` 2, `+menu` 10. Menu cursor on 10 times, moved by Skyrim's mouse (up to 56 moves a second). `gmod_server.log`: weapons really changed (physgun → ar2 → 357 → crowbar → physgun → gmod_tool → physgun); physics gun wheel up to 12 notches a second. So `ConCommand` binds work while Garry's Mod's window has no focus, and HOLOHUD's `input.SelectWeapon` survives our `StartCommand`.
**Cause of the twitch (from the code, numbers consistent):** Skyrim's camera prediction (Phase 1 step 1b, `Puppet.cpp` `PredictView`) turns Skyrim's camera by mouse movement Garry's Mod hasn't applied yet. With a cursor up, Garry's Mod discards that movement, so its next state "applies" it without turning and the camera snaps back. Skyrim logged prediction ahead by up to 5.28 degrees in this run; I did not match those seconds to the cursor times.
**Changed:** new flag `GMSKY_PF_CURSOR` (0x100, player state flags; no layout change): Garry's Mod sets it while a menu cursor is up **or Q is held** (Q counts at once, before the client sees the spawn menu open), and then doesn't turn the view. Skyrim doesn't predict while the flag is set **or while Q is held in its own window** (`Input::MenuKeyHeld`, no round trip), so the mouse never reaches the camera between pressing Q and Garry's Mod's answer. Skyrim's per-second VIEW line counts "paused for a menu cursor N frames". Server logs "menu cursor up / gone".
**Tested how:** harness PASSED (145; new: the flag reaches Skyrim with a cursor, Q alone suppresses the turn and sets the flag before the client reports, flag cleared after); module 10 + protocol 8 unit tests; PROTOCOL TEST PASSED; plugin built without warnings. **Fix not tested in game.**
**Still broken / not tested:** the fix in game. Menus with a cursor opened without Q (e.g. the C context menu, if added later) are covered only by the flag, i.e. after Garry's Mod's next tick: a few milliseconds of mouse movement there may still twitch the camera (not measured).
**Next:** in-game test 54: hold Q, move the cursor hard; camera must stay still; VIEW lines show "paused for a menu cursor" frames and "ahead max=0.00" during them.
## 2026-10-05 In-game test 54 (menu cursor, no camera): works

**Tested how:** user held Q and moved the cursor, then played on. User: "Good". Logs: `build/logs-ingame-test-54/`.
**Result:** Skyrim's VIEW lines: 160 whole seconds with "paused for a menu cursor 60 frames" (prediction off every frame), partial seconds where Q was pressed or released; Garry's Mod logged the menu cursor up 23 times. No twitch reported. **Step 3a done.**
**Next:** step 3b (nearby NPCs, Skyrim → Garry's Mod, log only).
## 2026-10-05 Step 3b built: Skyrim's nearby actors reach Garry's Mod (log only)

**Changed:** protocol **v10** (`gmsky_protocol.h` + `protocol/src/lib.rs`): new slot `actors` after `terrain` (`GmskyActors`: count, origin epoch, Skyrim frame, 64 × `GmskyActor` of 72 bytes: formId, flags hostile / dead / essential / fighting, feet position relative to the anchor, yaw, radius, height, health fraction, level, name); `GmskyShared` 17,320 → 21,952 bytes (slots area is 64 KB); `SkyrimHeadingToGmodYaw` (inverse of the verified yaw rule, unit-tested both ways). New `skse/src/Actors.cpp` (adapted from SkyCraft's `WriteActorTable`, MIT): every frame, Skyrim's "high process" actors within 6,000 Skyrim units, loaded, enabled, nearest first; once a second an `ACTORS` line (count, nearest three with distance in Skyrim and Garry's Mod units, health, flags, write ms). Module: `gmsky.ReadActors(last_seq)` (adds the anchor; nil for a table of another origin). New `gmod/lua/gmsky/actors.lua` (server): reads every tick, logs actors appearing / leaving (10 lines a second at most) and adds the count and nearest three (distance in both units) to the status line. `docs/DESIGN.md` 3b: range 6,000 and every frame (SkyCraft's choice) instead of 4,000 and 20 a second.
**Tested how:** harness PASSED (150; new: table read with the anchor added, names, flags, "nothing new", status line, another origin refused, log lines); module 10 + protocol 8 unit tests; PROTOCOL TEST PASSED (heading → yaw, round trip every 7°); plugin built without warnings. **Not tested in game.**
**Still broken / not tested:** in game (test 55). Both games must run this build (version 10): an old Garry's Mod module and the new plugin refuse each other's block until both are new.
**Next:** in-game test 55; then step 3c (stand-ins).
## 2026-10-05 In-game test 55 (step 3b, nearby actors): works

**Tested how:** user played both games (Riverwood, a wolf pack, a mudcrab). Logs: `build/logs-ingame-test-55/`. Committed before reading the logs (user's request): `7eb4e46`.
**Result:** Skyrim wrote the table every frame (56-60 writes a second), 5-17 actors, 0.024-0.054 ms a frame on average (0.552 ms max, the first second). Garry's Mod read 57-60 tables a second, logged 118 appear / leave lines (Ralof, Alvor, Hod, Gerdur, Embry, Hilde, Sigrid, cows, chickens, wolves, a mudcrab). **Distances agree exactly for actors standing still:** Alvor 939 Skyrim = 499 Garry's Mod and Embry 999 = 531 in both logs; Ralof (walking) differs by a few units between the two logs' second marks. Flags right: Ralof essential, the wolves "hostile fighting", the mudcrab "hostile dead" with 0% health. No errors on either side; the protocol v10 block opened on both.
**Still broken / not tested:** nothing to see yet (log only). Actor positions vs their models (feet vs body centre) can only be judged with stand-ins (3c).
**Next:** step 3c: invisible stand-ins in Garry's Mod.
## 2026-10-05 Hitbox plan (user's decisions); step 3c-1 built: Skyrim's NPC body shapes read and logged

**Decided (user):** bullets hit copies of Skyrim's own per-bone body shapes (option 2 in `docs/DESIGN.md` "Hitboxes: how bullets hit Skyrim's NPCs", with capsules between bones as the fallback); the hit / miss agreement against Skyrim's own physics is measured with `gmsky_hitbox_test` (target ≥ 98% on a standing NPC). **Corpses must stay hittable** (shoot or blow up bodies, as in Skyrim). **Head shots do extra damage** (amount in 3d).
**Changed:** `docs/DESIGN.md`: the hitbox plan and decisions; 3c split into 3c-1 / 3c-2 / 3c-3. New `skse/src/Hitboxes.cpp` / `.h`, called every frame from `Probe.cpp`: for the nearest 3 living NPCs and the nearest corpse within 3,000 Skyrim units, every collision body in the actor's 3D (scene-graph walk as SkyCraft's `FlingRagdoll`, MIT; Havok offsets as `Collision.cpp`), read under SEH: owning bone, layer, shape kind, motion kind, size, in the physics world or not. Logs once per NPC kind (race) alive and dead the full part list (`HITBOX kind ...`, `HITBOX   <bone> ...`); once a second how far the parts moved **in their bone's own frame** per frame against how far the bones moved (≈ 0 vs bones moving = they follow the animation), read faults, cost.
**Tested how:** plugin built without warnings and copied. Nothing else changed (Garry's Mod side untouched). **Not run in game.**
**Still broken / not tested:** in game (test 56). Assumption under test: living NPCs have per-bone bodies on the BIPED layers that follow the animation (from memory, not verified).
**Next:** in-game test 56; then decide 3c-2 (real shapes or bone capsules).
## 2026-10-05 In-game test 56 (step 3c-1, NPC body shapes): Skyrim's per-bone capsules exist and follow the animation

**Tested how:** user played near Riverwood (people, a dog, chickens, rabbits, wolves, a dead wolf). Logs: `build/logs-ingame-test-56/` (`HITBOX` lines in `skyrim.log`).
**Result:**
- **Parts:** people (Nord, Wood Elf, Old People) 19 capsules each: COM, Spine, Spine1, Spine2, Neck, Head, upper arms, forearms, hands, thighs, calves, feet (head radius 8.8, Spine2 12.6, forearms 4.0 Skyrim units); Dog / Wolf / Rabbit 22, a chicken ("Rabbit" race 000A919D) 7. **Every body part was a capsule.** Plus Ralof's bow: a box on `Bow_MidBone` (world box 36.7 × 56.5 × 107.5), on the BIPED layer too.
- **Layers:** living: BIPED (8) for most, BIPED_NO_CC (33) for Hilde and Faendal; dead wolf: DEADBIP (32). Motion read as "keyframed" for all, including the corpse (that reading may be wrong for a ragdoll: not trusted).
- **They follow the animation:** in their bone's own frame the parts moved 0.00-0.02 units per frame on average (max 1.49) while the bones moved up to 6.24 units per frame (wolves running, 76% of part-frames moving). So the real shapes can be copied (option 2); the bone-capsule fallback isn't needed for these kinds.
- **New:** most living NPCs' parts are **not in Skyrim's physics world** (BIPED: people, dog, wolf, rabbits); Hilde's and Faendal's BIPED_NO_CC parts and the corpse's are. So a plain Havok ray cast wouldn't hit most living NPCs' parts; how Skyrim's arrows hit them is not known yet. Matters for the "against Skyrim's real physics" check (3c-3), not for copying the shapes.
- 0 read faults in 66 seconds; 26-87 µs per frame for 3-4 NPCs (725 µs once).
**Decided (Claude, following the plan):** 3c-2 copies the capsules exactly; parts on non-body bones (the bow) are sent marked as gear and not hit by default.
**Next:** step 3c-2.
## 2026-10-05 Step 3c-2 built: invisible stand-ins in Garry's Mod with Skyrim's own NPC hitboxes

**Changed:** protocol **v11**: new slot `hitboxes` (`GmskyHitboxes`, 16,800 bytes: 24 actors, 512 parts of 32 bytes: capsule end points + radius, or a box marked approximate; Garry's Mod hit group; gear flag; bone index); `GmskyShared` 38,760 bytes. `skse/src/Hitboxes.cpp`: now also sends the nearest 24 actors within 3,000 Skyrim units (living and dead) every frame, each part's hit group from its bone's name (head; chest = Neck / Spine2 / Spine3; stomach = Spine / Spine1 / COM / Pelvis; left / right arm and leg by L / R in the name, a creature's front legs count as arms; tails generic; Bow / Weapon / Shield / Quiver / ... = gear); the group is in the once-per-kind part list; per second: writes, actors and parts sent, approximate, gear, table full, living NPCs with parts in the physics world. New `gmod/module/src/hitboxes.rs`: keeps the newest table; ray vs capsule / box (start inside = no hit), a moving box vs one upright capsule per actor around its parts widened by the box, nearest part to a point; Lua `gmsky.HitboxActors`, `gmsky.HitTrace`, `gmsky.HitPartAt`. New `gmod/lua/gmsky/standins.lua` (both sides): entity `gmsky_actor` per actor (no physics object, custom collisions, bounds = parts' box + 32), moved every tick, removed when Skyrim stops sending it; `TestCollision` through the module; `OnTakeDamage` logs `STAND-IN HIT <name> <formId>: <body part>, <damage> (type) by <weapon>` (not sent to Skyrim: 3d); status line: stand-ins, traces, hits, damage taken, what's under the crosshair (NPC, body part, distance). `gmsky_actor_solid` (bump into NPCs, default 1), `gmsky_hitbox_gear` (a carried bow counts, default 0).
**Tested how:** module 13 unit tests (3 new: capsule side / cap / grazing / misses / inside / parallel / sphere, box, a made-up person: head, chest behind a gear bow, the gap between head and shoulders, the player's box against the body capsule, nearest part); harness PASSED (158; 7 new through the real entity code: one stand-in with a formId above 2^31, a bullet hits the head, the bow is skipped, the gap misses, the player bumps at the body capsule and walks through with `gmsky_actor_solid 0`, damage logged with "head", removed when no longer sent); protocol 8 + PROTOCOL TEST PASSED; plugin built without warnings. **Not tested in game.**
**Still broken / not tested:** in game (test 57). Unknown: whether Garry's Mod's bullets, the crowbar and explosions reach `TestCollision` / `OnTakeDamage` of an entity like this (the ground entity gets the player's movement traces; bullets on it were never checked); the stand-ins' delay against Skyrim's NPCs (not measured; 3c-3).
**Next:** in-game test 57; then 3c-3 (`gmsky_hitbox_test`, Skyrim's crosshair target next to Garry's Mod's).
## 2026-10-05 In-game test 57 (step 3c-2, stand-ins): bullets, crowbar and explosions hit Skyrim's hitboxes; bumping felt weird → flat hull

**Tested how:** user shot and hit Riverwood's people (Embry, Faendal, Hilde, Ralof, Alvor) and a chicken with several weapons, walked into people. User: bumping "felt really weird". Logs: `build/logs-ingame-test-57/`.
**Result:**
- **Everything reached the stand-ins:** 114 logged hits (up to 20 a second are written): SMG 48, pistol 24, AR2 13, crowbar 12, shotgun 12, RPG 5 (inflictor `rpg_missile`: explosions reach them too). Body parts: stomach 34, left leg 19, chest 18, head 13, right arm 11, right leg 10, left arm 8, generic 1. Corpses hit too (Hilde, dead, 40+ hits).
- **Hits land on the parts:** distance from the damage point to the nearest part: median 0.0, the far ones are the RPG's (229.5: explosion damage is placed at the blast).
- Aim log named NPC and part (e.g. "Embry head at 103", "Ralof right arm at 131").
- Skyrim: 13 NPCs and 217 parts a write, 60 writes a second, table never full, 314-344 µs a frame (more than 3c-1's 26-87 µs: 13 NPCs read instead of 3-4). 6 gear parts per write (bows). Living NPCs with parts in the physics world: 180 / 720 (25%).
- Player movement: 10,104 box traces against the stand-ins; no jumps logged; Skyrim didn't push the player (max-drift xy 0.0).
**Cause of the weird bumping (inferred from the design; not measured):** the player bumped into a round capsule recomputed every frame from the NPC's animated parts: it wobbled with the arms and legs, and a round shape makes you slide around people instead of stopping flat.
**Changed:** `hitboxes.rs`: moving boxes now hit one **fixed, flat-sided, world-aligned box** per living NPC (like Garry's Mod's own NPCs), at its feet from the actors table, half width min(bound radius, 0.4 × height) clamped to 6-24 Garry's Mod units (a person: 12), as tall as the NPC; **corpses don't block**; fallback without the actors table: a box around the parts. The store also reloads when the actors table changes. `docs/DESIGN.md` 3c-2 updated.
**Tested how:** module 13 unit tests (new: the hull at the feet, its flat corner (27.9 hits, 28.1 misses), starting inside, corpse without a hull but with hittable parts); harness PASSED (158). **Not tested in game.**
**Still broken / not tested:** the new bumping in game (test 58). Skyrim's cost of reading 13 NPCs every frame (~0.3 ms): could be split over frames for far NPCs if it matters.
**Next:** test 58 (bumping); then 3c-3.
## 2026-10-05 In-game test 58 (flat hull for bumping): good; NPCs walking into you pass through

**Tested how:** user walked into people and let them walk into them. User: "This is nice"; but "if someone walks into me I just phase through them". Logs: `build/logs-ingame-test-58/`.
**Result:** 10,061 box traces against the stand-ins, 451 hits (bumps); no jumps, no errors. The phasing is the "start inside = no hit" rule working as designed (it stops the player getting stuck when an NPC walks into them), but it lets the NPC pass through the player. **Not fixed yet:** options in the reply to the user (push the player out, or make Skyrim's NPCs stop at the player).
**Next:** commit 3c-2; step 3c-3.
