#include "probes.h"
#include <stdarg.h>
#include <stdio.h>
#include <string.h>
#include "hash.h"
#include "record.h"
#include "toc.h"
#include "whitelist.h"

#define BUFSZ 40000   /* 16 sectors of 2352 = 37632 */

typedef struct {
    const gd_ops *ops;
    const gp_config *cfg;
    const gp_sink *sink;
    int img;
    int abort;
    int consec_timeouts;
    uint32_t ip_fad;     /* from REQ_SES(2) */
    uint32_t toc_ip;     /* fallback: first data track of the last valid TOC area */
    int toc_any;         /* some TOC area listed at least one valid track */
    uint32_t toc_audio;  /* first audio track of the first TOC area that has one (no-data-track images) */
    uint32_t toc_ip_end;     /* seq 21: start of the track after toc_ip in that area, else its lead-out (0 = unknown) */
    uint32_t toc_audio_end;  /* seq 21: the same for toc_audio */
    uint32_t leadout;
    gp_toc_area toc[2];  /* seq 22: both GET_TOC areas of the battery */
    char line[REC_LINE_MAX];
    char extra[160];
    uint8_t buf[BUFSZ];
} runner;

static runner g_r;   /* ~42 KB; single instance, the tool is single threaded */

/* ---- output ---------------------------------------------------------- */

static void out_line(runner *r, size_t len, int is_hash)
{
    if (len)
        r->sink->line(r->sink->ctx, r->line, len, is_hash);
}

static void say(runner *r, const char *msg)
{
    if (r->ops->progress)
        r->ops->progress(r->ops->ctx, msg);
}

static void event(runner *r, const char *name, const char *fmt, ...) __attribute__((format(printf, 3, 4)));
static void event(runner *r, const char *name, const char *fmt, ...)
{
    char f[200];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(f, sizeof(f), fmt, ap);
    va_end(ap);
    out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, name, f), 0);
}

static void sync_out(runner *r)
{
    if (r->sink->sync)
        r->sink->sync(r->sink->ctx);
}

/* ---- guarded command ---------------------------------------------------- */

/* Send one CDB through the whitelist, record it, return the result. The reply
 * data stays in r->buf. `data_in_record` = show the reply bytes in the line. */
/* The sending half of do_cmd (whitelist, drive, timeout accounting); writes no record line. */
static int send_cmd(runner *r, const uint8_t cdb[12], uint32_t cap, uint32_t timeout_ms, gd_result *res)
{
    memset(res, 0, sizeof(*res));
    if (r->abort) {
        res->rc = GD_REFUSED;
        return GD_REFUSED;
    }
    if (cap > BUFSZ)
        cap = BUFSZ;
    if (!wl_check_cfg(cdb, r->cfg->do_switch, r->cfg->cdread_flags)) {
        res->rc = GD_REFUSED;
    } else {
        r->ops->packet(r->ops->ctx, cdb, r->buf, cap, timeout_ms, res);
        if (res->rc == GD_TIMEOUT || res->rc == GD_BUSY) {
            if (++r->consec_timeouts >= 3) {
                r->abort = 1;
            }
        } else {
            r->consec_timeouts = 0;
        }
    }
    return res->rc;
}

static int do_cmd(runner *r, const char *name, const char *extra, const uint8_t cdb[12],
                  uint32_t cap, uint32_t timeout_ms, int data_in_record, gd_result *res)
{
    rec_probe p;
    if (r->abort) {
        memset(res, 0, sizeof(*res));
        res->rc = GD_REFUSED;
        return GD_REFUSED;
    }
    send_cmd(r, cdb, cap, timeout_ms, res);
    p.img = r->img;
    p.name = name;
    p.extra = extra;
    p.cdb = cdb;
    p.res = res;
    p.data = data_in_record ? r->buf : NULL;
    out_line(r, rec_probe_line(r->line, sizeof(r->line), &p), 0);
    if (r->abort && res->rc != GD_REFUSED) {
        event(r, "ABORT", "reason=drive-not-answering");
        say(r, "drive stopped answering: giving up");
    }
    return res->rc;
}

/* ---- CDB builders -------------------------------------------------------- */

static void cdb_simple(uint8_t c[12], uint8_t op, uint8_t b1, uint8_t b2, uint8_t alloc)
{
    memset(c, 0, 12);
    c[0] = op;
    c[1] = b1;
    c[2] = b2;
    c[4] = alloc;
}

static void cdb_be16(uint8_t c[12], uint8_t op, uint8_t b1, unsigned alloc)
{
    memset(c, 0, 12);
    c[0] = op;
    c[1] = b1;
    c[3] = (uint8_t)(alloc >> 8);
    c[4] = (uint8_t)alloc;
}

static void cdb_read(uint8_t c[12], uint8_t flags, uint32_t fad, uint32_t count)
{
    memset(c, 0, 12);
    c[0] = 0x30;
    c[1] = flags;
    c[2] = (uint8_t)(fad >> 16);
    c[3] = (uint8_t)(fad >> 8);
    c[4] = (uint8_t)fad;
    c[8] = (uint8_t)(count >> 16);
    c[9] = (uint8_t)(count >> 8);
    c[10] = (uint8_t)count;
}

static void cdb_mount(uint8_t c[12], int n)
{
    memset(c, 0, 12);
    c[0] = 0x52;
    c[1] = 0x82;
    c[2] = (uint8_t)n;
    c[3] = (uint8_t)(n >> 8);
}

/* ---- helpers ------------------------------------------------------------- */

static uint32_t fad3(const uint8_t *p) { return ((uint32_t)p[0] << 16) | ((uint32_t)p[1] << 8) | p[2]; }

/* seq 21: end (exclusive) of TOC entry t = the start of the next valid track after it, else the area lead-out */
static uint32_t toc_track_end(const uint8_t *toc, int t, uint32_t area_lo)
{
    uint32_t f = fad3(toc + 4 * t + 1);
    int u;
    for (u = t + 1; u < 99; u++) {
        const uint8_t *e = toc + 4 * u;
        uint32_t g = fad3(e + 1);
        if (e[0] == 0xFF || g < 150 || g >= 0x00F00000)
            continue;
        if (g > f)
            return g;
    }
    return area_lo > f ? area_lo : 0;
}

/* Hash each whole sector the drive returned. ssz = total / count (2048 for the
 * cooked data area, 2352 if the drive gave raw sectors); compare.py knows both. */
static void hash_sectors(runner *r, uint32_t fad, uint32_t count, const gd_result *res)
{
    uint32_t i, ssz;
    if (res->rc != GD_OK || !count || !res->total || res->total % count)
        return;
    ssz = res->total / count;
    for (i = 0; i < count; i++) {
        if ((i + 1) * ssz > res->stored)
            break;
        out_line(r, rec_hash_line(r->line, sizeof(r->line), r->img, fad + i,
                                  crc32_calc(r->buf + i * ssz, ssz), ssz), 1);
    }
}

static uint32_t read_timeout(runner *r) { return r->cfg->timeout_ms + 2000; }

/* ---- CDREAD_FLAGS --------------------------------------------------------- */

/* The same single sector (the IP-read FAD) read with CD_READ byte 1 = 0x20, 0x24,
 * 0x28, 0x10 in turn (FAD addressing). One line each:
 *   probe=CDREAD_FLAGS flags=0x28 fad=N count=1 ssz=2048|- crc=xxxxxxxx|- sense=none|sk/asc/ascq cdb=.. rc=..
 * ssz = bytes returned, crc = CRC-32 of them (both "-" unless rc=ok), sense from
 * REQ_ERROR (0x13) after a CHECK. A CHECK does not stop the battery. */
static void battery_cdread_flags(runner *r, uint32_t ip)
{
    static const uint8_t flags[4] = { 0x20, 0x24, 0x28, 0x10 };
    uint8_t c[12];
    gd_result res;
    char ex[120], sn[24], ssz[16], crc[16];
    int i;
    for (i = 0; i < 4 && !r->abort; i++) {
        rec_probe p;
        cdb_read(c, flags[i], ip, 1);
        send_cmd(r, c, BUFSZ, read_timeout(r), &res);
        strcpy(ssz, "-");
        strcpy(crc, "-");
        strcpy(sn, "none");
        if (res.rc == GD_OK && res.total && res.stored >= res.total) {
            snprintf(ssz, sizeof(ssz), "%lu", (unsigned long)res.total);
            snprintf(crc, sizeof(crc), "%08lx", (unsigned long)crc32_calc(r->buf, res.total));
        } else if (res.rc == GD_CHECK && !r->abort) {
            uint8_t sc[12];
            gd_result sr;
            cdb_simple(sc, 0x13, 0, 0, 10);
            send_cmd(r, sc, 10, r->cfg->timeout_ms, &sr);
            if (sr.rc == GD_OK && sr.stored >= 10)
                snprintf(sn, sizeof(sn), "%d/%02x/%02x", r->buf[2] & 0x0F, r->buf[8], r->buf[9]);
        }
        snprintf(ex, sizeof(ex), "flags=0x%02x fad=%lu count=1 ssz=%s crc=%s sense=%s", flags[i],
                 (unsigned long)ip, ssz, crc, sn);
        p.img = r->img; p.name = "CDREAD_FLAGS"; p.extra = ex; p.cdb = c; p.res = &res; p.data = NULL;
        out_line(r, rec_probe_line(r->line, sizeof(r->line), &p), 0);
        if (r->abort && res.rc != GD_REFUSED) {
            event(r, "ABORT", "reason=drive-not-answering");
            say(r, "drive stopped answering: giving up");
        }
    }
    sync_out(r);
}

/* ---- the battery ------------------------------------------------------- */

/* Wait (bounded) for the drive to come out of UNIT ATTENTION / spin-up. */
static int warmup(runner *r)
{
    uint8_t c[12];
    gd_result res;
    int i;
    char nm[24];
    for (i = 0; i < 8 && !r->abort; i++) {
        memset(c, 0, 12);
        snprintf(nm, sizeof(nm), "TUR_W%d", i);
        do_cmd(r, nm, NULL, c, 0, r->cfg->timeout_ms, 0, &res);
        if (res.rc == GD_OK)
            return 1;
        cdb_simple(c, 0x13, 0, 0, 10);
        snprintf(nm, sizeof(nm), "REQ_ERROR_W%d", i);
        do_cmd(r, nm, NULL, c, 10, r->cfg->timeout_ms, 1, &res);
        r->ops->sleep_ms(r->ops->ctx, 100);
    }
    return 0;
}

