# dc_auto.ps1 preset: gdprobe (read-only GD-ROM probe, NetGD repo tools\gdprobe).
#   .\scripts\dc_auto.ps1 -Preset <full path to this file>
# Copy this file to the BBA tools' profiles\ folder to use the short form `-Preset gdprobe`.
#
# This file is dot-sourced by dc_auto.ps1: keep variable names prefixed (preset_*).
# The records are written to <gdprobe>\host\gdprobe_<label>_<date>.txt (host\gdprobe.cfg selects
# label / image switching). dc-tool-ip must run with its current directory on the same drive as
# the NetGD repository (a /pc/ rule of dcload-ip).
$preset_gd = Split-Path -Parent $PSScriptRoot      # ...\tools\gdprobe
@{
    Elf       = Join-Path $preset_gd 'gdprobe.elf'
    BuildDir  = $preset_gd
    # build.ps1 also creates host\ and passes HOST_DIR_REL, so it replaces a plain make
    Build     = { & (Join-Path $preset_gd 'build.ps1') }
    Watch     = @((Join-Path $preset_gd 'main.c'), (Join-Path $preset_gd 'probes.c'), (Join-Path $preset_gd 'gdraw.c'), (Join-Path $preset_gd 'Makefile'))
    # one Dreamcast, one drive: never run two probes at the same time
    Exclusive = 'gdprobe'
    # (no ReloadReq: the program ends by itself and returns to dcload)
    PostRun   = {
        param($log)
        $recs = Get-ChildItem (Join-Path $preset_gd 'host') -Filter 'gdprobe_*.txt' -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
        if ($recs) { Write-Host "[gdprobe] newest record: $($recs.FullName)" }
        else { Write-Host '[gdprobe] no record file found in host\ (see the console log above)' -ForegroundColor Yellow }
    }
}
