# PowerShell wrapper: runs tools/ci-local/run.sh in Git bash / DreamSDK bash.
#   powershell -File tools\ci-local\run.ps1 [--only tests] [--worktree PATH]
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$sh = $null
foreach ($c in @("C:\DreamSDK\msys\1.0\bin\sh.exe", "C:\Program Files\Git\bin\sh.exe")) {
    if (Test-Path $c) { $sh = $c; break }
}
if (-not $sh) { $cmd = Get-Command sh -ErrorAction SilentlyContinue; if ($cmd) { $sh = $cmd.Source } }
if (-not $sh) { Write-Error "sh.exe not found"; exit 2 }
& $sh (Join-Path $here "run.sh") @args
exit $LASTEXITCODE
