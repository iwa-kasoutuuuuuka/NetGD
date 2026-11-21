<#
.SYNOPSIS  Builds netgd-menu.elf (and optionally netgd-menu.cdi) with the DreamSDK toolchain (KallistiOS), in the style of tools\netgd-recv\build.ps1.
.EXAMPLE   .\build.ps1                       # release build -> netgd-menu.elf
           .\build.ps1 -Debug                # -g -O0
           .\build.ps1 -Cdi                  # also make netgd-menu.cdi (needs makeip, scramble, mkisofs, cdi4dc in the DreamSDK shell)
           .\build.ps1 -BbaTools E:\Dreamcast-Dev-Tools-for-BBA
.NOTES     Needs the owner's Dreamcast-Dev-Tools-for-BBA (for scripts\_common.ps1: DreamSDK location and the Invoke-DreamSdkBash helper).
           Point -BbaTools at it, or set $env:BBA_TOOLS.  NOT run on this PC (no KOS toolchain here); see README_ja.md.
#>
param(
    [switch]$Debug,
    [switch]$Cdi,
    [string]$BbaTools = $env:BBA_TOOLS
)

if (-not $BbaTools -or -not (Test-Path (Join-Path $BbaTools 'scripts\_common.ps1'))) {
    throw 'Dreamcast-Dev-Tools-for-BBA not found. Pass -BbaTools <folder> or set $env:BBA_TOOLS.'
}
. (Join-Path $BbaTools 'scripts\_common.ps1')

$dbg = if ($Debug) { 'DEBUG=1' } else { '' }
Invoke-DreamSdkBash "make clean >/dev/null; make $dbg" $PSScriptRoot
Write-Host "Built $(Join-Path $PSScriptRoot 'netgd-menu.elf')"
if ($Cdi) {
    Invoke-DreamSdkBash "make cdi" $PSScriptRoot
    Write-Host "Built $(Join-Path $PSScriptRoot 'netgd-menu.cdi') - put it alone in the SD card folder 01"
}
