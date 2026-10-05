# Installs our patched personal copy of HOLOHUD 2 into Garry's Mod (docs/DESIGN.md "HOLOHUD 2,
# patched"). Run by the user: it writes into the game folder.
# Copies build\holohud2_gmsky (made by tools\build_holohud.py) to:
#   <GarrysMod>\garrysmod\addons\holohud2_gmsky
# Replaces only that folder, and only if it is one of ours (has gmsky_patch.txt).
# Disable the Workshop HOLOHUD 2 in Garry's Mod's Addons menu, or both copies load.
# To uninstall: delete that one folder.
#
# Usage: powershell -ExecutionPolicy Bypass -File tools\install_holohud.ps1 [-GmodDir <path>]
param(
    [string]$GmodDir = "B:\SteamLibrary\steamapps\common\GarrysMod"
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$source = Join-Path $root "build\holohud2_gmsky"
$addons = Join-Path $GmodDir "garrysmod\addons"
$target = Join-Path $addons "holohud2_gmsky"

if (-not (Test-Path (Join-Path $GmodDir "gmod_win64.exe"))) { throw "Not a Garry's Mod folder: $GmodDir" }
if (-not (Test-Path (Join-Path $source "gmsky_patch.txt"))) { throw "No build in $source. Run: python tools\build_holohud.py" }
if (-not (Test-Path $addons)) { throw "Missing $addons (not touching anything)" }
if (Test-Path $target) {
    if (-not (Test-Path (Join-Path $target "gmsky_patch.txt"))) { throw "$target exists and isn't ours. Not touching it." }
    Remove-Item -Recurse -Force $target
}

Copy-Item -Recurse -Path $source -Destination $target
$render = Join-Path $target "lua\holohud2\render.lua"
$hash = (Get-FileHash $render -Algorithm SHA256).Hash
$count = (Get-ChildItem -Recurse -File $target).Count
Write-Output "installed $target ($count files)  render.lua sha256=$hash"
Write-Output "Now disable the Workshop HOLOHUD 2 in Garry's Mod (main menu, Addons) if it is still on."
