# dc_auto.ps1 preset: g2netbench (phase D of docs/research/g2-perf-plan.md; no GD-ROM access).
#   .\scripts\dc_auto.ps1 -Preset <full path to this file> [-Ip 192.168.x.y]
# The Dreamcast runs g2netbench.elf with KOS's own TCP/IP stack (it takes the BBA over from dcload-ip). The PC
# sender g2n_send.py is started by Prepare (which an Exclusive preset runs only once the Dreamcast is ours, just
# before the ELF is uploaded): it keeps trying to connect to <Dreamcast IP>:20039 until the ELF is up, serves the
# test data and writes <g2bench>\host\g2net_NNN.txt. PostRun waits for it and shows its log and the result.
# Needs Python 3 on the PC. The Dreamcast IP is dc_auto's ($dcIp: -Ip or config\bba.config.ps1); $env:G2NET_DC_IP overrides.
$preset_g2 = if ($PSCommandPath) { Split-Path -Parent (Split-Path -Parent $PSCommandPath) }
             elseif ($env:G2BENCH_DIR) { $env:G2BENCH_DIR }
             else { throw 'g2netbench preset: cannot locate tools\g2bench (use -Preset <full path>, or set $env:G2BENCH_DIR)' }
@{
    Elf       = Join-Path $preset_g2 'g2netbench.elf'
    BuildDir  = $preset_g2
    Build     = { & (Join-Path $preset_g2 'build.ps1') -BbaTools $DevkitRoot -Net }
    Watch     = @((Join-Path $preset_g2 'main_net.c'), (Join-Path $preset_g2 'g2n_core.c'), (Join-Path $preset_g2 'g2n_core.h'), (Join-Path $preset_g2 'g2x_core.c'), (Join-Path $preset_g2 'g2x_core.h'), (Join-Path $preset_g2 'Makefile'))
    Exclusive = 'g2bench'      # the same name as the g2bench preset: one of them at a time (shared host\ folder)
    Prepare   = {
        $hostDir = Join-Path $preset_g2 'host'
        New-Item -ItemType Directory -Force $hostDir | Out-Null
        $py = @((Get-Command python -ErrorAction SilentlyContinue), (Get-Command py -ErrorAction SilentlyContinue)) | Where-Object { $_ } | Select-Object -First 1
        $target = if ($env:G2NET_DC_IP) { $env:G2NET_DC_IP } else { $dcIp }
        if (-not $py) { Write-Host '[g2netbench] python not found: start g2n_send.py yourself' -ForegroundColor Yellow; return }
        if (-not $target) { Write-Host '[g2netbench] Dreamcast IP unknown: set $env:G2NET_DC_IP' -ForegroundColor Yellow; return }
        # a sender left over from an earlier run would take the Dreamcast's connection
        if ($global:g2n_proc -and -not $global:g2n_proc.HasExited) { Stop-Process -Id $global:g2n_proc.Id -Force -ErrorAction SilentlyContinue }
        $global:g2n_log = Join-Path $hostDir 'g2n_send_last.log'
        $args2 = @('-u', (Join-Path $preset_g2 'g2n_send.py'), '--host', $target, '--out-dir', $hostDir, '--wait', '900')
        if ($py.Name -eq 'py.exe') { $args2 = @('-3') + $args2 }
        $global:g2n_proc = Start-Process -FilePath $py.Source -ArgumentList ($args2 | ForEach-Object { '"' + $_ + '"' }) `
            -WindowStyle Hidden -PassThru -RedirectStandardOutput $global:g2n_log -RedirectStandardError ($global:g2n_log + '.err')
        Write-Host "[g2netbench] PC sender started (pid $($global:g2n_proc.Id)) for $target; it connects once the ELF runs"
    }
    PostRun   = {
        param($log)
        if ($global:g2n_proc) {
            if (-not $global:g2n_proc.HasExited) { $global:g2n_proc.WaitForExit(15000) | Out-Null }
            if (-not $global:g2n_proc.HasExited) {
                Write-Host '[g2netbench] the PC sender is still running (the Dreamcast never connected?): stopping it' -ForegroundColor Yellow
                Stop-Process -Id $global:g2n_proc.Id -Force -ErrorAction SilentlyContinue
            }
        }
        if ($global:g2n_log -and (Test-Path $global:g2n_log)) {
            Get-Content $global:g2n_log | Write-Host
            if ((Get-Item ($global:g2n_log + '.err') -ErrorAction SilentlyContinue).Length -gt 0) { Get-Content ($global:g2n_log + '.err') | Write-Host -ForegroundColor Yellow }
        }
        $r = Get-ChildItem (Join-Path $preset_g2 'host') -Filter 'g2net_???.txt' -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
        if ($r) { Write-Host "[g2netbench] newest result: $($r.FullName)"; Get-Content $r.FullName | Write-Host }
        else { Write-Host '[g2netbench] no result file in host\ (see the console log above)' -ForegroundColor Yellow }
    }
}