static void battery_info(runner *r, int ready)
{
    uint8_t c[12];
    gd_result res;
    char nm[24];
    int s;

    memset(c, 0, 12);
    do_cmd(r, "TUR", NULL, c, 0, r->cfg->timeout_ms, 0, &res);
    cdb_simple(c, 0x10, 0, 0, 10);
    do_cmd(r, "REQ_STAT", NULL, c, 10, r->cfg->timeout_ms, 1, &res);
    cdb_simple(c, 0x11, 0, 0, 10);
    do_cmd(r, "REQ_MODE_0_10", NULL, c, 10, r->cfg->timeout_ms, 1, &res);
    cdb_simple(c, 0x11, 0, 18, 8);
    do_cmd(r, "REQ_MODE_18_8", NULL, c, 8, r->cfg->timeout_ms, 1, &res);
    cdb_simple(c, 0x11, 0, 0, 32);
    do_cmd(r, "REQ_MODE_0_32", NULL, c, 32, r->cfg->timeout_ms, 1, &res);
    cdb_simple(c, 0x13, 0, 0, 10);
    do_cmd(r, "REQ_ERROR", NULL, c, 10, r->cfg->timeout_ms, 1, &res);

    /* TOC of both areas; remember the lead-out of the last valid one. */
    for (s = 0; s < 2; s++) {
        cdb_be16(c, 0x14, (uint8_t)s, 408);
        snprintf(nm, sizeof(nm), "GET_TOC_%d", s);
        do_cmd(r, nm, NULL, c, 408, r->cfg->timeout_ms, 1, &res);
        if (res.rc == GD_OK && res.stored >= 408) {
            uint32_t lo = fad3(r->buf + 405), area_lo = 0;
            gp_toc_parse(r->buf, &r->toc[s]);
            if (r->buf[404] != 0xFF && lo && lo != 0xFFFFFF)
                r->leadout = area_lo = lo;
            {   /* first data track (control bit 2) of this area, for the IP_READ fallback */
                int t, found = 0;
                for (t = 0; t < 99; t++) {
                    const uint8_t *e = r->buf + 4 * t;
                    uint32_t f = fad3(e + 1);
                    if (e[0] == 0xFF || f < 150 || f >= 0x00F00000)
                        continue;
                    r->toc_any = 1;
                    if (!(e[0] & 0x40)) {
                        if (!r->toc_audio) {
                            r->toc_audio = f;
                            r->toc_audio_end = toc_track_end(r->buf, t, area_lo);
                        }
                    } else if (!found) {
                        r->toc_ip = f;
                        r->toc_ip_end = toc_track_end(r->buf, t, area_lo);
                        found = 1;
                    }
                }
            }
        }
    }

    /* Sessions 0..3. Session 2 is where IP.BIN lives (45150 on a GD-ROM). */
    for (s = 0; s <= 3; s++) {
        cdb_simple(c, 0x15, 0, (uint8_t)s, 6);
        snprintf(nm, sizeof(nm), "REQ_SES_%d", s);
        do_cmd(r, nm, NULL, c, 6, r->cfg->timeout_ms, 1, &res);
        if (s == 2 && res.rc == GD_OK && res.stored >= 6) {
            uint32_t f = fad3(r->buf + 3);
            if (f >= 150 && f < 0x00F00000)
                r->ip_fad = f;
        }
    }

    cdb_be16(c, 0x40, 0, 100);
    do_cmd(r, "GET_SCD_0", NULL, c, 100, r->cfg->timeout_ms, 1, &res);
    cdb_be16(c, 0x40, 1, 14);
    do_cmd(r, "GET_SCD_1", NULL, c, 14, r->cfg->timeout_ms, 1, &res);

    memset(c, 0, 12);
    c[0] = 0x52; /* sub-code 00: version */
    do_cmd(r, "GDEMU_VERSION", NULL, c, 8, r->cfg->timeout_ms, 1, &res);
    (void)ready;
}

static void battery_reads(runner *r)
{
    uint8_t c[12];
    gd_result res;
    char nm[24], ex[96];
    uint32_t ip = r->cfg->ip_fad ? r->cfg->ip_fad : (r->ip_fad ? r->ip_fad : (r->toc_ip ? r->toc_ip : 45150));
    int i;

    /* An image whose TOC lists tracks but no data track (an audio-only CDI, R148 image 11): there is no IP.BIN
     * to read and REQ_SES / the 45150 fallback would point at nothing. Skip IP_READ, throughput and seek; only
     * the CD_READ flag battery runs, on the first audio track (stock returns 2352-byte PCM for 0x20 / 0x10 and
     * 5/64/00 for 0x24 / 0x28 on GDI audio, R144). An explicit ip_fad= still forces the normal reads. */
    if (!r->cfg->ip_fad && r->toc_any && !r->toc_ip) {
        event(r, "NO_DATA_TRACK", "audio_fad=%lu reads=skipped", (unsigned long)r->toc_audio);
        sync_out(r);
        if (r->cfg->do_cdread_flags && r->toc_audio && !r->abort) {
            say(r, "no data track: cd_read flags on the first audio track...");
            battery_cdread_flags(r, r->toc_audio);
        }
        return;
    }

    /* IP.BIN: the BIOS reads 7 sectors from here at boot. */
    snprintf(ex, sizeof(ex), "fad=%lu count=7", (unsigned long)ip);
    cdb_read(c, r->cfg->cdread_flags, ip, 7);
    do_cmd(r, "IP_READ", ex, c, BUFSZ, read_timeout(r), 1, &res);
    hash_sectors(r, ip, 7, &res);
    sync_out(r);

    if (r->cfg->do_cdread_flags && !r->abort) {
        say(r, "cd_read flags...");
        battery_cdread_flags(r, ip);
    }

    /* Throughput: tp_batches x tp_sectors sequential sectors, timed per batch. */
    if (r->cfg->do_throughput && !r->abort) {
        uint32_t n = (uint32_t)r->cfg->tp_sectors, bytes = 0, total_us = 0, min_us = 0xFFFFFFFFu, max_us = 0;
        int ok = 0, b;
        say(r, "throughput...");
        for (b = 0; b < r->cfg->tp_batches && !r->abort; b++) {
            uint32_t fad = ip + (uint32_t)b * n;
            if (r->leadout && fad + n > r->leadout)
                break;
            cdb_read(c, r->cfg->cdread_flags, fad, n);
            snprintf(nm, sizeof(nm), "TP_%03d", b);
            snprintf(ex, sizeof(ex), "fad=%lu count=%lu", (unsigned long)fad, (unsigned long)n);
            do_cmd(r, nm, ex, c, BUFSZ, read_timeout(r), 0, &res);
            if (res.rc == GD_OK) {
                ok++;
                bytes += res.total;
                total_us += (uint32_t)res.us;
                if (res.us < min_us) min_us = (uint32_t)res.us;
                if (res.us > max_us) max_us = (uint32_t)res.us;
                hash_sectors(r, fad, n, &res);
            } else if (res.rc != GD_CHECK) {
                break;
            }
            if ((b & 15) == 15)
                sync_out(r);
        }
        if (!ok)
            min_us = 0;
        event(r, "TP_SUMMARY", "ok=%d bytes=%lu us=%lu min_us=%lu max_us=%lu kib_s=%lu", ok,
              (unsigned long)bytes, (unsigned long)total_us, (unsigned long)min_us,
              (unsigned long)max_us,
              total_us ? (unsigned long)(((uint64_t)bytes * 1000000u) / total_us / 1024u) : 0ul);
        sync_out(r);
    }

    /* Seek sampling: single-sector reads alternating far/near, so every read
     * after the first needs a long seek. */
    if (r->cfg->do_seek && !r->abort) {
        int n = r->cfg->seek_n;
        uint32_t span = r->cfg->seek_span;
        say(r, "seek...");
        if (r->leadout > ip + 2 && r->leadout - ip - 1 < span)
            span = r->leadout - ip - 1;
        for (i = 0; i < n && !r->abort; i++) {
            int idx = (i % 2 == 0) ? i / 2 : n - 1 - i / 2;
            uint32_t fad = ip + (uint32_t)(((uint64_t)span * (uint32_t)idx) / (uint32_t)(n - 1));
            cdb_read(c, r->cfg->cdread_flags, fad, 1);
            snprintf(nm, sizeof(nm), "SEEK_%02d", i);
            snprintf(ex, sizeof(ex), "fad=%lu", (unsigned long)fad);
            do_cmd(r, nm, ex, c, BUFSZ, read_timeout(r), 0, &res);
            hash_sectors(r, fad, 1, &res);
        }
        sync_out(r);
    }
}

/* full = 0: information commands only, no reads (the image as mounted at start,
 * or an image that never became READY). */
static int battery(runner *r, int full)
{
    int ready;
    r->ip_fad = 0;
    r->toc_ip = 0;
    r->toc_any = 0;
    r->toc_audio = 0;
    r->toc_ip_end = 0;
    r->toc_audio_end = 0;
    r->leadout = 0;
    memset(r->toc, 0, sizeof(r->toc));
    ready = warmup(r);
    event(r, "WARMUP", "ready=%d", ready);
    battery_info(r, ready);
    sync_out(r);
    if (full && ready && !r->abort)
        battery_reads(r);
    sync_out(r);
    return ready;
}

/* ---- R148: the swap window without commands ----------------------------- */

static void wait_step(runner *r, uint32_t ms)
{
    if (r->ops->spin_us)
        r->ops->spin_us(r->ops->ctx, ms * 1000u);
    else
        r->ops->sleep_ms(r->ops->ctx, ms);
}

/* ATA DEVICE RESET inside the window, then one line with the registers right after it:
 *   img=3 probe=R50_SW_DEVRESET delay_ms=50 t_ms=50 rc=ok reset_us=2000 bsy_seen=0 altst=0x.. err=0x.. irr=0x.. secnr=0x.. bcl=0x14 bch=0xeb
 * Only reached with switch=on and swap_devreset=on (and the ATA whitelist agrees). */
static int swap_devreset(runner *r, const char *pfx, uint64_t t0, int delay_ms)
{
    gd_regs g;
    uint32_t us = 0;
    int bsy = 0, rc;
    char nm[32], f[220];
    uint64_t t_ms = (r->ops->now_us(r->ops->ctx) - t0) / 1000u;
    memset(&g, 0, sizeof(g));
    if (r->abort) {
        rc = GD_REFUSED;
    } else if (!r->ops->ata_reset || !wl_ata_check(0x08, r->cfg->do_identify,
                                                    r->cfg->do_switch && (r->cfg->swap_devreset || r->cfg->devreset_only || r->cfg->devreset_cmd))) {
        rc = GD_REFUSED;
    } else {
        rc = r->ops->ata_reset(r->ops->ctx, r->cfg->timeout_ms, &g, &us, &bsy);
        if (rc == GD_TIMEOUT || rc == GD_BUSY) {
            if (++r->consec_timeouts >= 3)
                r->abort = 1;
        } else {
            r->consec_timeouts = 0;
        }
    }
    snprintf(nm, sizeof(nm), "%sSW_DEVRESET", pfx);
    snprintf(f, sizeof(f), "delay_ms=%d t_ms=%lu rc=%s reset_us=%lu bsy_seen=%d altst=0x%02x err=0x%02x irr=0x%02x secnr=0x%02x bcl=0x%02x bch=0x%02x",
             delay_ms, (unsigned long)t_ms, rec_rc_name(rc), (unsigned long)us, bsy, g.altst, g.err, g.irr, g.secnr, g.bcl, g.bch);
    out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, nm, f), 0);
    if (r->abort && rc != GD_REFUSED) {
        event(r, "ABORT", "reason=drive-not-answering");
        say(r, "drive stopped answering: giving up");
    }
    return rc;
}

