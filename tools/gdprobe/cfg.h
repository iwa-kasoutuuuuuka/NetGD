/* gdprobe: configuration file parser (/pc/.../gdprobe.cfg). Pure C.
 *
 * Plain "key=value" lines; '#' starts a comment. Unknown keys are ignored
 * (and counted), so a typo cannot enable something by accident.
 *
 *   label=stock            device name for the file name and the record (stock | netgd | ...)
 *   seq=1                  run number, used in the file name when the Dreamcast has no valid clock
 *   switch=off             on = run the image-switch loop (default off)
 *   images=3,4,5,6,7       folder numbers to visit, in order (only with switch=on); ranges 3-5 ok
 *   return_to=2            folder to switch back to at the end (0/absent = no switch back)
 *   switch_timeout_ms=15000
 *   identify=off           ATA IDENTIFY PACKET DEVICE (0xA1), default off
 *   throughput=on          timed CD_READ throughput + per-sector hashes
 *   tp_batches=64          batches
 *   tp_sectors=16          sectors per batch (1..16)
 *   seek=on                seek-time sampling
 *   seek_n=16              samples (2..64)
 *   seek_span=50000        sectors from the IP.BIN area to spread the samples over
 *   ip_fad=0               0 = auto (REQ_SES 2, else 45150)
 *   cdread_flags_test=on   CDREAD_FLAGS battery: one sector read with CDB byte 1 = 0x20,0x24,0x28,0x10
 *   cdread_flags=0x24      CD_READ CDB byte 1 (bit 0 must stay 0 = FAD)
 *   timeout_ms=3000        per-command timeout
 *
 * R148 swap-window probes (only with switch=on; see README_ja.md):
 *   swap_regpoll=off       on = after each `52 82 n` of the image loop send NO command for swap_regpoll_ms and only
 *                          read ALTSTATUS / ERROR / interrupt reason / SECNR every swap_regpoll_interval_ms
 *                          (SW_REGPOLL line per change), then the usual TUR / REQ_ERROR polling
 *   swap_regpoll_ms=1000   length of that command-free poll (100..10000)
 *   swap_regpoll_interval_ms=2   poll step (1..50)
 *   swap_devreset=off      on = after each image's battery, switch to the same image again once per
 *                          swap_devreset_ms value and issue ATA DEVICE RESET (0x08) that many ms after `52 82 n`
 *                          (R<ms>_SW_* lines); the register poll runs before and after the reset
 *   swap_devreset_ms=50,200   reset delays (up to 4 values, 0..5000)
 *
 * seq 17 probes (only with switch=on; both default off, independent of each other):
 *   swap_reswitch=off      on = after each image's battery, `52 82 n` for the folder that is ALREADY mounted, then only
 *                          register polling for swap_reswitch_wait_ms with NO DEVICE RESET, then the usual TUR /
 *                          REQ_ERROR flow (RS_* lines; does a UNIT ATTENTION 6/28/00 come?)
 *   swap_reswitch_wait_ms=1000   (100..10000)
 *   devreset_only=off      on = after each image's battery, 500 ms idle, NO switch command, ATA DEVICE RESET, register
 *                          poll for devreset_only_wait_ms, then the TUR / REQ_ERROR flow (DR_* lines)
 *   devreset_only_wait_ms=1000   (100..10000)
 *
 * seq 20 probe (only with switch=on; default off):
 *   devreset_cmd=off       on = after each image's battery, once per devreset_cmd_ms value: 500 ms idle, ATA DEVICE RESET,
 *                          wait that many ms, then TEST UNIT READY / REQ_STAT / REQ_ERROR (what does the drive answer while
 *                          it re-detects?), then TUR every 10 ms until READY (max 2 s) (DC<ms>_* lines)
 *   devreset_cmd_ms=10,50,100    delays after the reset (up to 4 values, 0..5000)
 *
 * seq 21 probe (default off; with switch=on after each image's battery, with switch=off on the mounted image):
 *   head_probe=off         on = where does REQ_STAT / GET_SCD(1) put the head after each kind of command? A fixed list
 *                          of cases (CD_READ of 1..16 sectors at different FADs of the first data track, a re-read
 *                          inside the last range, start / middle / end of the track, 7-sector reads with 0x24 and 0x28
 *                          (one of them is a type CHECK), TUR, GET_TOC, REQ_SES, reads on the first audio track, an idle
 *                          wait); after each: REQ_STAT, GET_SCD format 1, REQ_STAT again after head_late_ms (HD_* lines)
 *   head_flags=0x20        CD_READ byte 1 of the plain reads (bit 0 forced 0; must pass the whitelist)
 *   head_counts=1,2,7,16   sector counts of the plain reads (up to 6 values, 1..16)
 *   head_late_ms=300       second REQ_STAT this long after each case (0 = none; 0..5000)
 *   head_idle_ms=2000      last case: no command for this long, then REQ_STAT (0 = skip; 0..10000)
 *   head_scd=on            GET_SCD format 1 after each case's first REQ_STAT
 *
 * seq 22 probe (default off; runs after head_probe, on the same images): reads at the edges of the TOC, FADs computed
 * from the image's TOC at run time (GP_* lines; every case prints its FADs, a case the image lacks prints GP_<case>_SKIP)
 *   gap_probe=off          on = the cases below; after each case REQ_STAT, GET_SCD(1), late REQ_STAT (like HD_*_POS,
 *                          plus ctl / scd_ctl / scd_rel)
 *   gap_flags=0x20         CD_READ byte 1 of every gap-probe read (bit 0 forced 0; must pass the whitelist)
 *   gap_late_ms=300        second REQ_STAT this long after each case (0 = none; 0..5000)
 *   gap_scd=on             GET_SCD format 1 after each case's first REQ_STAT
 *   gap_track_end=0        TRK cases: first FAD of the gap after track 1 (0 = find it: 1-sector reads, binary search
 *                          for the last readable sector between track 1 and track 2, GP_SCAN_R lines)
 *   gap_leadout=0,1,100    LO cases: offsets from the lead-out FAD (up to 4 values, 0..10000)
 *   gap_low=0,100          LOW cases: FADs below 150 (up to 4 values, 0..149)
 *   gap_count0=on          CNT0 case: CD_READ with count 0 (the whitelist allows 1..64, so this prints GP_CNT0_SKIP)
 *
 * seq 23 (default off; seq 22 cfgs are unchanged):
 *   gap_crc=off            on = every GP_ read line also gets crc= (CRC-32 of the returned bytes), scrc= (per sector,
 *                          multi-sector reads) and stored= (only when the buffer held less than bytes=)
 *   pregap_probe=off       on = pregap / boundary reads (runs the GP_ queries without gap_probe too): PGL_<f> at FADs
 *                          below 150, PG<trk>_M<o>/_P<o> at each later track's start + offset (GP_PGB lists them), and
 *                          reads that cross a boundary: XPRE<trk> (start - 10), XGAP<trk> (start - 158), XEND (data end of
 *                          track 1 found by a search below the pregap, GP_DSCAN_R / GP_DSCAN, or gap_track_end), XLD (GD-ROM
 *                          LD lead-out), XLO (lead-out); those start pregap_cross/2 before the edge
 *   pregap_low=0,1,100,149 PGL FADs (up to 4, 0..149)
 *   pregap_offs=-156,-151,-150,-149,-100,-2,-1,0   offsets from each track start (up to 8, -300..300)
 *   pregap_cross=16        sector count of the crossing reads (2..16; the whitelist allows 1..64, the buffer 16 raw)
 *
 * seq 24 (seq 23 cfgs are unchanged):
 *   risky_cross=inline     where the pregap_probe reads that start on data and run past its end (XEND into the gap,
 *                          XLD past the LD lead-out, XLO past the lead-out; stock hung at XEND in record 23) go:
 *                          inline = in each image's sequence (seq 23), end = after every image's cases, one image at a
 *                          time again (GP_RISKY line flushed before each read), off = not sent (GP_RISKY_SKIP)
 *   ctl_probe=off          on = CTRL off-track cases (CTLF_* forward / CTLR_* reverse + GP_CTL lines) per region from the
 *                          TOC (GAP, HDG, LO; toc.c gp_ctl_regions); 1-sector reads only
 */
