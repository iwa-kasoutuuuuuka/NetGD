<#
.SYNOPSIS  Builds netgd-fwupd.elf with the DreamSDK toolchain (same layout as tools\gdprobe\build.ps1 and
           sample/hello_bba/build.ps1) and prepares its host\ folder (the /pc/ folder the program reads the
           update image from and writes the result file to).
.DESCRIPTION
  Needs the Dreamcast-Dev-Tools-for-BBA scripts: set $env:BBA_TOOLS to that repository's folder
  (or pass -BbaTools). Only the DreamSDK shell and the _common.ps1 helpers are used from it.
  -Image <file.ogfw> copies an update image (made with tools\fwimage\fwimage.py build) to host\netgd.ogfw,
  which is where the program looks for it, and checks it with fwimage.py inspect when python is available.
  The program never starts an update by itself: it needs the button combination held for 3 seconds.
.EXAMPLE   .\build.ps1 -BbaTools H:\DC\tools\dc-bba-tools -Image fw_2e.ogfw
#>
param([string]$BbaTools = $env:BBA_TOOLS, [string]$Image = '')
if (-not $BbaTools -or -not (Test-Path (Join-Path $BbaTools 'scripts\_common.ps1'))) {
    throw 'Set $env:BBA_TOOLS (or -BbaTools) to the Dreamcast-Dev-Tools-for-BBA folder.'
}
. (Join-Path $BbaTools 'scripts\_common.ps1')

$hostDir = Join-Path $PSScriptRoot 'host'
New-Item -ItemType Directory -Force $hostDir | Out-Null

if ($Image) {
    $img = (Resolve-Path $Image).Path
    $py = Get-Command python -ErrorAction SilentlyContinue
    if ($py) {
        $fwimage = Join-Path $PSScriptRoot '..\fwimage\fwimage.py'
        & $py.Source $fwimage inspect $img | Out-Host
        if ($LASTEXITCODE -ne 0) { throw "fwimage.py inspect refused $img" }
    } else {
        Write-Host 'python not found: the image was not inspected on the PC (the console checks it again anyway)' -ForegroundColor Yellow
    }
    Copy-Item $img (Join-Path $hostDir 'netgd.ogfw') -Force
    Write-Host "Copied $img to $(Join-Path $hostDir 'netgd.ogfw')"
}

$rel = ConvertTo-PcPath $hostDir
# Stop MSYS2 from rewriting "/pc/..." into "C:/DreamSDK/pc/..." when kos-cc (a Windows exe) is called.
$env:MSYS2_ARG_CONV_EXCL = '-DHOST_DIR_RAW='
Invoke-DreamSdkBash "make clean >/dev/null; make HOST_DIR_REL='$rel'" $PSScriptRoot
Write-Host "Built $(Join-Path $PSScriptRoot 'netgd-fwupd.elf')  (image: /pc$rel/netgd.ogfw = $(Join-Path $hostDir 'netgd.ogfw'); result files: /pc$rel/fwupd_result_NNN.txt)"
Write-Host 'Send it with dc_run.ps1 (one shot). Do NOT leave it under a watching dc_auto preset.'
