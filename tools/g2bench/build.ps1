<#
.SYNOPSIS  Builds g2bench.elf with the DreamSDK toolchain and creates host\ with the 32 MiB input file.
.EXAMPLE   .\build.ps1 -BbaTools H:\DC\tools\dc-bba-tools
#>
param([string]$BbaTools = $env:BBA_TOOLS)
if (-not $BbaTools -or -not (Test-Path (Join-Path $BbaTools 'scripts\_common.ps1'))) {
    throw 'Set $env:BBA_TOOLS (or -BbaTools) to the Dreamcast-Dev-Tools-for-BBA folder.'
}
. (Join-Path $BbaTools 'scripts\_common.ps1')

$hostDir = Join-Path $PSScriptRoot 'host'
New-Item -ItemType Directory -Force $hostDir | Out-Null
$data = Join-Path $hostDir 'g2bench_data.bin'
if (-not (Test-Path $data) -or (Get-Item $data).Length -ne 32MB) {
    $py = @((Get-Command python -ErrorAction SilentlyContinue), (Get-Command py -ErrorAction SilentlyContinue)) | Where-Object { $_ } | Select-Object -First 1
    if (-not $py) { Write-Warning 'python not found: create host\g2bench_data.bin with mkdata.py yourself (phases A and C are skipped without it)' }
    else { & $py.Source (Join-Path $PSScriptRoot 'mkdata.py') $data 32; Write-Host "Created $data (32 MiB)" }
}

$rel = ConvertTo-PcPath $hostDir
# Stop MSYS2 from rewriting "/pc/..." into "C:/DreamSDK/pc/..." when kos-cc (a Windows exe) is called.
$env:MSYS2_ARG_CONV_EXCL = '-DHOST_DIR_RAW='
Invoke-DreamSdkBash "make clean >/dev/null; make HOST_DIR_REL='$rel'" $PSScriptRoot
Write-Host "Built $(Join-Path $PSScriptRoot 'g2bench.elf')  (results go to /pc$rel, i.e. $hostDir)"