#ifndef GDPROBE_CFG_H
#define GDPROBE_CFG_H
#include <stdint.h>

#define GP_MAX_IMAGES 32
#define GP_MAX_DEVRESET 4
#define GP_MAX_HEADCNT 6
#define GP_MAX_GAPLIST 4
#define GP_MAX_PGOFFS 8
#define GP_RISKY_INLINE 0
#define GP_RISKY_END 1
#define GP_RISKY_OFF 2

typedef struct {
    char label[32];
    int seq;
    int do_switch;
    int images[GP_MAX_IMAGES];
    int n_images;
    int return_to;
    uint32_t switch_timeout_ms;
    int do_identify;
    int do_throughput;
    int tp_batches;
    int tp_sectors;
    int do_cdread_flags;  /* CDREAD_FLAGS battery (default on) */
    int do_seek;
    int seek_n;
    uint32_t seek_span;
    uint32_t ip_fad;
    uint8_t cdread_flags;
    uint32_t timeout_ms;
    int swap_regpoll;     /* R148: command-free register poll after each image switch */
    uint32_t swap_regpoll_ms;
    uint32_t swap_regpoll_interval_ms;
    int swap_devreset;    /* R148: extra switches with an ATA DEVICE RESET inside the window */
    int devreset_ms[GP_MAX_DEVRESET];
    int n_devreset;
    int swap_reswitch;    /* seq 17: re-switch to the mounted folder, no DEVICE RESET */
    uint32_t swap_reswitch_wait_ms;
    int devreset_only;    /* seq 17: DEVICE RESET with no switch command */
    uint32_t devreset_only_wait_ms;
    int devreset_cmd;     /* seq 20: commands right after a DEVICE RESET (DC<ms>_) */
    int devreset_cmd_ms[GP_MAX_DEVRESET];
    int n_devreset_cmd;
    int head_probe;       /* seq 21: head position after reads (HD_) */
    uint8_t head_flags;
    int head_counts[GP_MAX_HEADCNT];
    int n_head_counts;
    uint32_t head_late_ms;
    uint32_t head_idle_ms;
    int head_scd;
    int gap_probe;        /* seq 22: reads at the TOC's edges (GP_) */
    uint8_t gap_flags;
    uint32_t gap_late_ms;
    int gap_scd;
    uint32_t gap_track_end;
    int gap_leadout[GP_MAX_GAPLIST];
    int n_gap_leadout;
    int gap_low[GP_MAX_GAPLIST];
    int n_gap_low;
    int gap_count0;
    int gap_crc;          /* seq 23 */
    int pregap_probe;
    int pregap_low[GP_MAX_GAPLIST];
    int n_pregap_low;
    int pregap_offs[GP_MAX_PGOFFS];
    int n_pregap_offs;
    int pregap_cross;
    int risky_cross;      /* seq 24: GP_RISKY_INLINE / _END / _OFF */
    int ctl_probe;
    int unknown_keys;     /* lines that did not parse */
} gp_config;

void gp_config_defaults(gp_config *c);
/* Parse NUL-terminated text over the defaults already in *c. Always succeeds. */
void gp_config_parse(gp_config *c, const char *text);

#endif
