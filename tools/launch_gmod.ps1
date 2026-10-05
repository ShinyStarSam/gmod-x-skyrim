# Starts Garry's Mod for gmsky testing: windowed, and ALWAYS -insecure (VAC off, cannot join secure
# servers). Steam must be running.
# By default it loads gm_flatmap straight into single-player (open space to the engine's limit in
# every direction, ~32,600 units, measured 2026-10-04). That only works with the map as a plain
# file in garrysmod/maps (gm_flatmap.bsp): a Workshop map can't be started from this command line
# (`+map gm_largeemptyplane` failed with "No such map", 2026-10-04: Workshop content is mounted only
# after the menu loads, as far as we can tell). Without the file it stops at the main menu, as before:
# Start New Game -> gm_flatmap (Workshop item 3669476334), Single Player.
# -Map <name> loads another map (e.g. -Map gm_flatgrass); -Menu stops at the main menu.
# gmsky loads by itself once you're in the map (garrysmod/lua/autorun/server/gmsky_autoload.lua,
# installed once by tools/install_gmod_autoload.ps1). Without that file: press ~ and type
#   lua_run require("gmsky")
# Tried and dropped (2026-10-04): +lua_run on this command line does nothing (the map isn't loaded
# yet); sending it later with -hijack reaches the game but passes only one word to lua_run.
#
# Usage: powershell -ExecutionPolicy Bypass -File tools\launch_gmod.ps1 [-GmodDir <path>] [-Map <name>] [-Menu]
param(
    [string]$GmodDir = "B:\SteamLibrary\steamapps\common\GarrysMod",
    [string]$Map = "gm_flatmap",
    [switch]$Menu
)
$ErrorActionPreference = "Stop"
$exe = Join-Path $GmodDir "gmod_win64.exe"
# Safety net: the module starts its logs afresh on launch. If the last run's logs weren't saved
# (tools\save_logs.ps1 writes logs\.saved_as), save them now under the next test number.
$projectLogs = Join-Path (Split-Path -Parent $PSScriptRoot) "logs"
$client = Join-Path $projectLogs "gmod_client.log"
$marker = Join-Path $projectLogs ".saved_as"
if ((Test-Path $client) -and (-not (Test-Path $marker) -or (Get-Item $client).LastWriteTime -gt (Get-Item $marker).LastWriteTime)) {
    Write-Output "The last run's logs weren't saved yet; saving them first."
    & (Join-Path $PSScriptRoot "save_logs.ps1")
}
# +fps_max_nofocus 300: Garry's Mod caps itself at 20 fps while Skyrim's window is in front, which
# makes the view choppy (in-game test 4). This 64-bit build has no engine_no_focus_sleep.
$gameArgs = @("-insecure", "-windowed", "-w", "1280", "-h", "720", "-console", "+fps_max_nofocus", "300")
if (-not $Menu) {
    $bsp = Join-Path $GmodDir "garrysmod\maps\$Map.bsp"
    if (Test-Path $bsp) {
        $gameArgs += @("+maxplayers", "1", "+map", $Map)
    } else {
        Write-Output "No $bsp (a Workshop map can't be loaded from the command line): stopping at the main menu."
        $Menu = $true
    }
}
Write-Output "Starting: $exe $($gameArgs -join ' ')"
Start-Process -FilePath $exe -ArgumentList $gameArgs -WorkingDirectory $GmodDir
if ($Menu) {
    Write-Output "At the main menu: Start New Game -> gm_flatmap, Single Player -> Start Game."
} else {
    Write-Output "Loading $Map in single-player."
}
if (Test-Path (Join-Path $GmodDir "garrysmod\lua\autorun\server\gmsky_autoload.lua")) {
    Write-Output "gmsky loads by itself once you're in the map (check: logs\gmod_server.log)."
} else {
    Write-Output 'Auto-loader not installed. When the map has loaded: press ~ and type  lua_run require("gmsky")'
}
