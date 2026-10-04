# ONE-TIME install of the gmsky loader stubs into Garry's Mod (the single permitted write to lua/bin).
# Copies target\release\gmsky_stub.dll to:
#   <GarrysMod>\garrysmod\lua\bin\gmsv_gmsky_win64.dll
#   <GarrysMod>\garrysmod\lua\bin\gmcl_gmsky_win64.dll
# Refuses to run if either file already exists. After this, rebuilding the module never touches the game.
#
# Usage: powershell -ExecutionPolicy Bypass -File tools\install_gmod_stubs.ps1 [-GmodDir <path>]
param(
    [string]$GmodDir = "B:\SteamLibrary\steamapps\common\GarrysMod"
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$stub = Join-Path $root "target\release\gmsky_stub.dll"
$bin = Join-Path $GmodDir "garrysmod\lua\bin"
$targets = @("gmsv_gmsky_win64.dll", "gmcl_gmsky_win64.dll") | ForEach-Object { Join-Path $bin $_ }

if (-not (Test-Path $stub)) { throw "Build first: cargo build --release  (missing $stub)" }
if (-not (Test-Path (Join-Path $GmodDir "gmod_win64.exe"))) { throw "Not a Garry's Mod folder: $GmodDir" }
foreach ($t in $targets) {
    if (Test-Path $t) { throw "Already installed: $t. This script only runs once. Not touching anything." }
}

New-Item -ItemType Directory -Force -Path $bin | Out-Null
foreach ($t in $targets) {
    Copy-Item -Path $stub -Destination $t
    $hash = (Get-FileHash $t -Algorithm SHA256).Hash
    Write-Output "installed $t  sha256=$hash"
}
