# dc_auto.ps1 preset: netgd (send a game image to the NetGD SD card over the BBA, transport T1).
#
#   $env:NETGD_IMAGE = 'E:\games\crazytaxi\disc.gdi'     # required: a .gdi or .cdi
#   $env:NETGD_SLOT  = '2'                                # or NETGD_NAME = 'CRAZYTAXI' (ASCII); default: file name
#   .\scripts\dc_auto.ps1 -Preset <full path to this file>
# Use it by full path; don't copy it into the BBA tools' profiles\ folder (its paths are relative to this repo).
#
# Needs the Dreamcast app tools\netgd-recv\netgd-recv.elf: build it with tools\netgd-recv\build.ps1,
# or take it from the KOS build CI artifact `dreamcast-elfs`. Prepare = netgd_send prepare,
# PostRun = netgd_send collect. Both halves are host-tested; neither has run on hardware yet.
#
# Rules this preset relies on (dc-tool-ip /pc/ rules, KNOWHOW_ja.md):
#   - dc-tool-ip must run with its current directory on the SAME DRIVE as the job folder and the
#     image; the job folder is hard-linking the track files, so image and job must be on one NTFS volume.
#   - paths contain no spaces and no Japanese (the tool refuses otherwise).
#
# Optional environment variables (defaults in brackets):
#   NETGD_UNIT [dc1]     job folder = <drive of this repo>:\netgd\<unit>
#   NETGD_JOB            full job folder, overrides the above
#   NETGD_MODE [resume]  new | resume | overwrite  (put mode; resume also works for a fresh image)
#   NETGD_MOUNT [0]      1 = mount the image when finished
#   NETGD_PYTHON [python]
#
# This file is dot-sourced by dc_auto.ps1: keep variable names prefixed (preset_*).
$preset_netgd = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)   # ...\tools\netgd-send
$preset_repo  = Split-Path -Parent (Split-Path -Parent $preset_netgd)   # NetGD repository root
$preset_unit  = if ($env:NETGD_UNIT) { $env:NETGD_UNIT } else { 'dc1' }
$preset_job   = if ($env:NETGD_JOB) { $env:NETGD_JOB } else { Join-Path ([System.IO.Path]::GetPathRoot($preset_repo)) "netgd\$preset_unit" }
$preset_py    = if ($env:NETGD_PYTHON) { $env:NETGD_PYTHON } else { 'python' }
$preset_cli   = Join-Path $preset_netgd 'netgd-send.py'
@{
    Elf       = Join-Path $preset_repo 'tools\netgd-recv\netgd-recv.elf'

    # (no Build: the ELF is built separately with KallistiOS)

    # Before the run: validate the image and create the job folder (manifest, src.txt, hard links).
    Prepare   = {
        if (-not $env:NETGD_IMAGE) { throw 'Set $env:NETGD_IMAGE to the .gdi or .cdi to send.' }
        $a = @($preset_cli, 'prepare', $env:NETGD_IMAGE, '--job', $preset_job)
        if ($env:NETGD_SLOT) { $a += @('--slot', $env:NETGD_SLOT) } elseif ($env:NETGD_NAME) { $a += @('--name', $env:NETGD_NAME) }
        $a += @('--mode', $(if ($env:NETGD_MODE) { $env:NETGD_MODE } else { 'resume' }))
        if ($env:NETGD_MOUNT -eq '1') { $a += '--mount' }
        & $preset_py @a | Out-Host
        if ($LASTEXITCODE -ne 0) { throw "netgd-send prepare failed ($LASTEXITCODE)" }
    }

    # netgd-recv looks at reload.req / cancel.req once a second, finishes the current chunk,
    # leaves a checkpoint and returns to dcload with exit(0). Never kill dc-tool-ip yourself (noexec).
    ReloadReq = Join-Path $preset_job 'reload.req'

    # one job folder per unit, and one SD card write at a time
    Exclusive = "netgd-$preset_unit"

    # After every run: read result.ngr, print done / resumed-from / failed chunk.
    PostRun   = {
        param($log)
        & $preset_py $preset_cli collect --job $preset_job | Out-Host
        switch ($LASTEXITCODE) {
            0 { Write-Host '[netgd] image written and verified.' -ForegroundColor Green }
            2 { Write-Host '[netgd] incomplete: run again (the app resumes from its checkpoint).' -ForegroundColor Yellow }
            default { Write-Host '[netgd] FAILED: see the lines above.' -ForegroundColor Red }
        }
    }
}