/* After `52 82 n`: send NO packet command until `until_ms` (from t0), only read the registers every
 * swap_regpoll_interval_ms and write one line per change:
 *   img=3 probe=SW_REGPOLL t_ms=12 sample=4 altst=0x50 err=0x00 irr=0x03 secnr=0x06
 * reset_at_ms >= 0: issue DEVICE RESET at that time (swap_devreset) and record every register again right
 * after it. Ends with  probe=SW_REGPOLL_END t_ms=.. samples=.. changes=.. stop=.. . Bounded by time AND by
 * an iteration cap, so a stuck clock cannot hang it. Returns the time spent (ms). */
static uint32_t swap_regpoll(runner *r, const char *pfx, uint64_t t0, uint32_t until_ms, int reset_at_ms)
{
    char nm[32], f[160];
    gd_regs g, last;
    int have_last = 0, samples = 0, changes = 0, reset_done = reset_at_ms < 0;
    uint32_t step = r->cfg->swap_regpoll_interval_ms ? r->cfg->swap_regpoll_interval_ms : 2;
    uint32_t iter = 0, max_iter = (until_ms / step) * 4u + 1000u;
    const char *why = "time";
    uint64_t t_ms = 0;

    snprintf(nm, sizeof(nm), "%sSW_REGPOLL", pfx);
    memset(&last, 0, sizeof(last));
    if (!r->ops->regs) {
        snprintf(nm, sizeof(nm), "%sSW_REGPOLL_END", pfx);
        out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, nm, "t_ms=0 samples=0 changes=0 stop=no-register-access"), 0);
        return 0;
    }
    for (;;) {
        if (r->abort) { why = "abort"; break; }
        t_ms = (r->ops->now_us(r->ops->ctx) - t0) / 1000u;
        if (!reset_done && t_ms >= (uint64_t)reset_at_ms) {
            swap_devreset(r, pfx, t0, reset_at_ms);
            reset_done = 1;
            have_last = 0;      /* the first sample after the reset is always written */
            continue;
        }
        if (t_ms >= until_ms)
            break;
        if (++iter > max_iter) { why = "iteration-cap"; break; }
        if (r->ops->regs(r->ops->ctx, &g)) { why = "bus-lock"; break; }
        samples++;
        if (!have_last || g.altst != last.altst || g.err != last.err || g.irr != last.irr || g.secnr != last.secnr) {
            changes++;
            snprintf(f, sizeof(f), "t_ms=%lu sample=%d altst=0x%02x err=0x%02x irr=0x%02x secnr=0x%02x",
                     (unsigned long)t_ms, samples, g.altst, g.err, g.irr, g.secnr);
            out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, nm, f), 0);
            last = g;
            have_last = 1;
        }
        wait_step(r, step);
    }
    snprintf(nm, sizeof(nm), "%sSW_REGPOLL_END", pfx);
    snprintf(f, sizeof(f), "t_ms=%lu samples=%d changes=%d stop=%s", (unsigned long)t_ms, samples, changes, why);
    out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, nm, f), 0);
    sync_out(r);
    return (uint32_t)t_ms;
}

/* ---- image switch -------------------------------------------------------- */

/* `52 82 n`, then TEST UNIT READY / REQ_ERROR every 20 ms. Only changes of
 * state are written to the record. A drive that never gets READY (or an empty
 * folder) is a recorded result, not an error. Returns 1 if READY.
 * regpoll = 1: the command-free register poll (swap_regpoll) runs first.
 * reset_at_ms >= 0: the R148 DEVICE RESET variant; every line's probe name gets the prefix R<ms>_ and
 * the register poll always runs (before and after the reset). */
#define SK_NORMAL 0
#define SK_RESWITCH 1      /* seq 17: `52 82 n` for the folder already mounted, no reset (RS_) */
#define SK_DEVRESET_ONLY 2 /* seq 17: no switch command, idle, DEVICE RESET (DR_) */

static int switch_to(runner *r, int n, int tag, int regpoll, int reset_at_ms, int kind)
{
    uint8_t c[12];
    gd_result res;
    char ex[96], pfx[16], nm[40];
    uint64_t t0, now;
    int ready = 0, polls = 0, changes = 0;
    int last_secnr = -1, saw_ua = 0;
    int last_rc = -1, last_st = -1, last_er = -1, last_sk = -1, last_asc = -1, last_ascq = -1;
    uint32_t timeout_ms_total;
    char msg[64];

    r->img = tag;
    if (kind == SK_RESWITCH)
        strcpy(pfx, "RS_");
    else if (kind == SK_DEVRESET_ONLY)
        strcpy(pfx, "DR_");
    else if (reset_at_ms >= 0)
        snprintf(pfx, sizeof(pfx), "R%d_", reset_at_ms);
    else
        pfx[0] = 0;
    if (kind == SK_RESWITCH)
        snprintf(msg, sizeof(msg), "re-switch to image %d (no reset)...", n);
    else if (kind == SK_DEVRESET_ONLY)
        snprintf(msg, sizeof(msg), "DEVICE RESET only (image %d)...", n);
    else if (reset_at_ms >= 0)
        snprintf(msg, sizeof(msg), "switch to image %d, DEVICE RESET at %d ms...", n, reset_at_ms);
    else
        snprintf(msg, sizeof(msg), "switch to image %d...", n);
    say(r, msg);
    if (kind == SK_DEVRESET_ONLY) {
        /* No switch command at all: 500 ms of silence first (recorded), then the reset at t = 0 */
        char f[48];
        r->ops->sleep_ms(r->ops->ctx, 500);
        snprintf(nm, sizeof(nm), "%sIDLE", pfx);
        snprintf(f, sizeof(f), "idle_ms=%d", 500);
        out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, nm, f), 0);
        memset(&res, 0, sizeof(res));
        res.rc = GD_OK;
        t0 = r->ops->now_us(r->ops->ctx);
    } else {
        cdb_mount(c, n);
        snprintf(ex, sizeof(ex), "n=%d", n);
        snprintf(nm, sizeof(nm), "%sSW_CMD", pfx);
        t0 = r->ops->now_us(r->ops->ctx);
        do_cmd(r, nm, ex, c, 0, r->cfg->timeout_ms, 0, &res);
    }

    timeout_ms_total = r->cfg->switch_timeout_ms;
    /* Only after a switch command the drive accepted: never reset or sit silent after a refused/failed one. */
    if (res.rc == GD_OK && !r->abort && (regpoll || reset_at_ms >= 0 || kind != SK_NORMAL)) {
        uint32_t until = r->cfg->swap_regpoll_ms;
        int rat = reset_at_ms;
        if (kind == SK_RESWITCH) {
            until = r->cfg->swap_reswitch_wait_ms;
        } else if (kind == SK_DEVRESET_ONLY) {
            until = r->cfg->devreset_only_wait_ms;
            rat = 0;
        } else if (reset_at_ms >= 0 && until < (uint32_t)reset_at_ms + 300u) {
            until = (uint32_t)reset_at_ms + 300u;
        }
        timeout_ms_total += swap_regpoll(r, pfx, t0, until, rat);
    }

    snprintf(nm, sizeof(nm), "%sSW_POLL", pfx);
    while (!r->abort) {
        int sk = -1, asc = -1, ascq = -1, secnr_changed = 0;
        uint64_t t_ms;
        gd_result tr;
        memset(c, 0, 12);
        now = r->ops->now_us(r->ops->ctx);
        t_ms = (now - t0) / 1000u;
        if (t_ms >= timeout_ms_total)
            break;
        /* Send TUR without writing a line for every poll: do_cmd always
         * records, so run it through a temporary silent path. */
        {
            memset(&tr, 0, sizeof(tr));
            if (!wl_check_cfg(c, r->cfg->do_switch, r->cfg->cdread_flags)) {
                tr.rc = GD_REFUSED;
            } else {
                r->ops->packet(r->ops->ctx, c, r->buf, 0, r->cfg->timeout_ms, &tr);
            }
            polls++;
            if (tr.rc == GD_TIMEOUT || tr.rc == GD_BUSY) {
                if (++r->consec_timeouts >= 3) { r->abort = 1; }
            } else {
                r->consec_timeouts = 0;
            }
            if (tr.rc == GD_CHECK) {
                uint8_t sc[12];
                gd_result sr;
                cdb_simple(sc, 0x13, 0, 0, 10);
                memset(&sr, 0, sizeof(sr));
                if (wl_check_cfg(sc, r->cfg->do_switch, r->cfg->cdread_flags))
                    r->ops->packet(r->ops->ctx, sc, r->buf, 10, r->cfg->timeout_ms, &sr);
                if (sr.rc == GD_OK && sr.stored >= 10) {
                    /* REQ_ERROR (SPI spec): byte 2 low nibble = sense key, 8 = ASC, 9 = ASCQ */
                    sk = r->buf[2] & 0x0F;
                    asc = r->buf[8];
                    ascq = r->buf[9];
                }
            }
            {
                int sn_now = tr.has_secnr ? tr.secnr : -1;
                if (sn_now != last_secnr)
                    secnr_changed = 1;
                last_secnr = sn_now;
            }
            if (sk == 6 && asc == 0x28 && ascq == 0)
                saw_ua = 1;
            if (secnr_changed || tr.rc != last_rc || tr.status != last_st || tr.error != last_er || sk != last_sk ||
                asc != last_asc || ascq != last_ascq) {
                char f[140];
                char sn[24], sec[16];
                changes++;
                secnr_changed = 0;
                if (sk < 0)
                    strcpy(sn, "none");
                else
                    snprintf(sn, sizeof(sn), "%d/%02x/%02x", sk, asc, ascq);
                if (tr.has_secnr)
                    snprintf(sec, sizeof(sec), " secnr=0x%02x", tr.secnr);
                else
                    sec[0] = 0;
                snprintf(f, sizeof(f), "t_ms=%lu poll=%d rc=%s status=0x%02x err=0x%02x sense=%s%s",
                         (unsigned long)t_ms, polls, rec_rc_name(tr.rc), tr.status, tr.error, sn, sec);
                out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, nm, f), 0);
                last_rc = tr.rc; last_st = tr.status; last_er = tr.error;
                last_sk = sk; last_asc = asc; last_ascq = ascq;
            }
        }
        if (tr.rc == GD_OK) {
            ready = 1;
            break;
        }
        r->ops->sleep_ms(r->ops->ctx, 20);
    }
    now = r->ops->now_us(r->ops->ctx);
    {
        char f[140];
        snprintf(nm, sizeof(nm), "%sSW_RESULT", pfx);
        snprintf(f, sizeof(f), "n=%d rc=%s t_ms=%lu polls=%d changes=%d", n, ready ? "ok" : (r->abort ? "abort" : "timeout"),
                 (unsigned long)((now - t0) / 1000u), polls, changes);
        if (kind != SK_NORMAL)      /* did a UNIT ATTENTION 6/28/00 (medium may have changed) come? */
            snprintf(f + strlen(f), sizeof(f) - strlen(f), " ua_28=%d", saw_ua);
        out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, nm, f), 0);
    }
    /* After a reset switch: the drive's state block and sense once it is READY (did the reset change the
     * pause / standby state or the head position?). Read-only, like the battery. */
    if ((reset_at_ms >= 0 || kind != SK_NORMAL) && !r->abort) {
        cdb_simple(c, 0x10, 0, 0, 10);
        snprintf(nm, sizeof(nm), "%sREQ_STAT", pfx);
        do_cmd(r, nm, NULL, c, 10, r->cfg->timeout_ms, 1, &res);
        cdb_simple(c, 0x13, 0, 0, 10);
        snprintf(nm, sizeof(nm), "%sREQ_ERROR", pfx);
        do_cmd(r, nm, NULL, c, 10, r->cfg->timeout_ms, 1, &res);
    }
    sync_out(r);
    return ready;
}

