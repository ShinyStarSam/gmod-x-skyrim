# Run one of our Ghidra scripts (tools\ghidra\*.java) on a game DLL, headless and read-only.
# Ghidra 12.1.4 lives in .tools\ghidra_12.1.4_PUBLIC (untracked); its project and its own
# settings, cache and temp files go to build\ghidra (untracked), never to %APPDATA% or %TEMP%.
# The first run on a DLL analyses it (about a minute); later runs reuse the analysis.
#
#   tools\ghidra_dump.ps1 -Script ChainDump -Out build\ghidra\chain.txt -Offsets 0xaaff0,0x97c9b
#   tools\ghidra_dump.ps1 -Script Disasm    -Out build\ghidra\dis.txt   -Offsets 0xaaed0
param(
    [Parameter(Mandatory)] [string] $Script,
    [Parameter(Mandatory)] [string] $Out,
    [Parameter(Mandatory)] [string[]] $Offsets,
    [string] $Dll = 'B:\SteamLibrary\steamapps\common\GarrysMod\bin\win64\vphysics.dll'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$ghidra = Join-Path $root '.tools\ghidra_12.1.4_PUBLIC\support\analyzeHeadless.bat'
if (-not (Test-Path $ghidra)) { throw "Ghidra not found at $ghidra" }
$proj = Join-Path $root 'build\ghidra'
$home_ = (Join-Path $proj 'home') -replace '\\', '/'
New-Item -ItemType Directory -Force "$proj\home\settings", "$proj\home\cache", "$proj\home\temp" | Out-Null

# Ghidra's launcher splits VMARGS at spaces, and the project path has spaces, so the folders go
# through Java's own option variable (which accepts quotes) and the XDG variables (launcher).
$env:XDG_CONFIG_HOME = "$home_/settings"
$env:XDG_CACHE_HOME = "$home_/cache"
$env:JDK_JAVA_OPTIONS = "`"-Dapplication.settingsdir=$home_/settings`" `"-Dapplication.cachedir=$home_/cache`" `"-Dapplication.tempdir=$home_/temp`""

$name = Split-Path $Dll -Leaf
$outFull = [IO.Path]::GetFullPath((Join-Path $root $Out))
$common = @('-scriptPath', (Join-Path $PSScriptRoot 'ghidra'), '-postScript', "$Script.java", $outFull) + $Offsets
# Java prints a note on stderr, which Windows PowerShell 5.1 would treat as an error
$ErrorActionPreference = 'Continue'
if (Test-Path (Join-Path $proj "$([IO.Path]::GetFileNameWithoutExtension($name)).rep")) {
    & $ghidra $proj ([IO.Path]::GetFileNameWithoutExtension($name)) -process $name -noanalysis -readOnly @common *> "$proj\last_run.log"
} else {
    & $ghidra $proj ([IO.Path]::GetFileNameWithoutExtension($name)) -import $Dll @common *> "$proj\last_run.log"
}
$ErrorActionPreference = 'Stop'
if (-not (Test-Path $outFull)) { Get-Content "$proj\last_run.log" -Tail 20; throw 'Ghidra produced no output' }
Write-Host "wrote $outFull (Ghidra log: $proj\last_run.log)"
