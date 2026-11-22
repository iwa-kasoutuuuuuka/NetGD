<#
.SYNOPSIS
  Build the pin-finder firmware image for one stage, reproducibly, and print
  which bitstream is inside (SHA-256), so you can tell which one you flash.

.DESCRIPTION
  Stage 0 = no outputs driven (fpga/pinfind/FIRST_RUN_ja.md uses this one).
  Stage 1 = the first 4 balls only (mklist --max-drive 4), default design.
  Stage 2 = all balls, default design.

  Steps (README_ja.md 3-1 .. 3-4):
    1. WSL: mklist.py  (Supra's pinmap -> fpga/pinfind/build/balls.txt)
    2. Windows: gen.py (Amaranth venv; --stage0 for stage 0)
    3. WSL: build.sh under `flock $HOME/supra.lock` (Supra only through the repo script)
    4. Windows: cargo build --release --no-default-features --features pinfind
       (in mcu/netgd, default target dir)
    5. copy the ELF to tools/pinfind/out/netgd-pinfind-stage<N>-<sha8>.elf (+ .txt record)

  Nothing here touches hardware. Bitstreams are never committed
  (fpga/pinfind/output/ and tools/pinfind/out/ are ignored).

.EXAMPLE
  powershell -File tools\pinfind\mkstage.ps1 -Stage 0
#>
param(
    [Parameter(Mandatory = $true)][ValidateSet(0, 1, 2)][int]$Stage,
    [string]$Distro = 'Ubuntu-24.04',
    [string]$SupraHome = '$HOME/supra-linux/x/Supra-2026.03.b0',
    [string]$Python = 'H:\OpenGDEmu\fpga\amaranth\amaranth_env\Scripts\python.exe'
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$pf = Join-Path $root 'fpga\pinfind'
$fw = Join-Path $root 'mcu\netgd'
$outDir = Join-Path $PSScriptRoot 'out'
New-Item -ItemType Directory -Force $outDir | Out-Null

# Run a bash script text inside WSL. It goes through a temp file (LF endings):
# wsl.exe expands $VARS in a -c string with the default shell before bash sees it.
function Invoke-Wsl([string]$What, [string]$Script) {
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("mkstage-{0}.sh" -f [guid]::NewGuid().ToString('N'))
    [IO.File]::WriteAllText($tmp, ($Script -replace "`r", '') + "`n", (New-Object Text.UTF8Encoding($false)))
    try {
        $wtmp = (& wsl -d $Distro -- wslpath -a ($tmp.Replace('\', '/'))).Trim()
        & wsl -d $Distro -- bash $wtmp
        if ($LASTEXITCODE -ne 0) { throw "$What failed (exit $LASTEXITCODE)" }
    } finally { Remove-Item -ErrorAction SilentlyContinue $tmp }
}

function Invoke-Native([string]$What, [scriptblock]$Block) {
    & $Block
    if ($LASTEXITCODE -ne 0) { throw "$What failed (exit $LASTEXITCODE)" }
}

# /mnt/h/... path of the repo as WSL sees it
$wslRoot = (& wsl -d $Distro -- wslpath -a ($root -replace '\\', '/')).Trim()
if (-not $wslRoot) { throw 'wslpath failed' }

$mklistArgs = ''
if ($Stage -eq 1) { $mklistArgs = '--max-drive 4' }
$genArgs = @('--balls', 'build\balls.txt')
if ($Stage -eq 0) { $genArgs += '--stage0' }

Write-Host "== stage $Stage : 1/4 ball list (WSL, Supra pinmap read in place)"
Remove-Item -ErrorAction SilentlyContinue "$pf\build\balls.txt"
New-Item -ItemType Directory -Force "$pf\build" | Out-Null
$cmd1 = "set -e`ncd '$wslRoot'`nexport SUPRA_HOME=$SupraHome`npython3 tools/pinfind/mklist.py `"`$SUPRA_HOME/etc/AG16KDDF256.pinmap`" -o fpga/pinfind/build/balls.txt $mklistArgs"
Invoke-Wsl 'mklist' $cmd1

Write-Host "== stage $Stage : 2/4 gen.py $($genArgs -join ' ')"
Push-Location $pf
try { Invoke-Native 'gen.py' { & $Python gen.py @genArgs } } finally { Pop-Location }

Write-Host "== stage $Stage : 3/4 Supra build (WSL, flock \$HOME/supra.lock)"
$cmd3 = "set -e`ncd '$wslRoot'`nexport SUPRA_HOME=$SupraHome`nflock `$HOME/supra.lock bash fpga/pinfind/build.sh"
Invoke-Wsl 'build.sh' $cmd3

$rbf = Join-Path $pf 'output\pinfind_ag16k.rbf'
$params = Get-Content (Join-Path $pf 'output\pinfind_ag16k.params')
$kv = @{}
foreach ($l in $params) { $p = $l.Trim() -split ' ', 2; if ($p.Count -eq 2) { $kv[$p[0]] = $p[1] } }
$rbfSha = (Get-FileHash -Algorithm SHA256 $rbf).Hash.ToLower()
if ($kv['rbf_sha256'] -ne $rbfSha) { throw "params rbf_sha256 ($($kv['rbf_sha256'])) != SHA-256 of the .rbf ($rbfSha)" }
$isStage0 = ($kv['stage0'] -eq '1')
if (($Stage -eq 0) -ne $isStage0) { throw "requested stage $Stage but params say stage0=$($kv['stage0'])" }
if ($Stage -eq 1 -and $kv['n_drive'] -ne '4') { throw "stage 1 must drive 4 balls, params say n_drive=$($kv['n_drive'])" }
$lzma = ((Get-Content (Join-Path $pf 'output\pinfind_ag16k.txt')) | Where-Object { $_ -match '^lzma\s' }) -replace '^lzma\s+', ''

Write-Host "== stage $Stage : 4/4 cargo build (mcu/netgd)"
Push-Location $fw
try {
    Invoke-Native 'cargo build' { cargo build --release --no-default-features --features pinfind }
} finally { Pop-Location }
$elf = Join-Path $fw 'target\thumbv7m-none-eabi\release\netgd'
if (-not (Test-Path $elf)) { throw "ELF not found: $elf" }
$elfSha = (Get-FileHash -Algorithm SHA256 $elf).Hash.ToLower()
$elfSize = (Get-Item $elf).Length
$dest = Join-Path $outDir ("netgd-pinfind-stage{0}-{1}.elf" -f $Stage, $rbfSha.Substring(0, 8))
Copy-Item -Force $elf $dest

$report = @(
    "stage              $Stage  ($(if ($isStage0) { 'NO outputs driven' } else { 'DRIVES balls: n_drive=' + $kv['n_drive'] }))"
    "firmware ELF       $dest"
    "firmware SHA-256   $elfSha"
    "firmware ELF size  $elfSize bytes (file; flash image size: see 'size' of the ELF)"
    "bitstream SHA-256  $rbfSha"
    "bitstream size     $((Get-Item $rbf).Length) bytes raw, LZMA $lzma"
    "balls.txt SHA-256  $($kv['balls_sha256'])"
    "boot log shows     PF STAGE $(if ($isStage0) { '0 (no outputs driven)' } else { '1-3 (DRIVES balls; NOT stage 0)' }) bitstream sha256=$rbfSha"
)
$report | Set-Content -Encoding ascii ([IO.Path]::ChangeExtension($dest, '.txt'))
Write-Host ''
$report | ForEach-Object { Write-Host $_ }