/* ---- seq 20: commands right after a DEVICE RESET -------------------------- */

/* One TEST UNIT READY (silent: no record line). If it fails with CHECK, the sense is fetched at once with
 * REQ_ERROR (sk/asc/ascq = -1 if none). */
static void dc_tur(runner *r, gd_result *tr, int *sk, int *asc, int *ascq)
{
    uint8_t c[12];
    *sk = *asc = *ascq = -1;
    memset(c, 0, 12);
    send_cmd(r, c, 0, r->cfg->timeout_ms, tr);
    if (tr->rc == GD_CHECK && !r->abort) {
        gd_result sr;
        cdb_simple(c, 0x13, 0, 0, 10);
        send_cmd(r, c, 10, r->cfg->timeout_ms, &sr);
        if (sr.rc == GD_OK && sr.stored >= 10) {
            *sk = r->buf[2] & 0x0F;
            *asc = r->buf[8];
            *ascq = r->buf[9];
        }
    }
}

/* "t_ms=.. rc=.. status=.. err=.. sense=sk/asc/ascq|none [secnr=..] [altst=..]" (altst: a register read right after) */
static void dc_tur_fields(runner *r, char *f, size_t cap, uint64_t t0, const gd_result *tr, int sk, int asc, int ascq)
{
    char sn[24], sec[16], alt[16];
    gd_regs g;
    uint64_t t_ms = (r->ops->now_us(r->ops->ctx) - t0) / 1000u;
    if (sk < 0)
        strcpy(sn, "none");
    else
        snprintf(sn, sizeof(sn), "%d/%02x/%02x", sk, asc, ascq);
    if (tr->has_secnr)
        snprintf(sec, sizeof(sec), " secnr=0x%02x", tr->secnr);
    else
        sec[0] = 0;
    alt[0] = 0;
    if (r->ops->regs && !r->abort) {
        memset(&g, 0, sizeof(g));
        if (!r->ops->regs(r->ops->ctx, &g))
            snprintf(alt, sizeof(alt), " altst=0x%02x", g.altst);
    }
    snprintf(f, cap, "t_ms=%lu rc=%s status=0x%02x err=0x%02x sense=%s%s%s", (unsigned long)t_ms, rec_rc_name(tr->rc),
             tr->status, tr->error, sn, sec, alt);
}

/* DEVICE RESET, `delay` ms of silence, then: TUR (DC<d>_CMD_TUR), REQ_STAT, REQ_ERROR, then TUR every 10 ms until
 * READY (DC<d>_CMD_POLL per change) for at most 2 s, and DC<d>_CMD_RESULT with the time to READY counted from the
 * reset command. Read-only. Stops at once if the run was aborted (3 timeouts in a row). */
#define DC_POLL_MAX_MS 2000u
static void devreset_cmd_run(runner *r, int n, int delay)
{
    char pfx[16], nm[40], f[200];
    gd_result tr, res;
    uint8_t c[12];
    uint64_t t0, tp;
    int sk, asc, ascq;
    int polls = 1, changes = 0, ready, iter = 0;
    int last_rc, last_st, last_er, last_sk, last_asc, last_ascq, last_sec;
    unsigned long ready_ms = 0, total_ms;

    r->img = n;
    snprintf(pfx, sizeof(pfx), "DC%d_", delay);
    snprintf(f, sizeof(f), "DEVICE RESET, command after %d ms (image %d)...", delay, n);
    say(r, f);
    r->ops->sleep_ms(r->ops->ctx, 500);
    snprintf(nm, sizeof(nm), "%sIDLE", pfx);
    out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, nm, "idle_ms=500"), 0);
    t0 = r->ops->now_us(r->ops->ctx);
    if (swap_devreset(r, pfx, t0, 0) == GD_REFUSED || r->abort)
        return;
    wait_step(r, (uint32_t)delay);

    dc_tur(r, &tr, &sk, &asc, &ascq);
    snprintf(nm, sizeof(nm), "%sCMD_TUR", pfx);
    snprintf(f, sizeof(f), "delay_ms=%d ", delay);
    dc_tur_fields(r, f + strlen(f), sizeof(f) - strlen(f), t0, &tr, sk, asc, ascq);
    out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, nm, f), 0);
    ready = (tr.rc == GD_OK);
    if (ready)
        ready_ms = (unsigned long)((r->ops->now_us(r->ops->ctx) - t0) / 1000u);
    last_rc = tr.rc; last_st = tr.status; last_er = tr.error; last_sk = sk; last_asc = asc; last_ascq = ascq;
    last_sec = tr.has_secnr ? tr.secnr : -1;

    if (!r->abort) {
        cdb_simple(c, 0x10, 0, 0, 10);
        snprintf(nm, sizeof(nm), "%sCMD_REQ_STAT", pfx);
        do_cmd(r, nm, NULL, c, 10, r->cfg->timeout_ms, 1, &res);
    }
    if (!r->abort) {
        cdb_simple(c, 0x13, 0, 0, 10);
        snprintf(nm, sizeof(nm), "%sCMD_REQ_ERROR", pfx);
        do_cmd(r, nm, NULL, c, 10, r->cfg->timeout_ms, 1, &res);
    }

    tp = r->ops->now_us(r->ops->ctx);
    snprintf(nm, sizeof(nm), "%sCMD_POLL", pfx);
    while (!ready && !r->abort && iter++ < 500) {
        int sec;
        if ((r->ops->now_us(r->ops->ctx) - tp) / 1000u >= DC_POLL_MAX_MS)
            break;
        r->ops->sleep_ms(r->ops->ctx, 10);
        dc_tur(r, &tr, &sk, &asc, &ascq);
        polls++;
        sec = tr.has_secnr ? tr.secnr : -1;
        if (tr.rc != last_rc || tr.status != last_st || tr.error != last_er || sk != last_sk || asc != last_asc ||
            ascq != last_ascq || sec != last_sec) {
            changes++;
            snprintf(f, sizeof(f), "poll=%d ", polls);
            dc_tur_fields(r, f + strlen(f), sizeof(f) - strlen(f), t0, &tr, sk, asc, ascq);
            out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, nm, f), 0);
            last_rc = tr.rc; last_st = tr.status; last_er = tr.error; last_sk = sk; last_asc = asc; last_ascq = ascq;
            last_sec = sec;
        }
        if (tr.rc == GD_OK) {
            ready = 1;
            ready_ms = (unsigned long)((r->ops->now_us(r->ops->ctx) - t0) / 1000u);
        }
    }
    total_ms = (unsigned long)((r->ops->now_us(r->ops->ctx) - t0) / 1000u);
    snprintf(nm, sizeof(nm), "%sCMD_RESULT", pfx);
    if (ready)
        snprintf(f, sizeof(f), "delay_ms=%d rc=ok ready=1 ready_ms=%lu t_ms=%lu polls=%d changes=%d", delay, ready_ms, total_ms,
                 polls, changes);
    else
        snprintf(f, sizeof(f), "delay_ms=%d rc=%s ready=0 ready_ms=- t_ms=%lu polls=%d changes=%d", delay,
                 r->abort ? "abort" : "timeout", total_ms, polls, changes);
    out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, nm, f), 0);
    sync_out(r);
}

/* ---- seq 21: where does the drive put the head after a command? ----------- */

/* REQ_STAT byte 0 low nibble (SPI spec 6.1) */
static const char *st_name(int s)
{
    static const char *const n[10] = { "busy", "pause", "standby", "play", "seek", "scan", "open", "nodisc", "retry", "error" };
    return (s >= 0 && s < 10) ? n[s] : NULL;
}

static void fmt_st(char *out, size_t cap, uint8_t b0)
{
    const char *nm = st_name(b0 & 0x0F);
    if (nm)
        snprintf(out, cap, "%s", nm);
    else
        snprintf(out, cap, "0x%02x", b0);
}

/* One silent REQ_STAT (10 bytes) into out. 1 = ok. */
static int hp_req_stat(runner *r, uint8_t out[10], int *rc)
{
    uint8_t c[12];
    gd_result res;
    cdb_simple(c, 0x10, 0, 0, 10);
    send_cmd(r, c, 10, r->cfg->timeout_ms, &res);
    *rc = res.rc;
    if (res.rc == GD_OK && res.stored >= 10) {
        memcpy(out, r->buf, 10);
        return 1;
    }
    return 0;
}

typedef struct {
    int have_prev;
    uint32_t prev;       /* pos of the previous HD_*_POS line */
    int abort_noted;
    const char *pfx;     /* line name prefix: "HD" (seq 21) or "GP" (seq 22) */
    uint32_t late_ms;    /* head_late_ms / gap_late_ms */
    int scd;             /* head_scd / gap_scd */
    int ext;             /* seq 22: also ctl (REQ_STAT byte 2), scd_ctl, scd_rel (GET_SCD bytes 4, 7..9) */
    int crc;             /* seq 23: crc= (CRC-32 of the returned bytes) and scrc= (per sector) on read lines */
    /* seq 24: the last two REQ_STAT answers and the last GET_SCD(1) answer of hp_pos (for GP_CTL) */
    int q_ok, pq_ok, qscd_ok;
    uint8_t q[10], pq[10], qscd[14];
} hp_state;

static void hp_note_abort(runner *r, hp_state *s)
{
    if (r->abort && !s->abort_noted) {
        s->abort_noted = 1;
        event(r, "ABORT", "reason=drive-not-answering");
        say(r, "drive stopped answering: giving up");
    }
}

/* After a case: REQ_STAT at once, GET_SCD(1) (head_scd), REQ_STAT again head_late_ms later. One line:
 *   img=3 probe=HD_R7_POS stat_rc=ok st=pause trk=3 idx=1 pos=76657 d_start=+7 d_end=+0 moved=1
 *         scd_rc=ok scd_trk=3 scd_idx=1 scd_pos=76657 late_rc=ok late_st=pause late_pos=76657 stat=.. scd=..
 * d_start / d_end (reads only) = pos - first FAD read / pos - (first FAD + count); moved = pos differs from the
 * previous HD_*_POS line (- for the first). Fields of a failed query are "-". */
