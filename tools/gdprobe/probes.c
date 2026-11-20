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
static int do_cmd(runner *r, const char *name, const char *extra, const uint8_t cdb[12],
                  uint32_t cap, uint32_t timeout_ms, int data_in_record, gd_result *res)
{
    rec_probe p;
    memset(res, 0, sizeof(*res));
    if (r->abort) {
        res->rc = GD_REFUSED;
        return GD_REFUSED;
    }
    if (cap > BUFSZ)
        cap = BUFSZ;
    if (!wl_check(cdb, r->cfg->do_switch)) {
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
                int t;
                for (t = 0; t < 99; t++) {
                    const uint8_t *e = r->buf + 4 * t;
                    uint32_t f = fad3(e + 1);
                    if (e[0] != 0xFF && (e[0] & 0x40) && f >= 150 && f < 0x00F00000) {
                        r->toc_ip = f;
                        break;
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

    /* IP.BIN: the BIOS reads 7 sectors from here at boot. */
    snprintf(ex, sizeof(ex), "fad=%lu count=7", (unsigned long)ip);
    cdb_read(c, r->cfg->cdread_flags, ip, 7);
    do_cmd(r, "IP_READ", ex, c, BUFSZ, read_timeout(r), 1, &res);
    hash_sectors(r, ip, 7, &res);
    sync_out(r);

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
    r->leadout = 0;
    ready = warmup(r);
    event(r, "WARMUP", "ready=%d", ready);
    battery_info(r, ready);
    sync_out(r);
    if (full && ready && !r->abort)
        battery_reads(r);
    sync_out(r);
}

/* ---- image switch -------------------------------------------------------- */

/* `52 82 n`, then TEST UNIT READY / REQ_ERROR every 20 ms. Only changes of
 * state are written to the record. A drive that never gets READY (or an empty
 * folder) is a recorded result, not an error. Returns 1 if READY. */
static int switch_to(runner *r, int n, int tag)
{
    uint8_t c[12];
    gd_result res;
    char ex[96];
    uint64_t t0, now;
    int ready = 0, polls = 0, changes = 0;
    int last_rc = -1, last_st = -1, last_er = -1, last_sk = -1, last_asc = -1, last_ascq = -1;
    uint32_t timeout_ms_total;
    char msg[48];

    r->img = tag;
    snprintf(msg, sizeof(msg), "switch to image %d...", n);
    say(r, msg);
    cdb_mount(c, n);
    snprintf(ex, sizeof(ex), "n=%d", n);
    t0 = r->ops->now_us(r->ops->ctx);
    do_cmd(r, "SW_CMD", ex, c, 0, r->cfg->timeout_ms, 0, &res);

    timeout_ms_total = r->cfg->switch_timeout_ms;
    while (!r->abort) {
        int sk = -1, asc = -1, ascq = -1;
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
            if (!wl_check(c, r->cfg->do_switch)) {
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
                if (wl_check(sc, r->cfg->do_switch))
                    r->ops->packet(r->ops->ctx, sc, r->buf, 10, r->cfg->timeout_ms, &sr);
                if (sr.rc == GD_OK && sr.stored >= 10) {
                    /* REQ_ERROR (SPI spec): byte 2 low nibble = sense key, 8 = ASC, 9 = ASCQ */
                    sk = r->buf[2] & 0x0F;
                    asc = r->buf[8];
                    ascq = r->buf[9];
                }
            }
            if (tr.rc != last_rc || tr.status != last_st || tr.error != last_er || sk != last_sk ||
                asc != last_asc || ascq != last_ascq) {
                char f[120];
                changes++;
                char sn[24];
                if (sk < 0)
                    strcpy(sn, "none");
                else
                    snprintf(sn, sizeof(sn), "%d/%02x/%02x", sk, asc, ascq);
                snprintf(f, sizeof(f), "t_ms=%lu poll=%d rc=%s status=0x%02x err=0x%02x sense=%s",
                         (unsigned long)t_ms, polls, rec_rc_name(tr.rc), tr.status, tr.error, sn);
                out_line(r, rec_event_line(r->line, sizeof(r->line), r->img, "SW_POLL", f), 0);
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
    event(r, "SW_RESULT", "n=%d rc=%s t_ms=%lu polls=%d changes=%d", n, ready ? "ok" : (r->abort ? "abort" : "timeout"),
          (unsigned long)((now - t0) / 1000u), polls, changes);
    sync_out(r);
    return ready;
}

/* ---- entry --------------------------------------------------------------- */

int gp_run(const gd_ops *ops, const gp_config *cfg, const gp_sink *sink)
{
    runner *r = &g_r;
    int i;
    memset(r, 0, sizeof(*r));
    r->ops = ops;
    r->cfg = cfg;
    r->sink = sink;

    if (cfg->do_identify && ops->identify) {
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
            int ready = switch_to(r, n, n);
            snprintf(msg, sizeof(msg), "probing image %d%s...", n, ready ? "" : " (not ready)");
            say(r, msg);
            battery(r, 1);
        }
        if (cfg->return_to > 0 && !r->abort)
            switch_to(r, cfg->return_to, cfg->return_to);
    } else {
        r->img = 0;
        say(r, "probing the mounted image...");
        battery(r, 1);
    }
    event(r, "END", "abort=%d", r->abort);
    sync_out(r);
    return r->abort;
}
