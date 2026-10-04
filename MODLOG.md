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