static void hp_pos(runner *r, hp_state *s, const char *label, int is_read, uint32_t fad, uint32_t count,
                   const char *pre)
{
    uint8_t st[10], late[10], scd[14];
    int st_rc = GD_REFUSED, late_rc = GD_REFUSED, scd_rc = GD_REFUSED;
    int st_ok, late_ok = 0, scd_ok = 0;
    char f[600], nm[40], sn[16], hx[40];
    size_t n = 0;
    uint32_t pos = 0;

    if (r->abort)
        return;
    st_ok = hp_req_stat(r, st, &st_rc);
    if (s->scd && !r->abort) {
        uint8_t c[12];
        gd_result res;
        cdb_be16(c, 0x40, 1, 14);
        send_cmd(r, c, 14, r->cfg->timeout_ms, &res);
        scd_rc = res.rc;
        if (res.rc == GD_OK && res.stored >= 14) {
            memcpy(scd, r->buf, 14);
            scd_ok = 1;
        }
    }
    if (s->late_ms && !r->abort) {
        r->ops->sleep_ms(r->ops->ctx, s->late_ms);
        late_ok = hp_req_stat(r, late, &late_rc);
    }

#define HP_ADD(...) do { if (n < sizeof(f)) n += (size_t)snprintf(f + n, sizeof(f) - n, __VA_ARGS__); } while (0)
    if (pre && *pre)
        HP_ADD("%s ", pre);
    HP_ADD("stat_rc=%s", rec_rc_name(st_rc));
    if (st_ok) {
        pos = fad3(st + 5);
        fmt_st(sn, sizeof(sn), st[0]);
        HP_ADD(" st=%s trk=%u idx=%u pos=%lu", sn, st[3], st[4], (unsigned long)pos);
        if (s->ext)
            HP_ADD(" ctl=0x%02x", st[2]);
        if (is_read)
            HP_ADD(" d_start=%+ld d_end=%+ld", (long)pos - (long)fad, (long)pos - (long)(fad + count));
        if (s->have_prev)
            HP_ADD(" moved=%d", pos != s->prev);
        else
            HP_ADD(" moved=-");
    } else {
        HP_ADD(" st=- trk=- idx=- pos=-%s%s moved=-", s->ext ? " ctl=-" : "", is_read ? " d_start=- d_end=-" : "");
    }
    if (s->scd) {
        HP_ADD(" scd_rc=%s", rec_rc_name(scd_rc));
        if (scd_ok)
            HP_ADD(" scd_trk=%u scd_idx=%u scd_pos=%lu", scd[5], scd[6], (unsigned long)fad3(scd + 11));
        else
            HP_ADD(" scd_trk=- scd_idx=- scd_pos=-");
        if (s->ext && scd_ok)
            HP_ADD(" scd_ctl=0x%02x scd_rel=%lu", scd[4], (unsigned long)fad3(scd + 7));
        else if (s->ext)
            HP_ADD(" scd_ctl=- scd_rel=-");
    }
    if (s->late_ms) {
        HP_ADD(" late_rc=%s", rec_rc_name(late_rc));
        if (late_ok) {
            fmt_st(sn, sizeof(sn), late[0]);
            HP_ADD(" late_st=%s late_pos=%lu", sn, (unsigned long)fad3(late + 5));
        } else {
            HP_ADD(" late_st=- late_pos=-");
        }
    }
    if (st_ok) {
        rec_hex(hx, st, 10);
        HP_ADD(" stat=%s", hx);
    }
    if (scd_ok) {
        rec_hex(hx, scd, 14);
        HP_ADD(" scd=%s", hx);
    }
#undef HP_ADD
    if (st_ok) {
        s->prev = pos;
        s->have_prev = 1;
    }
    s->pq_ok = s->q_ok;
    memcpy(s->pq, s->q, 10);
    s->q_ok = st_ok;
    if (st_ok)
        memcpy(s->q, st, 10);
    s->qscd_ok = scd_ok;
    if (scd_ok)
        memcpy(s->qscd, scd, 14);
    snprintf(nm, sizeof(nm), "%s_%s_POS", s->pfx, label);
    out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, nm, f), 0);
    hp_note_abort(r, s);
}

/* One CD_READ case: the read (sense from REQ_ERROR at once if it is a CHECK), its line, then hp_pos.
 *   img=3 probe=HD_R7 flags=0x20 fad=76650 count=7 ssz=2048 sense=none cdb=.. rc=ok ... bytes=14336 us=.. */
/* The read and its line only (name = full line name, pre = optional leading fields); returns rc. */
static int hp_read_line(runner *r, hp_state *s, const char *nm, const char *pre, uint8_t flags, uint32_t fad,
                        uint32_t count)
{
    uint8_t c[12];
    gd_result res;
    rec_probe p;
    char ex[400], sn[24], ssz[16], cr[220];
    size_t cn = 0;
    if (r->abort)
        return GD_REFUSED;
    cdb_read(c, flags, fad, count);
    send_cmd(r, c, BUFSZ, read_timeout(r), &res);
    cr[0] = 0;
    if (s->crc) {   /* before REQ_ERROR reuses r->buf */
        uint32_t got = res.stored < res.total ? res.stored : res.total;
        if (got)
            cn += (size_t)snprintf(cr + cn, sizeof(cr) - cn, " crc=%08lx", (unsigned long)crc32_calc(r->buf, got));
        else
            cn += (size_t)snprintf(cr + cn, sizeof(cr) - cn, " crc=-");
        if (res.stored < res.total)
            cn += (size_t)snprintf(cr + cn, sizeof(cr) - cn, " stored=%lu", (unsigned long)res.stored);
        if (count > 1 && got && got == res.total && got % count == 0) {
            uint32_t k, z = got / count;
            cn += (size_t)snprintf(cr + cn, sizeof(cr) - cn, " scrc=");
            for (k = 0; k < count && cn < sizeof(cr) - 10; k++)
                cn += (size_t)snprintf(cr + cn, sizeof(cr) - cn, "%s%08lx", k ? "," : "",
                                       (unsigned long)crc32_calc(r->buf + k * z, z));
        }
    }
    strcpy(ssz, "-");
    strcpy(sn, "none");
    if (res.rc == GD_OK && res.total && count && res.total % count == 0)
        snprintf(ssz, sizeof(ssz), "%lu", (unsigned long)(res.total / count));
    else if (res.rc == GD_CHECK && !r->abort) {
        uint8_t sc[12];
        gd_result sr;
        cdb_simple(sc, 0x13, 0, 0, 10);
        send_cmd(r, sc, 10, r->cfg->timeout_ms, &sr);
        if (sr.rc == GD_OK && sr.stored >= 10)
            snprintf(sn, sizeof(sn), "%d/%02x/%02x", r->buf[2] & 0x0F, r->buf[8], r->buf[9]);
    }
    snprintf(ex, sizeof(ex), "%s%sflags=0x%02x fad=%lu count=%lu ssz=%s sense=%s%s", pre ? pre : "", pre ? " " : "",
             flags, (unsigned long)fad, (unsigned long)count, ssz, sn, cr);
    p.img = r->img; p.name = nm; p.extra = ex; p.cdb = c; p.res = &res; p.data = NULL;
    out_line(r, rec_probe_line(r->line, sizeof(r->line), &p), 0);
    hp_note_abort(r, s);
    return res.rc;
}

static int hp_read(runner *r, hp_state *s, const char *label, uint8_t flags, uint32_t fad, uint32_t count)
{
    char nm[40];
    int rc;
    if (r->abort)
        return GD_REFUSED;
    snprintf(nm, sizeof(nm), "%s_%s", s->pfx, label);
    rc = hp_read_line(r, s, nm, NULL, flags, fad, count);
    hp_pos(r, s, label, 1, fad, count, NULL);
    return rc;
}

/* A non-read command case (TUR, GET_TOC, REQ_SES): its normal line, then hp_pos. */
static void hp_cmd(runner *r, hp_state *s, const char *label, const uint8_t c[12], uint32_t cap)
{
    gd_result res;
    char nm[32];
    if (r->abort)
        return;
    snprintf(nm, sizeof(nm), "%s_%s", s->pfx, label);
    send_cmd(r, c, cap, r->cfg->timeout_ms, &res);
    {
        rec_probe p;
        p.img = r->img; p.name = nm; p.extra = NULL; p.cdb = c; p.res = &res; p.data = NULL;
        out_line(r, rec_probe_line(r->line, sizeof(r->line), &p), 0);
    }
    hp_note_abort(r, s);
    hp_pos(r, s, label, 0, 0, 0, NULL);
}

/* The seq 21 case list, after the image's battery (positions from the battery's TOC):
 *   BASE                          no command, only the position queries
 *   R<n>  for n in head_counts    head_flags read of n sectors at D + len*k/16 (k = 1, 2, ...)
 *   HIT                           1 sector inside the last R<n> range (a re-read, maybe from the buffer)
 *   MID7 / END7 / END1 / START7   7 sectors at the middle, 7 at E-40, 1 at E-2, 7 at D
 *   CHK24 / CHK28                 7 sectors with 0x24 / 0x28 at D + len*10/16, *12/16 (one is a type CHECK)
 *   TUR / TOC / SES               TEST UNIT READY, GET_TOC area 0 (408), REQ_SES session 1 (6)
 *   AUD1 / AUD7                   head_flags reads of 1 / 7 sectors on the first audio track (1/4, 1/2 in)
 *   IDLE                          head_idle_ms without a command, then the queries
 * D..E = first data track of the last TOC area (E = next track or lead-out); data cases need E - D >= 256,
 * audio cases a track of >= 32 sectors. Every read is FAD addressed, at most 16 sectors, inside one track. */
static void head_probe(runner *r)
{
    hp_state s;
    uint8_t c[12];
    char lb[16], f[160];
    uint32_t d = r->toc_ip, e = r->toc_ip_end, a = r->toc_audio, ae = r->toc_audio_end;
    uint8_t fl = r->cfg->head_flags;
    int data_ok, aud_ok, i;

    memset(&s, 0, sizeof(s));
    s.pfx = "HD";
    s.late_ms = r->cfg->head_late_ms;
    s.scd = r->cfg->head_scd;
    if (r->abort)
        return;
    data_ok = d && e > d && e - d >= 256;
    aud_ok = a && ae > a && ae - a >= 32;
    say(r, "head position probe...");
    snprintf(f, sizeof(f), "data_fad=%lu data_end=%lu audio_fad=%lu audio_end=%lu data_cases=%d audio_cases=%d",
             (unsigned long)d, (unsigned long)e, (unsigned long)a, (unsigned long)ae, data_ok, aud_ok);
    out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, "HD_START", f), 0);

    hp_pos(r, &s, "BASE", 0, 0, 0, NULL);
    if (data_ok) {
        uint32_t len = e - d, last_fad = d, last_n = 1;
        for (i = 0; i < r->cfg->n_head_counts && !r->abort; i++) {
            uint32_t n = (uint32_t)r->cfg->head_counts[i];
            last_fad = d + (uint32_t)(((uint64_t)len * (uint32_t)(i + 1)) / 16u);
            last_n = n;
            snprintf(lb, sizeof(lb), "R%lu", (unsigned long)n);
            hp_read(r, &s, lb, fl, last_fad, n);
        }
        hp_read(r, &s, "HIT", fl, last_fad + last_n / 2, 1);
        hp_read(r, &s, "MID7", fl, d + len / 2, 7);
        hp_read(r, &s, "END7", fl, e - 40, 7);
        hp_read(r, &s, "END1", fl, e - 2, 1);
        hp_read(r, &s, "START7", fl, d, 7);
        hp_read(r, &s, "CHK24", 0x24, d + (uint32_t)(((uint64_t)len * 10u) / 16u), 7);
        hp_read(r, &s, "CHK28", 0x28, d + (uint32_t)(((uint64_t)len * 12u) / 16u), 7);
    }
    memset(c, 0, 12);
    hp_cmd(r, &s, "TUR", c, 0);
    cdb_be16(c, 0x14, 0, 408);
    hp_cmd(r, &s, "TOC", c, 408);
    cdb_simple(c, 0x15, 0, 1, 6);
    hp_cmd(r, &s, "SES", c, 6);
    if (aud_ok) {
        uint32_t alen = ae - a;
        hp_read(r, &s, "AUD1", fl, a + alen / 4, 1);
        hp_read(r, &s, "AUD7", fl, a + alen / 2, 7);
    }
    if (r->cfg->head_idle_ms && !r->abort) {
        r->ops->sleep_ms(r->ops->ctx, r->cfg->head_idle_ms);
        snprintf(f, sizeof(f), "idle_ms=%lu", (unsigned long)r->cfg->head_idle_ms);
        hp_pos(r, &s, "IDLE", 0, 0, 0, f);
    }
    sync_out(r);
}

