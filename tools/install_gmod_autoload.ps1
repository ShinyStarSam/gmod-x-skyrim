# ONE-TIME install of the gmsky auto-loader into Garry's Mod (user's request, 2026-10-04).
# Copies gmod\autorun\gmsky_autoload.lua to:
#   <GarrysMod>\garrysmod\lua\autorun\server\gmsky_autoload.lua
# Garry's Mod runs it on every map load; in single-player it loads gmsky once the player is in game.
# Refuses to run if the file already exists. To stop auto-loading, delete that one file.
#
# Usage: powershell -ExecutionPolicy Bypass -File tools\install_gmod_autoload.ps1 [-GmodDir <path>]
param(
    [string]$GmodDir = "B:\SteamLibrary\steamapps\common\GarrysMod"
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$source = Join-Path $root "gmod\autorun\gmsky_autoload.lua"
$dir = Join-Path $GmodDir "garrysmod\lua\autorun\server"
$target = Join-Path $dir "gmsky_autoload.lua"

if (-not (Test-Path (Join-Path $GmodDir "gmod_win64.exe"))) { throw "Not a Garry's Mod folder: $GmodDir" }
if (-not (Test-Path $dir)) { throw "Missing $dir (not touching anything)" }
if (Test-Path $target) { throw "Already installed: $target. This script only runs once. Not touching anything." }

Copy-Item -Path $source -Destination $target
$hash = (Get-FileHash $target -Algorithm SHA256).Hash
Write-Output "installed $target  sha256=$hash"
