# Starts Garry's Mod for gmsky testing: windowed, and ALWAYS -insecure (VAC off, cannot join secure
# servers). Steam must be running. By default it stops at the main menu: pick Start New Game ->
# gm_flatmap (Workshop item 3669476334: open space to the engine's limit in every direction,
# ~32,600 units, measured 2026-10-04), Single Player. A Workshop map can't be started from this
# command line: `+map gm_largeemptyplane` failed with "No such map" (2026-10-04; Workshop content
# is mounted only after the menu loads, as far as we can tell).
# -Map <name> starts a built-in map directly, e.g. -Map gm_flatgrass.
# gmsky loads by itself once you're in the map (garrysmod/lua/autorun/server/gmsky_autoload.lua,
# installed once by tools/install_gmod_autoload.ps1). Without that file: press ~ and type
#   lua_run require("gmsky")
# Tried and dropped (2026-10-04): +lua_run on this command line does nothing (the map isn't loaded
# yet); sending it later with -hijack reaches the game but passes only one word to lua_run.
#
# Usage: powershell -ExecutionPolicy Bypass -File tools\launch_gmod.ps1 [-GmodDir <path>] [-Map <name>]
param(
    [string]$GmodDir = "B:\SteamLibrary\steamapps\common\GarrysMod",
    [string]$Map = ""
)
$ErrorActionPreference = "Stop"
$exe = Join-Path $GmodDir "gmod_win64.exe"
# +fps_max_nofocus 300: Garry's Mod caps itself at 20 fps while Skyrim's window is in front, which
# makes the view choppy (in-game test 4). This 64-bit build has no engine_no_focus_sleep.
$gameArgs = @("-insecure", "-windowed", "-w", "1280", "-h", "720", "-console", "+fps_max_nofocus", "300")
if ($Map) { $gameArgs += @("+maxplayers", "1", "+map", $Map) }
Write-Output "Starting: $exe $($gameArgs -join ' ')"
Start-Process -FilePath $exe -ArgumentList $gameArgs -WorkingDirectory $GmodDir
if (-not $Map) {
    Write-Output "At the main menu: Start New Game -> gm_flatmap, Single Player -> Start Game."
}
if (Test-Path (Join-Path $GmodDir "garrysmod\lua\autorun\server\gmsky_autoload.lua")) {
    Write-Output "gmsky loads by itself once you're in the map (check: logs\gmod_server.log)."
} else {
    Write-Output 'Auto-loader not installed. When the map has loaded: press ~ and type  lua_run require("gmsky")'
}