/* ---- seq 22: reads at the edges of the TOC -------------------------------- */

static void gp_skip(runner *r, const char *label, const char *fmt, ...) __attribute__((format(printf, 3, 4)));
static void gp_skip(runner *r, const char *label, const char *fmt, ...)
{
    char f[200], nm[40];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(f, sizeof(f), fmt, ap);
    va_end(ap);
    snprintf(nm, sizeof(nm), "GP_%s_SKIP", label);
    out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, nm, f), 0);
}

/* (1) the last readable sector of track A before track B: 1-sector reads (GP_SCAN_R lines), binary search.
 * Returns the first gap FAD (= last readable + 1), or 0 with *why set (no gap, unreadable start, abort). */
static uint32_t gp_find_gap(runner *r, hp_state *s, const char *nm, uint32_t lo, uint32_t hi, int *reads,
                            const char **why)
{
    uint8_t fl = r->cfg->gap_flags;
    uint32_t t;
    char pre[24];
    int rc;
    *reads = 0;
    snprintf(pre, sizeof(pre), "n=%d", ++*reads);
    rc = hp_read_line(r, s, nm, pre, fl, lo, 1);
    if (rc != GD_OK) {
        *why = r->abort ? "abort" : "track_start_unreadable";
        return 0;
    }
    snprintf(pre, sizeof(pre), "n=%d", ++*reads);
    rc = hp_read_line(r, s, nm, pre, fl, hi, 1);
    if (rc == GD_OK) {
        *why = "no_gap";
        return 0;
    }
    if (rc != GD_CHECK) {
        *why = r->abort ? "abort" : "scan_error";
        return 0;
    }
    while ((t = gp_bsearch_next(lo, hi)) != 0 && *reads < 48) {
        snprintf(pre, sizeof(pre), "n=%d", ++*reads);
        rc = hp_read_line(r, s, nm, pre, fl, t, 1);
        if (rc == GD_OK)
            lo = t;
        else if (rc == GD_CHECK)
            hi = t;
        else {
            *why = r->abort ? "abort" : "scan_error";
            return 0;
        }
    }
    if (gp_bsearch_next(lo, hi)) {
        *why = "scan_too_long";
        return 0;
    }
    return lo + 1;
}

/* The seq 22 case list, after the image's battery (and head_probe), FADs from both GET_TOC areas (toc.c):
 *   BASE                       no command, only the position queries
 *   TRK_*   (1) gap after track 1 (A) before the next track (B): GP_SCAN (+ GP_SCAN_R reads) finds the gap G..B-1,
 *           then T1 (A+50) -> GAPMID, T1END (G-1) -> GAPLO (G), T2 (B+16) -> GAPHI (B-1), T2B (B+32) -> GAPMID2
 *   LO_*    (2) REF (lead-out - 2), then LO_<o> at lead-out + o for o in gap_leadout
 *   HDG_*   (3) GD-ROM: REF (LD lead-out - 1), L0 (LD lead-out), MID, HI (HD start - 1), HD (HD start + 16), MID2
 *   LOW_*   (4) REF (reference FAD), then LOW_<f> at f for f in gap_low
 *   CNT0    (5) count 0 at the reference FAD: the whitelist allows 1..64, so normally GP_CNT0_SKIP (nothing sent);
 *           if it ever passes: CNT0_REF (reference + 8), then CNT0
 * Every read is 1 sector with gap_flags; after each one, REQ_STAT / GET_SCD(1) / late REQ_STAT (GP_<case>_POS). */
/* the seq 22 groups (gap_probe=on) */
static void gap_cases22(runner *r, hp_state *sp, const gp_gap_plan *pp)
{
    hp_state s;
    gp_gap_plan p = *pp;
    uint8_t fl = r->cfg->gap_flags;
    char f[300], lb[24];
    int i;
    s = *sp;
    /* (1) */
    if (!p.trk_ok) {
        gp_skip(r, "TRK", "reason=%s", p.trk_why);
    } else if (!r->abort) {
        uint32_t g = 0;
        int reads = 0;
        const char *why = "-", *src = "scan";
        if (r->cfg->gap_track_end) {
            src = "cfg";
            g = r->cfg->gap_track_end;
            if (g <= p.a_fad || g >= p.b_fad) {
                why = "gap_track_end_outside";
                g = 0;
            }
        } else {
            g = gp_find_gap(r, &s, "GP_SCAN_R", p.a_fad, p.b_fad - 1, &reads, &why);
        }
        if (!g) {
            if (!r->abort)
                gp_skip(r, "TRK", "a_fad=%lu b_fad=%lu src=%s reads=%d reason=%s", (unsigned long)p.a_fad,
                        (unsigned long)p.b_fad, src, reads, why);
        } else {
            uint32_t gh = p.b_fad - 1, mid = g + (gh - g) / 2, t1 = (p.a_fad + 50 < g) ? p.a_fad + 50 : p.a_fad;
            snprintf(f, sizeof(f), "a_fad=%lu b_fad=%lu last_ok=%lu gap_lo=%lu gap_hi=%lu gap_len=%lu gap_mid=%lu src=%s reads=%d",
                     (unsigned long)p.a_fad, (unsigned long)p.b_fad, (unsigned long)(g - 1), (unsigned long)g,
                     (unsigned long)gh, (unsigned long)(gh - g + 1), (unsigned long)mid, src, reads);
            out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, "GP_SCAN", f), 0);
            hp_read(r, &s, "TRK_T1", fl, t1, 1);
            hp_read(r, &s, "TRK_GAPMID", fl, mid, 1);
            hp_read(r, &s, "TRK_T1END", fl, g - 1, 1);
            hp_read(r, &s, "TRK_GAPLO", fl, g, 1);
            hp_read(r, &s, "TRK_T2", fl, p.b_fad + 16, 1);
            hp_read(r, &s, "TRK_GAPHI", fl, gh, 1);
            hp_read(r, &s, "TRK_T2B", fl, p.b_fad + 32, 1);
            hp_read(r, &s, "TRK_GAPMID2", fl, mid, 1);
        }
    }

    /* (2) */
    if (!p.lo_ok) {
        gp_skip(r, "LO", "leadout=%lu reason=%s", (unsigned long)p.leadout, p.lo_why);
    } else {
        hp_read(r, &s, "LO_REF", fl, p.leadout - 2, 1);
        for (i = 0; i < r->cfg->n_gap_leadout && !r->abort; i++) {
            snprintf(lb, sizeof(lb), "LO_%d", r->cfg->gap_leadout[i]);
            hp_read(r, &s, lb, fl, p.leadout + (uint32_t)r->cfg->gap_leadout[i], 1);
        }
    }

    /* (3) */
    if (!p.hd_ok) {
        gp_skip(r, "HDG", "reason=%s", p.hd_why);
    } else {
        uint32_t mid = p.ld_leadout + (p.hd_fad - p.ld_leadout) / 2;
        snprintf(f, sizeof(f), "ld_leadout=%lu hd_fad=%lu gap_lo=%lu gap_hi=%lu gap_mid=%lu", (unsigned long)p.ld_leadout,
                 (unsigned long)p.hd_fad, (unsigned long)p.ld_leadout, (unsigned long)(p.hd_fad - 1), (unsigned long)mid);
        out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, "GP_HDG", f), 0);
        hp_read(r, &s, "HDG_REF", fl, p.ld_leadout - 1, 1);
        hp_read(r, &s, "HDG_L0", fl, p.ld_leadout, 1);
        hp_read(r, &s, "HDG_MID", fl, mid, 1);
        hp_read(r, &s, "HDG_HI", fl, p.hd_fad - 1, 1);
        hp_read(r, &s, "HDG_HD", fl, p.hd_fad + 16, 1);
        hp_read(r, &s, "HDG_MID2", fl, mid, 1);
    }

    /* (4) */
    if (!p.ref_fad) {
        gp_skip(r, "LOW", "reason=no_toc");
    } else if (r->cfg->n_gap_low) {
        hp_read(r, &s, "LOW_REF", fl, p.ref_fad, 1);
        for (i = 0; i < r->cfg->n_gap_low && !r->abort; i++) {
            snprintf(lb, sizeof(lb), "LOW_%d", r->cfg->gap_low[i]);
            hp_read(r, &s, lb, fl, (uint32_t)r->cfg->gap_low[i], 1);
        }
    }

    /* (5) */
    if (r->cfg->gap_count0 && !r->abort) {
        uint8_t c[12];
        cdb_read(c, fl, p.ref_fad, 0);
        if (!p.ref_fad) {
            gp_skip(r, "CNT0", "reason=no_toc");
        } else if (!wl_check_cfg(c, r->cfg->do_switch, r->cfg->cdread_flags)) {
            char hx[30];
            rec_hex(hx, c, 12);
            gp_skip(r, "CNT0", "fad=%lu cdb=%s reason=whitelist_count_1_64", (unsigned long)p.ref_fad, hx);
        } else {
            hp_read(r, &s, "CNT0_REF", fl, p.ref_fad + 8, 1);
            hp_read(r, &s, "CNT0", fl, p.ref_fad, 0);
        }
    }
    *sp = s;
}

/* seq 23 / 24: the crossing reads that start on data and run past its end. Stock hung (BSY after the 8 real sectors)
 * at XEND in record 23; XLD and XLO have the same shape. warn = 1 (risky_cross=end): a GP_RISKY line, flushed, before
 * each one, so a hang is visible in the record. */
static void risky_note(runner *r, int warn, const char *cs, uint32_t fad, uint32_t count, uint32_t edge, const char *what)
{
    if (!warn || r->abort)
        return;
    event(r, "GP_RISKY", "case=%s fad=%lu count=%lu edge=%lu crosses=%s note=may_hang", cs, (unsigned long)fad,
          (unsigned long)count, (unsigned long)edge, what);
    sync_out(r);
}

/* 1-sector check of XGAP<t>'s first sector (GP_XGAP<t>_CHK, no position queries): 1 = it reads */
static int xgap_starts_on_data(runner *r, hp_state *s, uint32_t num, uint32_t fad)
{
    char nm[40];
    snprintf(nm, sizeof(nm), "GP_XGAP%lu_CHK", (unsigned long)num);
    return hp_read_line(r, s, nm, NULL, r->cfg->gap_flags, fad, 1) == GD_OK;
}

