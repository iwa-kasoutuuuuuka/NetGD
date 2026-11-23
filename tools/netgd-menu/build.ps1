<#
.SYNOPSIS  Builds netgd-menu.elf (and optionally netgd-menu-tcpfix.elf, netgd-menu.cdi) with the DreamSDK toolchain (KallistiOS), in the style of tools\netgd-recv\build.ps1.
.EXAMPLE   .\build.ps1                       # release build -> netgd-menu.elf (stock KOS TCP: NGTP window 16 KiB x 1)
           .\build.ps1 -Debug                # -g -O0
           .\build.ps1 -TcpFix               # also netgd-menu-tcpfix.elf (patched net_tcp.c from ..\kos-patches linked in; window 16 KiB x 1 unless -Make 'NM_WANT=.. NM_DEPTH=..')
           .\build.ps1 -Make 'NM_IP=0xC0A80A02'   # make variables (the static IP, NM_RCVBUF; numbers only)
           .\build.ps1 -Cdi                  # also make netgd-menu.cdi (needs makeip, scramble, mkisofs, cdi4dc in the DreamSDK shell)
           .\build.ps1 -BbaTools E:\Dreamcast-Dev-Tools-for-BBA
.NOTES     Needs the owner's Dreamcast-Dev-Tools-for-BBA (for scripts\_common.ps1: DreamSDK location and the Invoke-DreamSdkBash helper).
           Point -BbaTools at it, or set $env:BBA_TOOLS.  DreamSDK is not modified.  NOT run on hardware; see README_ja.md.
#>
param(
    [switch]$Debug,
    [switch]$Cdi,
    [switch]$TcpFix,
    [string]$Make = '',
    [string]$BbaTools = $env:BBA_TOOLS
)

if (-not $BbaTools -or -not (Test-Path (Join-Path $BbaTools 'scripts\_common.ps1'))) {
    throw 'Dreamcast-Dev-Tools-for-BBA not found. Pass -BbaTools <folder> or set $env:BBA_TOOLS.'
}
. (Join-Path $BbaTools 'scripts\_common.ps1')

$dbg = if ($Debug) { 'DEBUG=1' } else { '' }
Invoke-DreamSdkBash "make clean >/dev/null; make $dbg $Make" $PSScriptRoot
Write-Host "Built $(Join-Path $PSScriptRoot 'netgd-menu.elf')"
if ($TcpFix) {
    Invoke-DreamSdkBash "make TCPFIX=1 $dbg $Make" $PSScriptRoot
    Write-Host "Built $(Join-Path $PSScriptRoot 'netgd-menu-tcpfix.elf')"
}
if ($Cdi) {
    Invoke-DreamSdkBash "make cdi $Make" $PSScriptRoot
    Write-Host "Built $(Join-Path $PSScriptRoot 'netgd-menu.cdi') - put it alone in the SD card folder 01"
}
