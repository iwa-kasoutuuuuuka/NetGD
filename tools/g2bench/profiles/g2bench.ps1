# dc_auto.ps1 preset: g2bench (M0 of docs/research/g2-perf-plan.md; no GD-ROM access).
#   .\scripts\dc_auto.ps1 -Preset <full path to this file>
# Results: <g2bench>\host\g2bench_NNN.txt. dc-tool-ip must run with its current directory on the same
# drive as the NetGD repository (a /pc/ rule of dcload-ip).
$preset_g2 = if ($PSCommandPath) { Split-Path -Parent (Split-Path -Parent $PSCommandPath) }
             elseif ($env:G2BENCH_DIR) { $env:G2BENCH_DIR }
             else { throw 'g2bench preset: cannot locate tools\g2bench (use -Preset <full path>, or set $env:G2BENCH_DIR)' }
@{
    Elf       = Join-Path $preset_g2 'g2bench.elf'
    BuildDir  = $preset_g2
    Build     = { & (Join-Path $preset_g2 'build.ps1') -BbaTools $DevkitRoot }
    Watch     = @((Join-Path $preset_g2 'main.c'), (Join-Path $preset_g2 'g2b_core.c'), (Join-Path $preset_g2 'Makefile'))
    Exclusive = 'g2bench'
    PostRun   = {
        param($log)
        $r = Get-ChildItem (Join-Path $preset_g2 'host') -Filter 'g2bench_???.txt' -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
        if ($r) { Write-Host "[g2bench] newest result: $($r.FullName)"; Get-Content $r.FullName | Write-Host }
        else { Write-Host '[g2bench] no result file in host\ (see the console log above)' -ForegroundColor Yellow }
    }
}