static void risky_xend(runner *r, hp_state *s, const gp_gap_plan *p, int warn)
{
    uint8_t fl = r->cfg->gap_flags;
    uint32_t cross = (uint32_t)r->cfg->pregap_cross, half = cross / 2;
    char f[300];
    if (!p->trk_ok) {
        gp_skip(r, "XEND", "reason=%s", p->trk_why);
    } else if (!r->abort) {
        uint32_t g = 0, hi = p->b_fad > 151 ? p->b_fad - 151 : 0;
        int reads = 0;
        const char *why = "-", *src = "scan";
        if (r->cfg->gap_track_end) {
            src = "cfg";
            g = r->cfg->gap_track_end;
            if (g <= p->a_fad || g >= p->b_fad) {
                why = "gap_track_end_outside";
                g = 0;
            }
        } else if (hi <= p->a_fad) {
            why = "short_track";
        } else {
            g = gp_find_gap(r, s, "GP_DSCAN_R", p->a_fad, hi, &reads, &why);
        }
        if (!g) {
            if (!r->abort)
                gp_skip(r, "XEND", "a_fad=%lu b_fad=%lu hi=%lu src=%s reads=%d reason=%s", (unsigned long)p->a_fad,
                        (unsigned long)p->b_fad, (unsigned long)hi, src, reads, why);
        } else {
            snprintf(f, sizeof(f), "a_fad=%lu b_fad=%lu hi=%lu last_ok=%lu gap_lo=%lu src=%s reads=%d", (unsigned long)p->a_fad,
                     (unsigned long)p->b_fad, (unsigned long)hi, (unsigned long)(g - 1), (unsigned long)g, src, reads);
            out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, "GP_DSCAN", f), 0);
            risky_note(r, warn, "XEND", g > half ? g - half : 0, cross, g, "data_end_into_gap");
            hp_read(r, s, "XEND", fl, g > half ? g - half : 0, cross);
        }
    }
}

static void risky_xld_xlo(runner *r, hp_state *s, const gp_gap_plan *p, int warn)
{
    uint8_t fl = r->cfg->gap_flags;
    uint32_t cross = (uint32_t)r->cfg->pregap_cross, half = cross / 2;
    if (p->hd_ok && !r->abort) {
        risky_note(r, warn, "XLD", p->ld_leadout - half, cross, p->ld_leadout, "ld_leadout");
        hp_read(r, s, "XLD", fl, p->ld_leadout - half, cross);
    }
    if (p->lo_ok && !r->abort) {
        risky_note(r, warn, "XLO", p->leadout - half, cross, p->leadout, "leadout");
        hp_read(r, s, "XLO", fl, p->leadout - half, cross);
    } else if (!p->lo_ok)
        gp_skip(r, "XLO", "reason=%s", p->lo_why);
}

/* warn = 0 (inline, seq 23): XEND, XLD, XLO. warn = 1 (risky_cross=end): the XGAP<t> that start on data (checked with
 * one sector first), XLD, XLO, then XEND (the one stock hung at) last. */
static void risky_reads(runner *r, hp_state *s, const gp_gap_plan *p, int warn)
{
    if (!warn) {
        risky_xend(r, s, p, 0);
        risky_xld_xlo(r, s, p, 0);
        return;
    }
    {
        static gp_track t[198];
        uint32_t cross = (uint32_t)r->cfg->pregap_cross;
        char lb[32];
        int nt = gp_toc_tracks(r->toc, t), i;
        for (i = 1; i < nt && !r->abort; i++) {
            if (t[i].fad < 158 || !xgap_starts_on_data(r, s, t[i].num, t[i].fad - 158))
                continue;
            snprintf(lb, sizeof(lb), "XGAP%lu", (unsigned long)t[i].num);
            risky_note(r, 1, lb, t[i].fad - 158, cross, t[i].fad - 150, "data_into_pregap");
            hp_read(r, s, lb, r->cfg->gap_flags, t[i].fad - 158, cross);
        }
    }
    risky_xld_xlo(r, s, p, 1);
    risky_xend(r, s, p, 1);
}

/* seq 23 (pregap_probe=on): what the drive returns in the pregaps and across boundaries (FADs from the TOC):
 *   GP_PG_START                 the merged track list
 *   PGL_<f>                     1 sector at each pregap_low FAD (track 1 pregap, below 150)
 *   GP_PGB, PG<t>_M<o> / _P<o>  for each track t after the first: 1 sector at start + o for o in pregap_offs
 *   XPRE<t>                     pregap_cross sectors from start - 10 (pregap into the track body; t = 1: FAD 140)
 *   XGAP<t>                     pregap_cross sectors from start - 158 (before the pregap into it), tracks after the first
 *   GP_DSCAN_R / GP_DSCAN, XEND track 1's data end below track 2's pregap (search, or gap_track_end), then
 *                               pregap_cross sectors from data end - cross/2 (data into the gap)
 *   XLD / XLO                   pregap_cross sectors from LD lead-out - cross/2 (GD-ROM) / lead-out - cross/2
 * Each read: sense, bytes, crc (gap_crc), then GP_<case>_POS. */
static void pregap_cases(runner *r, hp_state *s, const gp_gap_plan *p)
{
    static gp_track t[198];
    uint8_t fl = r->cfg->gap_flags;
    uint32_t fads[GP_MAX_PGOFFS], cross = (uint32_t)r->cfg->pregap_cross;
    char f[600], lb[32];
    size_t n = 0;
    int nt, i, j, m;

    if (r->abort)
        return;
    nt = gp_toc_tracks(r->toc, t);
    n += (size_t)snprintf(f + n, sizeof(f) - n, "cross=%lu tracks=", (unsigned long)cross);
    for (i = 0; i < nt && n < sizeof(f) - 40; i++)
        n += (size_t)snprintf(f + n, sizeof(f) - n, "%s%lu/%lu/0x%02lx", i ? "," : "", (unsigned long)t[i].num,
                              (unsigned long)t[i].fad, (unsigned long)t[i].ctl);
    if (!nt)
        n += (size_t)snprintf(f + n, sizeof(f) - n, "-");
    snprintf(f + n, sizeof(f) - n, " leadout=%lu ld_leadout=%lu", (unsigned long)p->leadout, (unsigned long)p->ld_leadout);
    out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, "GP_PG_START", f), 0);
    if (!nt) {
        gp_skip(r, "PG", "reason=no_toc");
        return;
    }

    /* (1) single sectors */
    for (i = 0; i < r->cfg->n_pregap_low && !r->abort; i++) {
        snprintf(lb, sizeof(lb), "PGL_%d", r->cfg->pregap_low[i]);
        hp_read(r, s, lb, fl, (uint32_t)r->cfg->pregap_low[i], 1);
    }
    for (i = 1; i < nt && !r->abort; i++) {
        m = gp_offset_fads(t[i].fad, r->cfg->pregap_offs, r->cfg->n_pregap_offs, fads);
        n = (size_t)snprintf(f, sizeof(f), "trk=%lu start=%lu prev=%lu/%lu fads=", (unsigned long)t[i].num,
                             (unsigned long)t[i].fad, (unsigned long)t[i - 1].num, (unsigned long)t[i - 1].fad);
        for (j = 0; j < m && n < sizeof(f) - 12; j++)
            n += (size_t)snprintf(f + n, sizeof(f) - n, "%s%lu", j ? "," : "", (unsigned long)fads[j]);
        out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, "GP_PGB", f), 0);
        for (j = 0; j < r->cfg->n_pregap_offs && !r->abort; j++) {
            int o = r->cfg->pregap_offs[j];
            long fa = (long)t[i].fad + o;
            if (fa < 0)
                continue;
            snprintf(lb, sizeof(lb), "PG%lu_%c%d", (unsigned long)t[i].num, o < 0 ? 'M' : 'P', o < 0 ? -o : o);
            hp_read(r, s, lb, fl, (uint32_t)fa, 1);
        }
    }

    /* (2) crossing reads */
    for (i = 0; i < nt && !r->abort; i++) {
        if (t[i].fad >= 10) {
            snprintf(lb, sizeof(lb), "XPRE%lu", (unsigned long)t[i].num);
            hp_read(r, s, lb, fl, t[i].fad - 10, cross);
        }
        if (i && t[i].fad >= 158) {
            snprintf(lb, sizeof(lb), "XGAP%lu", (unsigned long)t[i].num);
            /* seq 24: if its first sector reads, this one starts on data and runs into the pregap (like XEND) */
            if (r->cfg->risky_cross != GP_RISKY_INLINE && xgap_starts_on_data(r, s, t[i].num, t[i].fad - 158))
                gp_skip(r, lb, "fad=%lu reason=starts_on_data_%s", (unsigned long)(t[i].fad - 158),
                        r->cfg->risky_cross == GP_RISKY_END ? "deferred_to_end" : "risky_cross_off");
            else
                hp_read(r, s, lb, fl, t[i].fad - 158, cross);
        }
    }
    if (r->cfg->risky_cross == GP_RISKY_INLINE)
        risky_reads(r, s, p, 0);
    else
        gp_skip(r, "RISKY", "cases=XEND,XLD,XLO reason=%s", r->cfg->risky_cross == GP_RISKY_END ? "deferred_to_end" : "risky_cross_off");
}

/* seq 24 (ctl_probe=on): is the CTRL that REQ_STAT reports off-track (gap, lead-out) a constant 0x41, the CTRL of the
 * track read last, of the previous query, or of the next track (lead-out: the lead-out entry)? Per region (toc.c):
 *   GP_CTL_REGION                     the region's tracks and FADs
 *   CTLF_<R>_IN, CTLF_<R>_OFF<k>      forward: read the track before, then each off-track FAD
 *   CTLR_<R>_PRE, CTLR_<R>_OFF, CTLR_<R>_IN   reverse: read the track after (LO: the first track), the middle
 *                                     off-track FAD, then the track before again
 *   GP_CTL                            after each off-track read: what was reported and which hypotheses it matches */
static void ctl_verdict(runner *r, hp_state *s, const gp_ctl_region *g, const char *dir, uint32_t off, int rc,
                        uint32_t last_num, uint32_t last_ctl)
{
    char f[400], m[64];
    size_t n = 0, mn = 0;
    int ctl = s->q_ok ? s->q[2] : -1;
    if (r->abort)
        return;
    n += (size_t)snprintf(f + n, sizeof(f) - n, "region=%s dir=%s off_fad=%lu off_rc=%s", g->name, dir, (unsigned long)off,
                          rec_rc_name(rc));
    if (rc == GD_OK)
        n += (size_t)snprintf(f + n, sizeof(f) - n, " off_track=0");
    if (s->q_ok)
        n += (size_t)snprintf(f + n, sizeof(f) - n, " trk=%u idx=%u ctl=0x%02x", s->q[3], s->q[4], s->q[2]);
    else
        n += (size_t)snprintf(f + n, sizeof(f) - n, " trk=- idx=- ctl=-");
    if (s->qscd_ok)
        n += (size_t)snprintf(f + n, sizeof(f) - n, " scd_trk=%u scd_ctl=0x%02x scd_rel=%lu", s->qscd[5], s->qscd[4],
                              (unsigned long)fad3(s->qscd + 7));
    n += (size_t)snprintf(f + n, sizeof(f) - n, " last_read=%lu/0x%02lx", (unsigned long)last_num, (unsigned long)last_ctl);
    if (s->pq_ok)
        n += (size_t)snprintf(f + n, sizeof(f) - n, " prev_query=%u/0x%02x", s->pq[3], s->pq[2]);
    else
        n += (size_t)snprintf(f + n, sizeof(f) - n, " prev_query=-");
    if (g->has_next)
        n += (size_t)snprintf(f + n, sizeof(f) - n, " next=%lu/0x%02lx", (unsigned long)g->next_num, (unsigned long)g->next_ctl);
    else
        n += (size_t)snprintf(f + n, sizeof(f) - n, " next=- leadout_entry=0x%02lx", (unsigned long)g->next_ctl);
    m[0] = 0;
#define M_ADD(t) do { mn += (size_t)snprintf(m + mn, sizeof(m) - mn, "%s%s", mn ? "," : "", t); } while (0)
    if (ctl >= 0) {
        if (ctl == 0x41) M_ADD("const41");
        if ((uint32_t)ctl == last_ctl) M_ADD("last_read");
        if (s->pq_ok && ctl == s->pq[2]) M_ADD("prev_query");
        if ((uint32_t)ctl == g->next_ctl) M_ADD(g->has_next ? "next" : "leadout_entry");
    }
#undef M_ADD
    snprintf(f + n, sizeof(f) - n, " match=%s", mn ? m : "none");
    out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, "GP_CTL", f), 0);
}

