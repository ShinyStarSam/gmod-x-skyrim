# Starts Garry's Mod for gmsky testing: single-player, gm_flatgrass, windowed, and ALWAYS -insecure
# (VAC off, cannot join secure servers). Steam must be running.
# Once the map has loaded, open the console (~) and type:
#   lua_run require("gmsky")
# (+lua_run on the command line was tried and does nothing: in-game test 2026-10-04.)
#
# Usage: powershell -ExecutionPolicy Bypass -File tools\launch_gmod.ps1 [-GmodDir <path>]
param(
    [string]$GmodDir = "B:\SteamLibrary\steamapps\common\GarrysMod"
)
$ErrorActionPreference = "Stop"
$exe = Join-Path $GmodDir "gmod_win64.exe"
$gameArgs = @("-insecure", "-windowed", "-w", "1280", "-h", "720", "-console", "+maxplayers", "1", "+map", "gm_flatgrass")
Write-Output "Starting: $exe $($gameArgs -join ' ')"
Start-Process -FilePath $exe -ArgumentList $gameArgs -WorkingDirectory $GmodDir
Write-Output 'When the map has loaded: press ~ and type  lua_run require("gmsky")'
