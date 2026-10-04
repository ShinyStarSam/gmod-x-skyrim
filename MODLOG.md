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
