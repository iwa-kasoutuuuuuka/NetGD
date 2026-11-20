<#
.SYNOPSIS  Builds netgd-recv.elf with the DreamSDK toolchain (KallistiOS), in the style of the BBA tools' sample\hello_bba\build.ps1.
.EXAMPLE   .\build.ps1                       # release build
           .\build.ps1 -Debug                # -g -O0
           .\build.ps1 -BbaTools E:\Dreamcast-Dev-Tools-for-BBA
.NOTES     Needs the owner's Dreamcast-Dev-Tools-for-BBA (for scripts\_common.ps1: DreamSDK location and the Invoke-DreamSdkBash helper).
           Point -BbaTools at it, or set $env:BBA_TOOLS.  NOT run on this PC (no KOS toolchain here); see README_ja.md.
           -JobDir changes the default job folder compiled into the ELF (default /pc/netgd/dc1; netgd-send's
           job folder is <drive>:\netgd\<unit>, and the app also finds the only /pc/netgd/<unit> that has a job.ngj).
#>
param(
    [switch]$Debug,
    [string]$BbaTools = $env:BBA_TOOLS,
    [string]$JobDir = ''
)

if (-not $BbaTools -or -not (Test-Path (Join-Path $BbaTools 'scripts\_common.ps1'))) {
    throw 'Dreamcast-Dev-Tools-for-BBA not found. Pass -BbaTools <folder> or set $env:BBA_TOOLS.'
}
. (Join-Path $BbaTools 'scripts\_common.ps1')

$dbg = if ($Debug) { 'DEBUG=1' } else { '' }
$job = if ($JobDir) { "NG_JOBDIR=$JobDir" } else { '' }
# Stop MSYS2 from rewriting "/pc/..." into "C:/DreamSDK/pc/..." when kos-cc (a Windows exe) is called.
$env:MSYS2_ARG_CONV_EXCL = '-DNG_JOBDIR_DEFAULT='
Invoke-DreamSdkBash "make clean >/dev/null; make $dbg $job" $PSScriptRoot
Write-Host "Built $(Join-Path $PSScriptRoot 'netgd-recv.elf')"