static void ctl_cases(runner *r, hp_state *s)
{
    gp_gap_plan p;
    gp_ctl_region g[3];
    uint8_t fl = r->cfg->gap_flags;
    char f[300], lb[32];
    int ng, i, k, rc;

    if (r->abort)
        return;
    gp_gap_plan_make(r->toc, &p);
    ng = gp_ctl_regions(r->toc, &p, g);
    if (!ng) {
        gp_skip(r, "CTL", "reason=no_region");
        return;
    }
    for (i = 0; i < ng && !r->abort; i++) {
        size_t n = (size_t)snprintf(f, sizeof(f), "region=%s prev=%lu/0x%02lx in=%lu next=", g[i].name,
                                    (unsigned long)g[i].prev_num, (unsigned long)g[i].prev_ctl, (unsigned long)g[i].in_fad);
        if (g[i].has_next)
            n += (size_t)snprintf(f + n, sizeof(f) - n, "%lu/0x%02lx", (unsigned long)g[i].next_num, (unsigned long)g[i].next_ctl);
        else
            n += (size_t)snprintf(f + n, sizeof(f) - n, "- leadout_entry=0x%02lx", (unsigned long)g[i].next_ctl);
        n += (size_t)snprintf(f + n, sizeof(f) - n, " rev=%lu/0x%02lx/%lu off=", (unsigned long)g[i].rev_num,
                              (unsigned long)g[i].rev_ctl, (unsigned long)g[i].rev_in);
        for (k = 0; k < g[i].n_off && n < sizeof(f) - 12; k++)
            n += (size_t)snprintf(f + n, sizeof(f) - n, "%s%lu", k ? "," : "", (unsigned long)g[i].off[k]);
        out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, "GP_CTL_REGION", f), 0);

        /* forward: the track before, then each off-track FAD */
        snprintf(lb, sizeof(lb), "CTLF_%s_IN", g[i].name);
        hp_read(r, s, lb, fl, g[i].in_fad, 1);
        for (k = 0; k < g[i].n_off && !r->abort; k++) {
            snprintf(lb, sizeof(lb), "CTLF_%s_OFF%d", g[i].name, k);
            rc = hp_read(r, s, lb, fl, g[i].off[k], 1);
            ctl_verdict(r, s, &g[i], "fwd", g[i].off[k], rc, g[i].prev_num, g[i].prev_ctl);
        }
        /* reverse: the track after (or the first track), the middle off-track FAD, then the track before */
        {
            uint32_t mid = g[i].off[g[i].n_off / 2];
            snprintf(lb, sizeof(lb), "CTLR_%s_PRE", g[i].name);
            hp_read(r, s, lb, fl, g[i].rev_in, 1);
            snprintf(lb, sizeof(lb), "CTLR_%s_OFF", g[i].name);
            rc = hp_read(r, s, lb, fl, mid, 1);
            ctl_verdict(r, s, &g[i], "rev", mid, rc, g[i].rev_num, g[i].rev_ctl);
            snprintf(lb, sizeof(lb), "CTLR_%s_IN", g[i].name);
            hp_read(r, s, lb, fl, g[i].in_fad, 1);
        }
    }
}

/* seq 24 (risky_cross=end): the risky reads of the image now mounted, after every image's normal cases */
static void risky_phase(runner *r)
{
    hp_state s;
    gp_gap_plan p;
    memset(&s, 0, sizeof(s));
    s.pfx = "GP";
    s.late_ms = r->cfg->gap_late_ms;
    s.scd = r->cfg->gap_scd;
    s.ext = 1;
    s.crc = r->cfg->gap_crc;
    if (r->abort)
        return;
    gp_gap_plan_make(r->toc, &p);
    event(r, "GP_RISKY_START", "leadout=%lu ld_leadout=%lu a_fad=%lu b_fad=%lu cross=%d", (unsigned long)p.leadout,
          (unsigned long)p.ld_leadout, (unsigned long)p.a_fad, (unsigned long)p.b_fad, r->cfg->pregap_cross);
    hp_pos(r, &s, "RISKY_BASE", 0, 0, 0, NULL);
    risky_reads(r, &s, &p, 1);
    sync_out(r);
}

static void gap_probe(runner *r)
{
    hp_state s;
    gp_gap_plan p;
    char f[300];

    memset(&s, 0, sizeof(s));
    s.pfx = "GP";
    s.late_ms = r->cfg->gap_late_ms;
    s.scd = r->cfg->gap_scd;
    s.ext = 1;
    s.crc = r->cfg->gap_crc;
    if (r->abort)
        return;
    say(r, "TOC edge probe...");
    gp_gap_plan_make(r->toc, &p);
    snprintf(f, sizeof(f), "toc0=%d/%lu toc1=%d/%lu trk_a=%lu/%lu/0x%02lx trk_b=%lu/%lu/0x%02lx leadout=%lu last=%lu/%lu "
             "ld_leadout=%lu hd=%lu/%lu ref=%lu trk=%s lo=%s hdg=%s",
             r->toc[0].n, (unsigned long)r->toc[0].leadout, r->toc[1].n, (unsigned long)r->toc[1].leadout,
             (unsigned long)p.a_num, (unsigned long)p.a_fad, (unsigned long)p.a_ctl,
             (unsigned long)p.b_num, (unsigned long)p.b_fad, (unsigned long)p.b_ctl,
             (unsigned long)p.leadout, (unsigned long)p.last_num, (unsigned long)p.last_fad,
             (unsigned long)p.ld_leadout, (unsigned long)p.hd_num, (unsigned long)p.hd_fad, (unsigned long)p.ref_fad,
             p.trk_ok ? "ok" : p.trk_why, p.lo_ok ? "ok" : p.lo_why, p.hd_ok ? "ok" : p.hd_why);
    out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, "GP_START", f), 0);
    hp_pos(r, &s, "BASE", 0, 0, 0, NULL);

    if (r->cfg->gap_probe)
        gap_cases22(r, &s, &p);
    /* seq 23 */
    if (r->cfg->pregap_probe)
        pregap_cases(r, &s, &p);
    /* seq 24 */
    if (r->cfg->ctl_probe)
        ctl_cases(r, &s);
    sync_out(r);
}

/* ---- entry --------------------------------------------------------------- */

int gp_run(const gd_ops *ops, const gp_config *cfg, const gp_sink *sink)
{
    runner *r = &g_r;
    int i, j;
    memset(r, 0, sizeof(*r));
    r->ops = ops;
    r->cfg = cfg;
    r->sink = sink;

    if (cfg->do_identify && ops->identify && wl_ata_check(0xA1, cfg->do_identify, 0)) {
        gd_result res;
        rec_probe p;
        memset(&res, 0, sizeof(res));
        ops->identify(ops->ctx, r->buf, cfg->timeout_ms, &res);
        p.img = 0; p.name = "IDENTIFY_PACKET"; p.extra = NULL; p.cdb = NULL; p.res = &res; p.data = r->buf;
        out_line(r, rec_probe_line(r->line, sizeof(r->line), &p), 0);
        sync_out(r);
    }

    if (cfg->do_switch && cfg->n_images > 0) {
        char msg[48];
        /* The image mounted at start (dcload-ip's disc): information only. */
        say(r, "image as mounted at start...");
        r->img = 0;
        battery(r, 0);
        for (i = 0; i < cfg->n_images && !r->abort; i++) {
            int n = cfg->images[i];
            int ready = switch_to(r, n, n, cfg->swap_regpoll, -1, SK_NORMAL);
            snprintf(msg, sizeof(msg), "probing image %d%s...", n, ready ? "" : " (not ready)");
            say(r, msg);
            /* seq 21: the head-position cases, right after the battery (needs a READY image) */
            if (battery(r, 1) && !r->abort) {
                if (cfg->head_probe)
                    head_probe(r);
                /* seq 22: reads at the TOC's edges */
                if ((cfg->gap_probe || cfg->pregap_probe || cfg->ctl_probe) && !r->abort)
                    gap_probe(r);
            }
            /* R148: the same image again, once per delay, with a DEVICE RESET inside the window */
            for (j = 0; cfg->swap_devreset && j < cfg->n_devreset && !r->abort; j++)
                switch_to(r, n, n, 1, cfg->devreset_ms[j], SK_NORMAL);
            /* seq 17: the same folder again with no reset, then a reset with no switch command */
            if (cfg->swap_reswitch && !r->abort)
                switch_to(r, n, n, 1, -1, SK_RESWITCH);
            if (cfg->devreset_only && !r->abort)
                switch_to(r, n, n, 1, -1, SK_DEVRESET_ONLY);
            /* seq 20: commands at several delays after a DEVICE RESET */
            for (j = 0; cfg->devreset_cmd && j < cfg->n_devreset_cmd && !r->abort; j++)
                devreset_cmd_run(r, n, cfg->devreset_cmd_ms[j]);
        }
        /* seq 24: the reads that may hang the drive, after everything else, one image at a time */
        for (i = 0; cfg->pregap_probe && cfg->risky_cross == GP_RISKY_END && i < cfg->n_images && !r->abort; i++) {
            int n = cfg->images[i];
            snprintf(msg, sizeof(msg), "risky reads, image %d...", n);
            say(r, msg);
            switch_to(r, n, n, cfg->swap_regpoll, -1, SK_NORMAL);
            if (battery(r, 0) && !r->abort)
                risky_phase(r);
        }
        if (cfg->return_to > 0 && !r->abort)
            switch_to(r, cfg->return_to, cfg->return_to, 0, -1, SK_NORMAL);
    } else {
        r->img = 0;
        say(r, "probing the mounted image...");
        if (battery(r, 1) && !r->abort) {
            if (cfg->head_probe)
                head_probe(r);
            if ((cfg->gap_probe || cfg->pregap_probe || cfg->ctl_probe) && !r->abort)
                gap_probe(r);
            if (cfg->pregap_probe && cfg->risky_cross == GP_RISKY_END && !r->abort)
                risky_phase(r);
        }
    }
    event(r, "END", "abort=%d", r->abort);
    sync_out(r);
    return r->abort;
}
