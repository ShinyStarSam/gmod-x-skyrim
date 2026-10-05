# Saves this run's logs\ into build\logs-ingame-test-<N>\ (untracked), so the next launch can't
# overwrite them. Claude runs it after every in-game test; the user doesn't need to.
# Without -Test it takes the next number after the highest build\logs-ingame-test-N.
# Writes logs\.saved_as (the folder name), so tools\launch_gmod.ps1 knows this run is saved.
#
# Usage: powershell -ExecutionPolicy Bypass -File tools\save_logs.ps1 [-Test <N>] [-Quiet]
param(
    [int]$Test = 0,
    [switch]$Quiet
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$logs = Join-Path $root "logs"
$build = Join-Path $root "build"
if (-not (Test-Path (Join-Path $logs "gmod_client.log")) -and -not (Test-Path (Join-Path $logs "skyrim.log"))) {
    Write-Output "Nothing to save: no game logs in $logs."
    exit 0
}
if ($Test -le 0) {
    $nums = Get-ChildItem $build -Directory -Filter "logs-ingame-test-*" -ErrorAction SilentlyContinue |
        ForEach-Object { if ($_.Name -match '^logs-ingame-test-(\d+)$') { [int]$Matches[1] } }
    $Test = 1 + (($nums | Measure-Object -Maximum).Maximum)
}
$name = "logs-ingame-test-$Test"
$dest = Join-Path $build $name
if (Test-Path $dest) {
    Write-Output "$dest already exists; not overwriting. Pass another -Test number."
    exit 1
}
New-Item -ItemType Directory -Force $build | Out-Null
Copy-Item -Recurse $logs $dest
Remove-Item -Force (Join-Path $dest ".saved_as") -ErrorAction SilentlyContinue
Set-Content -Path (Join-Path $logs ".saved_as") -Value $name -Encoding ascii
$files = (Get-ChildItem $dest -Recurse -File).Count
if (-not $Quiet) { Write-Output "Saved $files files to build\$name" }
