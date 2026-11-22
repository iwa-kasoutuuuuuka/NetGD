# dc_auto.ps1 preset: gdprobe (read-only GD-ROM probe, NetGD repo tools\gdprobe).
#   .\scripts\dc_auto.ps1 -Preset <full path to this file>
# Do not copy this file into the BBA tools' profiles\ folder: it locates gdprobe from its own path
# (or from $env:GDPROBE_DIR if $PSCommandPath is empty).
#
# This file is dot-sourced by dc_auto.ps1: keep variable names prefixed (preset_*).
# The records are written to <gdprobe>\host\gdprobe_<label>_<date>.txt (host\gdprobe.cfg selects
# label / image switching). dc-tool-ip must run with its current directory on the same drive as
# the NetGD repository (a /pc/ rule of dcload-ip).
# This file's own path (not dc_auto's): profiles\ is one level below tools\gdprobe.
$preset_gd = if ($PSCommandPath) { Split-Path -Parent (Split-Path -Parent $PSCommandPath) }   # ...\tools\gdprobe
             elseif ($env:GDPROBE_DIR) { $env:GDPROBE_DIR }
             else { throw 'gdprobe preset: cannot locate tools\gdprobe (use -Preset <full path>, or set $env:GDPROBE_DIR)' }
@{
    Elf       = Join-Path $preset_gd 'gdprobe.elf'
    BuildDir  = $preset_gd
    # build.ps1 also creates host\ and passes HOST_DIR_REL, so it replaces a plain make
    Build     = { & (Join-Path $preset_gd 'build.ps1') -BbaTools $DevkitRoot }
    Watch     = @((Join-Path $preset_gd 'main.c'), (Join-Path $preset_gd 'probes.c'), (Join-Path $preset_gd 'gdraw.c'), (Join-Path $preset_gd 'toc.c'), (Join-Path $preset_gd 'Makefile'))
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
