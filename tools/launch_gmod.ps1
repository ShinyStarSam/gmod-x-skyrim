# Starts Garry's Mod for gmsky testing: single-player, gm_flatgrass, windowed, and ALWAYS -insecure
# (VAC off, cannot join secure servers). Steam must be running.
# gmsky loads by itself once you're in the map (garrysmod/lua/autorun/server/gmsky_autoload.lua,
# installed once by tools/install_gmod_autoload.ps1). Without that file: press ~ and type
#   lua_run require("gmsky")
# Tried and dropped (2026-10-04): +lua_run on this command line does nothing (the map isn't loaded
# yet); sending it later with -hijack reaches the game but passes only one word to lua_run.
#
# Usage: powershell -ExecutionPolicy Bypass -File tools\launch_gmod.ps1 [-GmodDir <path>]
param(
    [string]$GmodDir = "B:\SteamLibrary\steamapps\common\GarrysMod"
)
$ErrorActionPreference = "Stop"
$exe = Join-Path $GmodDir "gmod_win64.exe"
# +fps_max_nofocus 300: Garry's Mod caps itself at 20 fps while Skyrim's window is in front, which
# makes the view choppy (in-game test 4). This 64-bit build has no engine_no_focus_sleep.
$gameArgs = @("-insecure", "-windowed", "-w", "1280", "-h", "720", "-console", "+fps_max_nofocus", "300", "+maxplayers", "1", "+map", "gm_flatgrass")
Write-Output "Starting: $exe $($gameArgs -join ' ')"
Start-Process -FilePath $exe -ArgumentList $gameArgs -WorkingDirectory $GmodDir
if (Test-Path (Join-Path $GmodDir "garrysmod\lua\autorun\server\gmsky_autoload.lua")) {
    Write-Output "gmsky loads by itself once you're in the map (check: logs\gmod_server.log)."
} else {
    Write-Output 'Auto-loader not installed. When the map has loaded: press ~ and type  lua_run require("gmsky")'
}
