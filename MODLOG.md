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
