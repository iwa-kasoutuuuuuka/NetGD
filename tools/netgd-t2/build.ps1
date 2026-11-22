<#
.SYNOPSIS  Builds netgd-t2.elf (stock KOS TCP) and, with -TcpFix, netgd-t2-tcpfix.elf (patched net_tcp.c linked in) with DreamSDK.
.EXAMPLE   .\build.ps1 -BbaTools H:\DC\tools\dc-bba-tools
           .\build.ps1 -BbaTools H:\DC\tools\dc-bba-tools -TcpFix
           .\build.ps1 -BbaTools H:\DC\tools\dc-bba-tools -Make 'T2_NO_DRIVE=1 T2_CODE=1234'
.NOTES     Uses scripts\_common.ps1 of the BBA tools (DreamSDK location, Invoke-DreamSdkBash), like tools\netgd-menu\build.ps1.
           DreamSDK is not modified.  NOT run on hardware.
#>
param(
    [switch]$Debug,
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
Write-Host "Built $(Join-Path $PSScriptRoot 'netgd-t2.elf')"
if ($TcpFix) {
    Invoke-DreamSdkBash "make TCPFIX=1 $dbg $Make" $PSScriptRoot
    Write-Host "Built $(Join-Path $PSScriptRoot 'netgd-t2-tcpfix.elf')"
}
