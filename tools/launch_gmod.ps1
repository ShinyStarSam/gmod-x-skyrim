# Starts Garry's Mod for gmsky testing: single-player, gm_flatgrass, windowed, and ALWAYS -insecure
# (VAC off, cannot join secure servers). Steam must be running.
# Tries to load gmsky automatically with +lua_run; if that doesn't work, type in the console (~):
#   lua_run require("gmsky")
#
# Usage: powershell -ExecutionPolicy Bypass -File tools\launch_gmod.ps1 [-NoAutoLoad] [-GmodDir <path>]
param(
    [string]$GmodDir = "B:\SteamLibrary\steamapps\common\GarrysMod",
    [switch]$NoAutoLoad
)
$ErrorActionPreference = "Stop"
$exe = Join-Path $GmodDir "gmod_win64.exe"
$gameArgs = @("-insecure", "-windowed", "-w", "1280", "-h", "720", "-console", "+maxplayers", "1", "+map", "gm_flatgrass")
if (-not $NoAutoLoad) { $gameArgs += @("+lua_run", "require('gmsky')") }
Write-Output "Starting: $exe $($gameArgs -join ' ')"
Start-Process -FilePath $exe -ArgumentList $gameArgs -WorkingDirectory $GmodDir
