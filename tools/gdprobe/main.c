/* gdprobe: read-only probe of the GD-ROM drive (stock GDEMU or NetGD).
 * Records raw replies and timings to /pc/ over the BBA (dcload-ip).
 * See README_ja.md. KOS-only glue; the logic is in probes.c (host-tested). */
#include <kos.h>
#include <dc/fs_dcload.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "cfg.h"
#include "gdraw.h"
#include "pcout.h"
#include "probes.h"
#include "record.h"

/* Leave the drive to gdraw alone. Without INIT_CDROM, KOS skips cdrom_init
 * (which sends its own INIT commands through the BIOS) and fs_iso9660 (whose
 * vblank handler calls cdrom_get_status every frame, i.e. a BIOS check-drive
 * between our commands). INIT_DEFAULT includes INIT_CDROM via
 * INIT_DEFAULT_ARCH (kos/init.h:141, arch/init_flags.h:79,90). /pc/ (dcload)
 * does not need it. */
KOS_INIT_FLAGS(INIT_DEFAULT & ~INIT_CDROM);

/* Folder on the PC as seen through dc-tool-ip's /pc/ (the build tooling sets
 * HOST_DIR_RAW, e.g. /pc/work/OpenGDEmu/tools/gdprobe/host). No spaces. */
#define STR_(x) #x
#define STR(x)  STR_(x)
#ifdef HOST_DIR_RAW
#define HOST_DIR STR(HOST_DIR_RAW)
#else
#define HOST_DIR "/pc"
#endif

static pcout g_out;
static int g_out_ok;

static void sink_line(void *ctx, const char *s, size_t len, int is_hash)
{
    (void)ctx;
    if (g_out_ok)
        pcout_write(&g_out, s, len);
    if (!is_hash) {
        /* console: the first 150 characters of each line */
        printf("%.150s%s", s, len > 150 ? "...\n" : "");
    }
}

static void sink_sync(void *ctx)
{
    (void)ctx;
    if (g_out_ok)
        pcout_sync(&g_out);
}

static void header(const char *text)
{
    char line[REC_LINE_MAX];
    size_t n = rec_comment_line(line, sizeof(line), text);
    sink_line(NULL, line, n, 0);
}

int main(int argc, char **argv)
{
    gp_config cfg;
    gd_ops ops;
    gp_sink sink;
    char cfgbuf[2048], path[256], stamp[40], h[400];
    FILE *f;
    time_t now;
    struct tm *tm;
    int rc, i, have_pc = (dcload_type != DCLOAD_TYPE_NONE);
    size_t n;
    (void)argc;
    (void)argv;

    printf("\n[gdprobe] version %s (read-only GD-ROM probe), KOS %s\n", GDPROBE_VERSION, KOS_VERSION_STRING);
    if (!have_pc)
        printf("[gdprobe] WARNING: not started from dcload: no /pc/, the record goes to the console only\n");

    /* configuration */
    gp_config_defaults(&cfg);
    cfgbuf[0] = 0;
    if (have_pc && (f = fopen(HOST_DIR "/gdprobe.cfg", "rb")) != NULL) {
        n = fread(cfgbuf, 1, sizeof(cfgbuf) - 1, f);
        cfgbuf[n] = 0;
        fclose(f);
        gp_config_parse(&cfg, cfgbuf);
        printf("[gdprobe] config: " HOST_DIR "/gdprobe.cfg (%u bytes, %d unknown lines)\n", (unsigned)n, cfg.unknown_keys);
    } else {
        printf("[gdprobe] no config file (" HOST_DIR "/gdprobe.cfg): defaults, no image switching\n");
    }

    /* output file: gdprobe_<label>_<date-time or seqN>.txt */
    now = time(NULL);
    tm = (now > 1577836800 /* 2020-01-01 */) ? gmtime(&now) : NULL;
    if (tm)
        snprintf(stamp, sizeof(stamp), "%04d%02d%02d-%02d%02d%02d", tm->tm_year + 1900, tm->tm_mon + 1,
                 tm->tm_mday, tm->tm_hour, tm->tm_min, tm->tm_sec);
    else
        snprintf(stamp, sizeof(stamp), "seq%05d", cfg.seq);
    snprintf(path, sizeof(path), HOST_DIR "/gdprobe_%s_%s.txt", cfg.label, stamp);

    g_out_ok = 0;
    if (have_pc) {
        if (pcout_open(&g_out, pcout_stdio_io(), path) == 0) {
            g_out_ok = 1;
            printf("[gdprobe] record: %s\n", path);
        } else {
            printf("[gdprobe] cannot open %s (does the folder exist on the PC?): console only\n", path);
        }
    }

    snprintf(h, sizeof(h), "gdprobe version=%s kos=%s label=%s stamp=%s seq=%d dcload=%d", GDPROBE_VERSION,
             KOS_VERSION_STRING, cfg.label, stamp, cfg.seq, dcload_type);
    header(h);
    n = (size_t)snprintf(h, sizeof(h), "config switch=%d identify=%d throughput=%d tp=%dx%d seek=%d/%d ip_fad=%lu cdread_flags=0x%02x timeout_ms=%lu return_to=%d images=",
                         cfg.do_switch, cfg.do_identify, cfg.do_throughput, cfg.tp_batches, cfg.tp_sectors,
                         cfg.do_seek, cfg.seek_n, (unsigned long)cfg.ip_fad, cfg.cdread_flags,
                         (unsigned long)cfg.timeout_ms, cfg.return_to);
    for (i = 0; i < cfg.n_images && n < sizeof(h) - 8; i++)
        n += (size_t)snprintf(h + n, sizeof(h) - n, "%s%d", i ? "," : "", cfg.images[i]);
    header(h);
    n = (size_t)snprintf(h, sizeof(h), "config swap_regpoll=%d regpoll_ms=%lu regpoll_interval_ms=%lu swap_devreset=%d devreset_ms=",
                         cfg.swap_regpoll, (unsigned long)cfg.swap_regpoll_ms, (unsigned long)cfg.swap_regpoll_interval_ms,
                         cfg.swap_devreset);
    for (i = 0; i < cfg.n_devreset && n < sizeof(h) - 8; i++)
        n += (size_t)snprintf(h + n, sizeof(h) - n, "%s%d", i ? "," : "", cfg.devreset_ms[i]);
    header(h);
    sink_sync(NULL);

    /* Without cdrom_init the G1 bus protection may still be closed: every
     * drive register then reads back the same value (run 1: status = irq
     * reason = 0xae, TUR timed out). */
    gdraw_bus_prepare(h, sizeof(h));
    header(h);
    sink_sync(NULL);

    gdraw_get_ops(&ops);
    sink.ctx = NULL;
    sink.line = sink_line;
    sink.sync = sink_sync;
    rc = gp_run(&ops, &cfg, &sink);

    snprintf(h, sizeof(h), "done abort=%d bytes_written=%lu", rc, g_out_ok ? (unsigned long)g_out.written + (unsigned long)g_out.fill : 0ul);
    header(h);
    if (g_out_ok) {
        if (pcout_close(&g_out) != 0)
            printf("[gdprobe] WARNING: writing the record failed\n");
    }
    printf("[gdprobe] finished%s. Returning to dcload.\n", rc ? " (ABORTED: the drive stopped answering)" : "");
    exit(0);
    return 0;
}
