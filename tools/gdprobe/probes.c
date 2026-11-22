#include "probes.h"
#include <stdarg.h>
#include <stdio.h>
#include <string.h>
#include "hash.h"
#include "record.h"
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
    uint32_t leadout;
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
            uint32_t lo = fad3(r->buf + 405);
            if (r->buf[404] != 0xFF && lo && lo != 0xFFFFFF)
                r->leadout = lo;
            {   /* first data track (control bit 2) of this area, for the IP_READ fallback */
                int t, found = 0;
                for (t = 0; t < 99; t++) {
                    const uint8_t *e = r->buf + 4 * t;
                    uint32_t f = fad3(e + 1);
                    if (e[0] == 0xFF || f < 150 || f >= 0x00F00000)
                        continue;
                    r->toc_any = 1;
                    if (!(e[0] & 0x40)) {
                        if (!r->toc_audio)
                            r->toc_audio = f;
                    } else if (!found) {
                        r->toc_ip = f;
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
static void battery(runner *r, int full)
{
    int ready;
    r->ip_fad = 0;
    r->toc_ip = 0;
    r->toc_any = 0;
    r->toc_audio = 0;
    r->leadout = 0;
    ready = warmup(r);
    event(r, "WARMUP", "ready=%d", ready);
    battery_info(r, ready);
    sync_out(r);
    if (full && ready && !r->abort)
        battery_reads(r);
    sync_out(r);
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
            battery(r, 1);
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
        if (cfg->return_to > 0 && !r->abort)
            switch_to(r, cfg->return_to, cfg->return_to, 0, -1, SK_NORMAL);
    } else {
        r->img = 0;
        say(r, "probing the mounted image...");
        battery(r, 1);
    }
    event(r, "END", "abort=%d", r->abort);
    sync_out(r);
    return r->abort;
}
