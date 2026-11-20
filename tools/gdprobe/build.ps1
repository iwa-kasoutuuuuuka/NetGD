<#
.SYNOPSIS  Builds gdprobe.elf with the DreamSDK toolchain (same layout as sample/hello_bba/build.ps1)
           and prepares its host\ folder (the /pc/ folder the record is written to).
.DESCRIPTION
  Needs the Dreamcast-Dev-Tools-for-BBA scripts: set $env:BBA_TOOLS to that repository's folder
  (or pass -BbaTools). Only the DreamSDK shell, _common.ps1 helpers and the host folder are used from it.
  gdprobe.cfg (the run configuration, see README_ja.md) is created in host\ if it is not there yet.
.EXAMPLE   .\build.ps1 -BbaTools H:\DC\tools\dc-bba-tools
#>
param([string]$BbaTools = $env:BBA_TOOLS)
if (-not $BbaTools -or -not (Test-Path (Join-Path $BbaTools 'scripts\_common.ps1'))) {
    throw 'Set $env:BBA_TOOLS (or -BbaTools) to the Dreamcast-Dev-Tools-for-BBA folder.'
}
. (Join-Path $BbaTools 'scripts\_common.ps1')

$hostDir = Join-Path $PSScriptRoot 'host'
New-Item -ItemType Directory -Force $hostDir | Out-Null
$cfg = Join-Path $hostDir 'gdprobe.cfg'
if (-not (Test-Path $cfg)) {
    Copy-Item (Join-Path $PSScriptRoot 'gdprobe.cfg.example') $cfg
    Write-Host "Created $cfg (edit label= and the switch settings before a run)"
}

$rel = ConvertTo-PcPath $hostDir
# Stop MSYS2 from rewriting "/pc/..." into "C:/DreamSDK/pc/..." when kos-cc (a Windows exe) is called.
$env:MSYS2_ARG_CONV_EXCL = '-DHOST_DIR_RAW='
Invoke-DreamSdkBash "make clean >/dev/null; make HOST_DIR_REL='$rel'" $PSScriptRoot
Write-Host "Built $(Join-Path $PSScriptRoot 'gdprobe.elf')  (records go to /pc$rel, i.e. $hostDir)"
