/* Host tests for gdprobe (plain gcc, no KOS): hash, record, whitelist, cfg,
 * pcout chunking, and the whole probe runner against a scripted fake drive. */
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "../cfg.h"
#include "../gdtypes.h"
#include "../hash.h"
#include "../pcout.h"
#include "../probes.h"
#include "../record.h"
#include "../whitelist.h"

static int g_checks;
#define CHECK(c) do { g_checks++; if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); exit(1); } } while (0)

/* ---------------------------------------------------------------- hash */
static void test_hash(void)
{
    CHECK(crc32_calc("123456789", 9) == 0xCBF43926u);
    CHECK(crc32_calc("", 0) == 0);
    {
        uint32_t c = crc32_update(0, "1234", 4);
        c = crc32_update(c, "56789", 5);
        CHECK(c == 0xCBF43926u);
    }
}

/* -------------------------------------------------------------- record */
static void test_record(void)
{
    char line[REC_LINE_MAX];
    uint8_t cdb[12] = { 0x15, 0, 2, 0, 6 };
    uint8_t d[6] = { 1, 0, 3, 0xAF, 0x1E, 0x00 };
    gd_result r = { GD_OK, 0x50, 0, 3, 6, 6, 1234, 0, 0, 0, 0 };
    rec_probe p = { 3, "REQ_SES_2", NULL, cdb, &r, d };
    size_t n = rec_probe_line(line, sizeof(line), &p);
    CHECK(!strcmp(line, "img=3 probe=REQ_SES_2 cdb=150002000600000000000000 rc=ok status=0x50 err=0x00 irr=0x03 bytes=6 data=010003af1e00 us=1234\n"));
    CHECK(n == strlen(line));
    p.extra = "fad=45150 count=7";
    r.rc = GD_TIMEOUT;
    rec_probe_line(line, sizeof(line), &p);
    CHECK(strstr(line, "probe=REQ_SES_2 fad=45150 count=7 cdb=") && strstr(line, "rc=timeout"));
    rec_hash_line(line, sizeof(line), 3, 45150, 0xDEADBEEF, 2048);
    CHECK(!strcmp(line, "img=3 probe=HASH fad=45150 ssz=2048 crc=deadbeef\n"));
    rec_comment_line(line, sizeof(line), "gdprobe version=1");
    CHECK(!strcmp(line, "# gdprobe version=1\n"));
    /* truncation is safe and always ends in newline + NUL */
    {
        char small[20];
        size_t m = rec_probe_line(small, sizeof(small), &p);
        CHECK(m < sizeof(small) && small[m - 1] == '\n' && small[m] == 0);
    }
    /* data= is capped, bytes= keeps the true size */
    {
        static uint8_t big[2000];
        gd_result rb = { GD_OK, 0x50, 0, 3, 2000, 2000, 1, 0, 0, 0, 0 };
        rec_probe pb = { 0, "BIG", NULL, NULL, &rb, big };
        char *l = malloc(REC_LINE_MAX);
        rec_probe_line(l, REC_LINE_MAX, &pb);
        CHECK(strstr(l, "bytes=2000"));
        CHECK(strlen(strstr(l, "data=") + 5) == 2 * REC_DATA_MAX + strlen(" us=1\n"));
        free(l);
    }
}

/* ----------------------------------------------------------- whitelist */
static void test_whitelist(void)
{
    uint8_t c[12];
    int op, i, k;
    /* every table row has a reason */
    for (i = 0; i < wl_count; i++)
        CHECK(wl_table[i].why && wl_table[i].why[0]);

    memset(c, 0, 12);
    CHECK(wl_check(c, 0));                        /* TUR */
    c[0] = 0x03; c[4] = 18; CHECK(!wl_check(c, 0));   /* REQUEST SENSE is not a GD-ROM command */
    c[0] = 0x13; c[4] = 10; CHECK(wl_check(c, 0));    /* REQ_ERROR */
    c[4] = 11; CHECK(!wl_check(c, 0));
    memset(c, 0, 12); c[0] = 0x14; c[1] = 1; c[3] = 0x01; c[4] = 0x98; CHECK(wl_check(c, 0));   /* 408 */
    c[1] = 2; CHECK(!wl_check(c, 0));
    memset(c, 0, 12); c[0] = 0x15; c[2] = 3; c[4] = 6; CHECK(wl_check(c, 0));
    c[2] = 4; CHECK(!wl_check(c, 0));
    memset(c, 0, 12); c[0] = 0x40; c[1] = 1; c[4] = 14; CHECK(wl_check(c, 0));
    c[1] = 2; CHECK(!wl_check(c, 0));
    memset(c, 0, 12); c[0] = 0x11; c[2] = 18; c[4] = 8; CHECK(wl_check(c, 0));
    c[4] = 20; CHECK(!wl_check(c, 0));               /* beyond the 32-byte mode data */

    /* CD_READ */
    memset(c, 0, 12); c[0] = 0x30; c[1] = 0x24; c[2] = 0; c[3] = 0xB0; c[4] = 0x5E; c[10] = 16;
    CHECK(wl_check(c, 0));
    c[10] = 64; CHECK(wl_check(c, 0));
    c[10] = 65; CHECK(!wl_check(c, 0));
    c[10] = 0;  CHECK(!wl_check(c, 0));
    c[10] = 16; c[1] = 0x25; CHECK(!wl_check(c, 0));  /* MSF addressing */
    c[1] = 0x24; c[5] = 1; CHECK(!wl_check(c, 0));    /* reserved byte */
    c[5] = 0; c[11] = 1; CHECK(!wl_check(c, 0));

    /* CD_READ byte 1: exactly the CDREAD_FLAGS probe values (+ the configured one), FAD mode only */
    {
        static const uint8_t okf[4] = { 0x20, 0x24, 0x28, 0x10 };
        int f;
        memset(c, 0, 12); c[0] = 0x30; c[3] = 0xB0; c[4] = 0x5E; c[10] = 1;
        for (i = 0; i < 4; i++) {
            c[1] = okf[i]; c[10] = 1;  CHECK(wl_check(c, 0) && wl_check(c, 1));
            c[10] = 64; CHECK(wl_check(c, 0));
            c[10] = 65; CHECK(!wl_check(c, 0));
            c[10] = 0;  CHECK(!wl_check(c, 0));
            c[10] = 1; c[1] = (uint8_t)(okf[i] | 1); CHECK(!wl_check(c, 0));    /* MSF variant of each */
        }
        c[10] = 1;
        c[1] = 0x30; CHECK(!wl_check(c, 0));
        c[1] = 0x11; CHECK(!wl_check(c, 0));
        c[1] = 0xE6; CHECK(!wl_check(c, 0));
        c[1] = 0xF0; CHECK(!wl_check(c, 0));
        c[1] = 0x00; CHECK(!wl_check(c, 0));
        c[1] = 0x2C; CHECK(!wl_check(c, 0));
        /* the configured value is allowed in addition, still FAD only */
        c[1] = 0x30; CHECK(wl_check_cfg(c, 0, 0x30));
        c[1] = 0xE6; CHECK(!wl_check_cfg(c, 0, 0x30));
        c[1] = 0x20; CHECK(wl_check_cfg(c, 0, 0x30));
        c[1] = 0x31; CHECK(!wl_check_cfg(c, 0, 0x31));
        /* sweep all 256 values: accepted set is exactly the probe values (+ the cfg one) */
        for (f = 0; f < 256; f++) {
            int exp = (f == 0x20 || f == 0x24 || f == 0x28 || f == 0x10);
            c[1] = (uint8_t)f;
            CHECK((wl_check(c, 0) != NULL) == exp);
            CHECK((wl_check_cfg(c, 0, 0x44) != NULL) == (exp || f == 0x44));
            CHECK((wl_check_cfg(c, 1, 0x44) != NULL) == (exp || f == 0x44));
        }
    }

    /* 0x52: version always, mount only when switching is enabled */
    memset(c, 0, 12); c[0] = 0x52;
    CHECK(wl_check(c, 0));
    c[2] = 1; CHECK(!wl_check(c, 0) && !wl_check(c, 1));      /* 52 00 with junk */
    memset(c, 0, 12); c[0] = 0x52; c[1] = 0x82; c[2] = 3;
    CHECK(!wl_check(c, 0));
    CHECK(wl_check(c, 1));
    c[2] = 0; CHECK(!wl_check(c, 1));                          /* 0 = "default image": not used */
    {   /* 1000 must be refused */
        uint8_t m[12] = { 0x52, 0x82, 0xE8, 0x03 };
        CHECK(!wl_check(m, 1));
        m[2] = 0xE7;
        CHECK(wl_check(m, 1));
    }
    {   /* next / previous image and every other sub-code */
        uint8_t m[12] = { 0x52, 0x81, 0x55 };
        CHECK(!wl_check(m, 1));
        m[2] = 0x44; CHECK(!wl_check(m, 1));
        for (k = 1; k < 256; k++) {
            if (k == 0x82) continue;
            memset(m, 0, 12); m[0] = 0x52; m[1] = (uint8_t)k; m[2] = 5;
            CHECK(!wl_check(m, 1));
        }
    }

    /* the commands that must never be sent */
    {
        static const uint8_t bad[] = { 0x12, 0x16, 0x20, 0x21, 0x22, 0x70, 0x71, 0x2A, 0xA1, 0xA0, 0x04, 0x1B,
                                       0x55, 0x5A, 0xF9, 0xE0, 0xE1, 0x31, 0x41, 0x42, 0x43, 0xFF, 0x53, 0x51 };
        for (i = 0; i < (int)sizeof(bad); i++) {
            memset(c, 0, 12); c[0] = bad[i];
            CHECK(!wl_check(c, 0) && !wl_check(c, 1));
            memset(c, 0x11, 12); c[0] = bad[i];
            CHECK(!wl_check(c, 1));
        }
    }

    /* fuzz: whatever passes has an opcode from the table, and zero bytes outside its mask */
    srand(12345);
    for (i = 0; i < 300000; i++) {
        const wl_entry *e;
        for (k = 0; k < 12; k++)
            c[k] = (rand() % 3 == 0) ? (uint8_t)rand() : 0;
        if (rand() % 2) c[0] = (uint8_t)"\x00\x03\x10\x11\x13\x14\x15\x40\x30\x52"[rand() % 10];
        if (c[0] == 0x52 && rand() % 2) c[1] = 0x82;
        e = wl_check(c, 1);
        if (e) {
            CHECK(e->opcode == c[0]);
            for (k = 0; k < 12; k++)
                CHECK(c[k] == 0 || (e->mask & (1u << k)));
        }
    }
    for (op = 0; op < 256; op++) {
        int allowed_op = (op == 0 || op == 3 || op == 0x10 || op == 0x11 || op == 0x13 || op == 0x14 ||
                          op == 0x15 || op == 0x40 || op == 0x30 || op == 0x52);
        int any = 0;
        for (i = 0; i < 2000 && !any; i++) {
            for (k = 1; k < 12; k++) c[k] = (rand() % 2) ? (uint8_t)rand() : 0;
            c[0] = (uint8_t)op;
            if (op == 0x52) { c[1] = (rand() % 2) ? 0x82 : 0; }
            any = wl_check(c, 1) != NULL;
        }
        if (!allowed_op) CHECK(!any);
    }
}

/* ----------------------------------------------------------------- cfg */
static void test_cfg(void)
{
    gp_config c;
    gp_config_defaults(&c);
    CHECK(!c.do_switch && c.do_throughput && c.tp_batches == 64 && c.tp_sectors == 16 && !c.do_identify);
    gp_config_parse(&c, "# comment\nlabel = netgd\nswitch=on\nimages=3,4, 5-6 ,7\nreturn_to=2\nidentify=off\n"
                        "tp_sectors=99\nseek_n=1\nbogus=1\nlabel2\ncdread_flags=0x25 # trailing\n");
    CHECK(!strcmp(c.label, "netgd"));
    CHECK(c.do_switch);
    CHECK(c.n_images == 5 && c.images[0] == 3 && c.images[2] == 5 && c.images[3] == 6 && c.images[4] == 7);
    CHECK(c.return_to == 2);
    CHECK(c.tp_sectors == 16);          /* clamped */
    CHECK(c.seek_n == 2);               /* clamped */
    CHECK(c.cdread_flags == 0x24);      /* bit 0 (MSF) forced off */
    CHECK(c.unknown_keys == 2);
    gp_config_defaults(&c);
    CHECK(c.do_cdread_flags == 1);
    gp_config_parse(&c, "cdread_flags_test=off\n");
    CHECK(c.do_cdread_flags == 0 && c.unknown_keys == 0);
    gp_config_parse(&c, "cdread_flags_test=on\n");
    CHECK(c.do_cdread_flags == 1);
    gp_config_parse(&c, "cdread_flags_test=maybe\n");
    CHECK(c.do_cdread_flags == 1 && c.unknown_keys == 1);
    gp_config_defaults(&c);
    gp_config_parse(&c, "label=a/b c;d\nimages=0,1000,x\n");
    CHECK(!strcmp(c.label, "abcd"));
    CHECK(c.n_images == 0 && c.unknown_keys == 1);
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=maybe\n");
    CHECK(!c.do_switch && c.unknown_keys == 1);
}

/* --------------------------------------------------------------- pcout */
typedef struct { size_t max_write; int writes, flushes, closes, opened; size_t total; char *data; size_t cap; int flush_after_write_ok; int last_was_write; } mock_file;
static mock_file g_mf;
static void *m_open(const char *p, const char *m) { (void)p; (void)m; g_mf.opened = 1; return &g_mf; }
static size_t m_write(const void *p, size_t n, void *f)
{
    mock_file *mf = f;
    if (n > mf->max_write) mf->max_write = n;
    if (mf->last_was_write) mf->flush_after_write_ok = 0;   /* two writes without a flush */
    mf->last_was_write = 1;
    memcpy(mf->data + mf->total, p, n);
    mf->total += n; mf->writes++;
    return n;
}
static int m_flush(void *f) { mock_file *mf = f; mf->flushes++; mf->last_was_write = 0; return 0; }
static int m_close(void *f) { ((mock_file *)f)->closes++; return 0; }

static void *fail_open(const char *p, const char *m) { (void)p; (void)m; return NULL; }

static void test_pcout(void)
{
    static pcout o;
    static const pc_io io = { m_open, m_write, m_flush, m_close };
    size_t i, sent = 0;
    char line[300];
    memset(&g_mf, 0, sizeof(g_mf));
    g_mf.cap = 1 << 20; g_mf.data = malloc(g_mf.cap); g_mf.flush_after_write_ok = 1;
    CHECK(pcout_open(&o, &io, "/pc/x.txt") == 0 && g_mf.opened);
    for (i = 0; i < 700; i++) {
        size_t n = (size_t)snprintf(line, sizeof(line), "line %zu %0*d\n", i, (int)(i % 200), 7);
        CHECK(pcout_write(&o, line, n) == 0);
        sent += n;
        if (i % 97 == 0) CHECK(pcout_sync(&o) == 0);
    }
    /* one huge single write must still be chunked */
    {
        static char huge[100000];
        memset(huge, 'x', sizeof(huge));
        CHECK(pcout_write(&o, huge, sizeof(huge)) == 0);
        sent += sizeof(huge);
    }
    CHECK(pcout_close(&o) == 0);
    CHECK(g_mf.max_write <= 4096);
    CHECK(g_mf.flushes >= g_mf.writes);
    CHECK(g_mf.flush_after_write_ok);
    CHECK(g_mf.total == sent && g_mf.closes == 1);
    CHECK(sent > 100000 && g_mf.writes > 25);
    free(g_mf.data);
    /* open failure */
    {
        static const pc_io bad = { NULL, NULL, NULL, NULL };
        pc_io b2 = bad;
        b2.open = fail_open;
        CHECK(pcout_open(&o, &b2, "x") != 0);
        CHECK(pcout_write(&o, "a", 1) != 0);
    }
}

/* ------------------------------------------------------- fake drive */
typedef struct {
    uint64_t now;            /* virtual microseconds */
    int mounted;             /* folder number */
    int ua;                  /* unit attention pending */
    uint64_t ready_at;       /* not ready until this time */
    int empty_target;        /* mounted folder has no image: never ready */
    int stuck;               /* every command times out */
    int not_ready_forever;
    int empty_folder;        /* the folder number that is empty */
    int mounts[16], n_mounts;
    int cmds, refused_seen;
    int sent_bad;            /* something outside the whitelist reached the drive */
    int sw_enabled;
    int identify_calls;
    int ua_sense;
    int ses_fail;            /* REQ_SES(2) returns CHECK */
    uint32_t toc_fad;        /* FAD of track 3 in the HD-area TOC (0 = 45150) */
    int cd_typecheck;        /* CD_READ 0x28 (expected type = Mode 2 Form 1) on a Mode 1 image: CHECK 5/64/00 */
    int cd_no_raw;           /* CD_READ with bit 4 ("Other", 2352 bytes) is refused: CHECK 5/24/00 */
    int cd_no_flags;         /* every CD_READ flags value but 0x24 is refused (a drive that ignores the new ones) */
    uint8_t cdflags;         /* cfg->cdread_flags, for the whitelist check */
    int pend_sk, pend_asc, pend_ascq;   /* sense for the next REQ_ERROR */
    int cd_reads[256];       /* CD_READ commands seen per CDB byte 1 */
    int passive;             /* report cdbst / secnr like the hardware transport (R137 a, b) */
    int secnr_flip_at;       /* virtual us after which SECNR changes while the drive is not ready (0 = never) */
} fake;

/* The raw 2352-byte Mode 1 sector the fake returns for flags 0x10: sync, header, mode 1, user data, zeroed EDC/ECC */
static void raw_sector(uint32_t fad, uint8_t out[2352])
{
    uint32_t i, lba = fad - 150;
    memset(out, 0, 2352);
    memset(out + 1, 0xFF, 10);
    out[12] = (uint8_t)(lba / 4500); out[13] = (uint8_t)((lba / 75) % 60); out[14] = (uint8_t)(lba % 75); out[15] = 1;
    for (i = 0; i < 2048; i++) out[16 + i] = (uint8_t)(fad * 131u + i * 7u + 3u);
}

static uint8_t pat(uint32_t fad, uint32_t i) { return (uint8_t)(fad * 131u + i * 7u + 3u); }

static int f_packet0(void *ctx, const uint8_t cdb[12], uint8_t *buf, uint32_t cap, uint32_t timeout_ms, gd_result *r)
{
    fake *f = ctx;
    uint8_t d[512];
    uint32_t len = 0, i;
    memset(r, 0, sizeof(*r));
    f->cmds++;
    if (!wl_check_cfg(cdb, f->sw_enabled, f->cdflags)) f->sent_bad++;
    if (f->stuck) {
        f->now += (uint64_t)timeout_ms * 1000u;
        r->rc = GD_TIMEOUT; r->us = (uint64_t)timeout_ms * 1000u; r->status = 0x80;
        return r->rc;
    }
    f->now += 1500;
    r->us = 1500; r->irr = 3; r->status = 0x50;
    memset(d, 0, sizeof(d));
    if (cdb[0] == 0x52 && cdb[1] == 0x82) {
        int n = cdb[2] | (cdb[3] << 8);
        f->mounts[f->n_mounts++] = n;
        f->mounted = n;
        f->ua = 1;
        f->ready_at = f->now + 300000;          /* 300 ms to remount */
        f->not_ready_forever = (n == f->empty_folder);
        return r->rc;
    }
    if (cdb[0] == 0x13) {
        len = 10; d[0] = 0xF0;
        if (f->not_ready_forever || f->now < f->ready_at) { d[2] = 2; d[8] = f->not_ready_forever ? 0x3A : 0x04; d[9] = f->not_ready_forever ? 0 : 1; }
        else if (f->ua_sense) { d[2] = 6; d[8] = 0x29; f->ua_sense = 0; }
        else if (f->pend_sk) { d[2] = (uint8_t)f->pend_sk; d[8] = (uint8_t)f->pend_asc; d[9] = (uint8_t)f->pend_ascq; f->pend_sk = 0; }
        goto data;
    }
    if (f->not_ready_forever || f->now < f->ready_at) {
        r->rc = GD_CHECK; r->status = 0x51; r->error = 0x20;
        return r->rc;
    }
    if (f->ua && cdb[0] != 0x52) {
        r->rc = GD_CHECK; r->status = 0x51; r->error = 0x60; f->ua = 0; f->ua_sense = 1;
        return r->rc;
    }
    switch (cdb[0]) {
    case 0x00: break;
    case 0x10: len = 10; d[0] = 2; d[2] = 0x41; break;
    case 0x11: len = cdb[4]; if (cdb[2] == 18) memcpy(d, "Rev 6.42", 8); break;
    case 0x14:
        len = 408;
        {
            static uint8_t toc[408];
            memset(toc, 0xFF, sizeof(toc));
            if (cdb[1] == 0) { toc[0] = 0x41; toc[1] = 0; toc[2] = 0; toc[3] = 150; }
            toc[404] = 0x41; toc[405] = 0x08; toc[406] = 0x65; toc[407] = 0x1E; /* 549150 */
            if (cdb[1] == 1) { uint32_t tf = f->toc_fad ? f->toc_fad : 45150; toc[8] = 0x41; toc[9] = (uint8_t)(tf >> 16); toc[10] = (uint8_t)(tf >> 8); toc[11] = (uint8_t)tf; }
            for (i = 0; i < 408 && i < cap; i++) buf[i] = toc[i];
            r->total = 408; r->stored = cap < 408 ? cap : 408;
            return r->rc;
        }
    case 0x15:
        if (f->ses_fail && cdb[2] == 2) { r->rc = GD_CHECK; r->status = 0x51; r->error = 0x50; return r->rc; }
        len = 6; d[0] = 1; d[2] = (uint8_t)(cdb[2] + 1);
        { uint32_t fad = cdb[2] == 2 ? 45150 : 150; d[3] = (uint8_t)(fad >> 16); d[4] = (uint8_t)(fad >> 8); d[5] = (uint8_t)fad; }
        break;
    case 0x40: len = cdb[1] ? 14 : 100; d[1] = 0x15; break;
    case 0x52: len = 8; d[6] = 0x20; d[7] = 0x05; break;
    case 0x30: {
        uint32_t fad = ((uint32_t)cdb[2] << 16) | ((uint32_t)cdb[3] << 8) | cdb[4];
        uint32_t cnt = ((uint32_t)cdb[8] << 16) | ((uint32_t)cdb[9] << 8) | cdb[10], s, b;
        uint32_t ssz = (cdb[1] & 0x10) ? 2352 : 2048;
        f->cd_reads[cdb[1]]++;
        f->now += 20000 + (uint64_t)cnt * 1000;
        r->us = 21500 + (uint64_t)cnt * 1000;
        if ((cdb[1] == 0x28 && f->cd_typecheck) || (cdb[1] == 0x10 && f->cd_no_raw) ||
            (cdb[1] != 0x24 && f->cd_no_flags)) {
            r->rc = GD_CHECK; r->status = 0x51; r->error = 0x50;
            f->pend_sk = 5; f->pend_asc = (cdb[1] == 0x28) ? 0x64 : 0x24; f->pend_ascq = 0;
            return r->rc;
        }
        for (s = 0; s < cnt; s++) {
            uint8_t raw[2352];
            if (ssz == 2352) raw_sector(fad + s, raw);
            for (b = 0; b < ssz; b++) {
                uint32_t o = s * ssz + b;
                if (o < cap) buf[o] = (ssz == 2352) ? raw[b] : pat(fad + s, b);
            }
        }
        r->total = cnt * ssz; r->stored = r->total < cap ? r->total : cap;
        return r->rc;
    }
    default: f->refused_seen++; r->rc = GD_CHECK; r->status = 0x51; r->error = 0x50; return r->rc;
    }
data:
    for (i = 0; i < len && i < cap; i++) buf[i] = d[i];
    r->total = len; r->stored = len < cap ? len : cap;
    return r->rc;
}
/* The hardware transport adds two passive observations: ALTSTATUS in the CDB phase and SECNR at completion. */
static int f_packet(void *ctx, const uint8_t cdb[12], uint8_t *buf, uint32_t cap, uint32_t timeout_ms, gd_result *r)
{
    fake *f = ctx;
    int rc = f_packet0(ctx, cdb, buf, cap, timeout_ms, r);
    if (f->passive && rc != GD_REFUSED && !f->stuck) {
        int busy = f->not_ready_forever || f->now < f->ready_at;
        r->cdbst = 0x58; r->has_cdbst = 1;
        /* low nibble = drive status (1 = busy/seeking, 2 = standby), high nibble = disc format (8 = GD-ROM) */
        r->secnr = (uint8_t)(0x80 | (busy ? ((f->secnr_flip_at && f->now >= (uint64_t)f->secnr_flip_at) ? 0x04 : 0x01) : 0x02));
        r->has_secnr = 1;
    }
    return rc;
}
static int f_identify(void *ctx, uint8_t *buf, uint32_t t, gd_result *r)
{
    fake *f = ctx; (void)t;
    f->identify_calls++;
    memset(r, 0, sizeof(*r));
    memcpy(buf, "NetGD-fake", 10);
    r->total = r->stored = 512; r->status = 0x58;
    return 0;
}
static uint64_t f_now(void *ctx) { return ((fake *)ctx)->now; }
static void f_sleep(void *ctx, uint32_t ms) { ((fake *)ctx)->now += (uint64_t)ms * 1000u; }

/* sink capturing the record */
static char *g_rec;
static size_t g_rec_len;
static int g_syncs, g_hash_lines;
static void s_line(void *ctx, const char *s, size_t len, int is_hash)
{
    (void)ctx;
    CHECK(len == strlen(s) && s[len - 1] == '\n');
    memcpy(g_rec + g_rec_len, s, len);
    g_rec_len += len; g_rec[g_rec_len] = 0;
    if (is_hash) g_hash_lines++;
    CHECK((is_hash != 0) == (strstr(s, "probe=HASH") != NULL));
}
static void s_sync(void *ctx) { (void)ctx; g_syncs++; }

static int run_fake(fake *f, const gp_config *cfg)
{
    gd_ops ops = { f, f_packet, f_identify, f_now, f_sleep, NULL };
    gp_sink sink = { NULL, s_line, s_sync };
    g_rec_len = 0; g_rec[0] = 0; g_syncs = 0; g_hash_lines = 0;
    f->sw_enabled = cfg->do_switch;
    f->cdflags = cfg->cdread_flags;
    return gp_run(&ops, cfg, &sink);
}

static const char *find_line(const char *needle)
{
    return strstr(g_rec, needle);
}

static int count_of(const char *needle)
{
    int n = 0;
    const char *p = g_rec;
    while ((p = strstr(p, needle)) != NULL) { n++; p += strlen(needle); }
    return n;
}

static void test_runner_plain(void)
{
    static fake f;
    gp_config c;
    uint8_t sec[2048];
    char expect[64];
    uint32_t i;
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.empty_folder = 7;
    gp_config_defaults(&c);
    c.tp_batches = 4; c.seek_n = 8;
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.sent_bad == 0 && f.n_mounts == 0);           /* nothing mounted when switch=off */
    CHECK(!find_line("probe=SW_"));
    CHECK(find_line("img=0 probe=TUR_W0 cdb=000000000000000000000000 rc=check status=0x51 err=0x60"));
    CHECK(find_line("probe=REQ_ERROR_W0"));
    CHECK(find_line("probe=TUR_W1 ") && find_line("probe=WARMUP ready=1"));
    CHECK(find_line("probe=GET_TOC_1 cdb=14010001980000000000"));
    CHECK(find_line("probe=REQ_SES_2 cdb=150002000600"));
    CHECK(find_line("probe=GDEMU_VERSION cdb=520000000000"));
    CHECK(find_line("probe=REQ_MODE_18_8") && find_line("data=526576203"));   /* "Rev 6..." */
    CHECK(find_line("probe=IP_READ fad=45150 count=7 cdb=3024"));
    CHECK(find_line("probe=TP_003 fad=45198 count=16"));
    CHECK(!find_line("probe=TP_004"));
    CHECK(find_line("probe=TP_SUMMARY ok=4 bytes=131072"));
    CHECK(find_line("probe=SEEK_07 fad="));
    CHECK(!find_line("probe=IDENTIFY") && f.identify_calls == 0);
    /* IP.BIN sector 0 hash */
    for (i = 0; i < 2048; i++) sec[i] = pat(45150, i);
    snprintf(expect, sizeof(expect), "probe=HASH fad=45150 ssz=2048 crc=%08x", crc32_calc(sec, 2048));
    CHECK(find_line(expect));
    CHECK(g_hash_lines == 7 + 4 * 16 + 8);
    CHECK(g_syncs > 3);
    CHECK(find_line("probe=END abort=0"));
    /* seek positions alternate far / near and stay inside the span */
    CHECK(find_line("probe=SEEK_00 fad=45150") && find_line("probe=SEEK_01 fad=95150"));
}

static void test_runner_cdread_flags(void)
{
    static fake f;
    gp_config c;
    uint8_t raw[2352], sec[2048];
    char expect[200];
    uint32_t i;
    gp_config_defaults(&c);
    c.tp_batches = 2; c.seek_n = 4;

    /* a drive that accepts all four: 2048 / 2048 / 2048 / 2352 */
    memset(&f, 0, sizeof(f)); f.mounted = 2;
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.sent_bad == 0);
    CHECK(count_of("probe=CDREAD_FLAGS flags=") == 4);
    for (i = 0; i < 2048; i++) sec[i] = pat(45150, i);
    raw_sector(45150, raw);
    snprintf(expect, sizeof(expect), "probe=CDREAD_FLAGS flags=0x20 fad=45150 count=1 ssz=2048 crc=%08x sense=none cdb=302000b05e00000000000100 rc=ok", crc32_calc(sec, 2048));
    CHECK(find_line(expect));
    snprintf(expect, sizeof(expect), "probe=CDREAD_FLAGS flags=0x24 fad=45150 count=1 ssz=2048 crc=%08x sense=none", crc32_calc(sec, 2048));
    CHECK(find_line(expect));
    snprintf(expect, sizeof(expect), "probe=CDREAD_FLAGS flags=0x28 fad=45150 count=1 ssz=2048 crc=%08x sense=none", crc32_calc(sec, 2048));
    CHECK(find_line(expect));
    snprintf(expect, sizeof(expect), "probe=CDREAD_FLAGS flags=0x10 fad=45150 count=1 ssz=2352 crc=%08x sense=none cdb=301000b05e00000000000100", crc32_calc(raw, 2352));
    CHECK(find_line(expect));
    CHECK(f.cd_reads[0x20] == 1 && f.cd_reads[0x28] == 1 && f.cd_reads[0x10] == 1);
    CHECK(f.cd_reads[0x24] > 1);                       /* IP_READ, CDREAD_FLAGS 0x24, throughput, seek */
    CHECK(find_line("img=0 probe=CDREAD_FLAGS flags=0x20") < find_line("probe=TP_000"));   /* before throughput */
    CHECK(g_hash_lines == 7 + 2 * 16 + 4);              /* the battery writes no HASH lines */

    /* a type-checking drive: 0x28 on a Mode 1 image = CHECK with sense; the battery goes on to 0x10 */
    memset(&f, 0, sizeof(f)); f.mounted = 2; f.cd_typecheck = 1;
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.sent_bad == 0);
    CHECK(find_line("probe=CDREAD_FLAGS flags=0x28 fad=45150 count=1 ssz=- crc=- sense=5/64/00 cdb=302800b05e00000000000100 rc=check"));
    CHECK(find_line("probe=CDREAD_FLAGS flags=0x10 fad=45150 count=1 ssz=2352"));
    CHECK(find_line("probe=CDREAD_FLAGS flags=0x20 fad=45150 count=1 ssz=2048"));
    CHECK(find_line("probe=TP_SUMMARY ok=2") && find_line("probe=END abort=0"));

    /* no raw reads, and a drive that rejects every flags value but 0x24 */
    memset(&f, 0, sizeof(f)); f.mounted = 2; f.cd_no_raw = 1;
    CHECK(run_fake(&f, &c) == 0);
    CHECK(find_line("probe=CDREAD_FLAGS flags=0x10 fad=45150 count=1 ssz=- crc=- sense=5/24/00"));
    memset(&f, 0, sizeof(f)); f.mounted = 2; f.cd_no_flags = 1;
    CHECK(run_fake(&f, &c) == 0);
    CHECK(count_of("probe=CDREAD_FLAGS flags=") == 4 && count_of("rc=check") == 3);
    CHECK(find_line("flags=0x24 fad=45150 count=1 ssz=2048"));

    /* a configured cdread_flags outside the four is still sent (and the four still run) */
    memset(&f, 0, sizeof(f)); f.mounted = 2;
    gp_config_parse(&c, "cdread_flags=0x30\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.sent_bad == 0 && f.cd_reads[0x30] > 0 && count_of("probe=CDREAD_FLAGS flags=") == 4);
    CHECK(find_line("probe=IP_READ fad=45150 count=7 cdb=303000b05e"));
    gp_config_defaults(&c);
    c.tp_batches = 2; c.seek_n = 4;

    /* opt-out */
    memset(&f, 0, sizeof(f)); f.mounted = 2;
    gp_config_parse(&c, "cdread_flags_test=off\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(!find_line("probe=CDREAD_FLAGS") && f.cd_reads[0x20] == 0 && f.cd_reads[0x28] == 0 && f.cd_reads[0x10] == 0);
    CHECK(find_line("probe=IP_READ") && find_line("probe=TP_SUMMARY"));
    gp_config_defaults(&c);

    /* with image switching: every ready image gets its battery, the empty folder and img=0 do not */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.empty_folder = 7;
    gp_config_parse(&c, "switch=on\nimages=3,7,4\nreturn_to=2\ntp_batches=1\nseek_n=2\nswitch_timeout_ms=2000\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.sent_bad == 0);
    CHECK(find_line("img=3 probe=CDREAD_FLAGS flags=0x10") && find_line("img=4 probe=CDREAD_FLAGS flags=0x28"));
    CHECK(!find_line("img=7 probe=CDREAD_FLAGS") && !find_line("img=0 probe=CDREAD_FLAGS"));
    CHECK(count_of("probe=CDREAD_FLAGS flags=") == 8);
}

static void test_runner_switch(void)
{
    static fake f;
    gp_config c;
    const char *p;
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.empty_folder = 7;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3,7,4\nreturn_to=2\nidentify=on\ntp_batches=2\nseek_n=4\nswitch_timeout_ms=2000\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.sent_bad == 0);
    CHECK(f.n_mounts == 4 && f.mounts[0] == 3 && f.mounts[1] == 7 && f.mounts[2] == 4 && f.mounts[3] == 2);
    CHECK(f.identify_calls == 1 && find_line("img=0 probe=IDENTIFY_PACKET rc=ok"));
    CHECK(find_line("img=3 probe=SW_CMD n=3 cdb=528203000000"));
    CHECK(find_line("img=3 probe=SW_RESULT n=3 rc=ok t_ms="));
    CHECK(find_line("img=3 probe=SW_POLL") && strstr(g_rec, "sense=2/04/01") && strstr(g_rec, "sense=6/29/00") && strstr(g_rec, "sense=none"));
    CHECK(find_line("img=7 probe=SW_RESULT n=7 rc=timeout t_ms=2"));
    CHECK(strstr(g_rec, "sense=2/3a/00"));
    CHECK(find_line("img=2 probe=SW_RESULT n=2 rc=ok"));
    /* the empty folder: info commands recorded, no reads */
    p = find_line("img=7 probe=WARMUP ready=0");
    CHECK(p != NULL);
    CHECK(!find_line("img=7 probe=IP_READ") && !find_line("img=7 probe=TP_"));
    CHECK(find_line("img=3 probe=IP_READ") && find_line("img=4 probe=IP_READ"));
    CHECK(find_line("img=0 probe=REQ_STAT") && !find_line("img=0 probe=IP_READ"));
    CHECK(count_of("probe=SW_CMD") == 4);
    /* poll lines are only written on change: far fewer than polls */
    CHECK(count_of("probe=SW_POLL") < 40);
    CHECK(find_line("probe=END abort=0"));
    /* the fake drive's time-to-ready for folder 3: 300 ms (+ poll granularity) */
    {
        const char *r3 = find_line("img=3 probe=SW_RESULT n=3 rc=ok t_ms=");
        unsigned long ms = strtoul(r3 + strlen("img=3 probe=SW_RESULT n=3 rc=ok t_ms="), NULL, 10);
        CHECK(ms >= 300 && ms < 340);
    }
    if (getenv("GDPROBE_DUMP")) {   /* sample record for compare.py smoke tests */
        FILE *o = fopen(getenv("GDPROBE_DUMP"), "wb");
        if (o) { fwrite(g_rec, 1, g_rec_len, o); fclose(o); }
    }
}

/* R137 a, b: cdbst / secnr in probe lines and SW_POLL; a SECNR change alone is a state change */
static void test_runner_passive(void)
{
    static fake f;
    gp_config c;
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.empty_folder = 7; f.passive = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=0\ntp_batches=1\nseek_n=2\nswitch_timeout_ms=2000\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(find_line("img=3 probe=IP_READ") && strstr(find_line("img=3 probe=IP_READ"), " rc=ok status=0x50 err=0x00 irr=0x03 bytes=14336 cdbst=0x58 secnr=0x82 data="));
    CHECK(find_line("img=3 probe=SW_POLL") && strstr(g_rec, "sense=2/04/01 secnr=0x81"));
    CHECK(strstr(g_rec, "sense=none secnr=0x82"));
    CHECK(count_of("probe=SW_POLL") < 40);
    /* SECNR alone changing (everything else equal) writes a new SW_POLL line */
    {
        static fake g;
        memset(&g, 0, sizeof(g)); g.ua = 1; g.mounted = 2; g.empty_folder = 3; g.passive = 1;   /* folder 3 never ready */
        g.secnr_flip_at = 1000000;       /* 1 s: SECNR 0x81 -> 0x84, status/error/sense unchanged */
        gp_config_defaults(&c);
        gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=0\nswitch_timeout_ms=2000\n");
        run_fake(&g, &c);
        CHECK(count_of("img=3 probe=SW_POLL") == 2);
        CHECK(strstr(g_rec, "sense=2/3a/00 secnr=0x81") && strstr(g_rec, "sense=2/3a/00 secnr=0x84"));
    }
    /* without the passive fields (older transports) nothing is added */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.empty_folder = 7;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=0\nswitch_timeout_ms=2000\n");
    run_fake(&f, &c);
    CHECK(!strstr(g_rec, "cdbst=") && !strstr(g_rec, "secnr="));
}

static void test_runner_no_switch_when_off(void)
{
    static fake f;
    gp_config c;
    memset(&f, 0, sizeof(f)); f.mounted = 2;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=off\nimages=3,4\nreturn_to=2\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.n_mounts == 0 && f.sent_bad == 0);
    CHECK(!find_line("probe=SW_"));
}

static void test_runner_ip_fallback(void)
{
    static fake f;
    gp_config c;
    /* REQ_SES(2) unusable: IP_READ uses the first data track of the HD-area TOC */
    memset(&f, 0, sizeof(f)); f.mounted = 2; f.ses_fail = 1; f.toc_fad = 46000;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=off\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.sent_bad == 0);
    CHECK(find_line("probe=IP_READ fad=46000 count=7"));
    /* explicit ip_fad still wins */
    memset(&f, 0, sizeof(f)); f.mounted = 2; f.ses_fail = 1; f.toc_fad = 46000;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=off\nip_fad=12345\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(find_line("probe=IP_READ fad=12345 count=7"));
}

static void test_runner_stuck(void)
{
    static fake f;
    gp_config c;
    memset(&f, 0, sizeof(f)); f.stuck = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3,4,5\nreturn_to=2\n");
    CHECK(run_fake(&f, &c) == 1);
    CHECK(f.cmds <= 6);                                  /* gave up quickly */
    CHECK(find_line("probe=ABORT reason=drive-not-answering"));
    CHECK(find_line("probe=END abort=1"));
    CHECK(f.n_mounts == 0);
}

int main(void)
{
    g_rec = malloc(8u << 20);
    test_hash();
    test_record();
    test_whitelist();
    test_cfg();
    test_pcout();
    test_runner_plain();
    test_runner_cdread_flags();
    test_runner_switch();
    test_runner_passive();
    test_runner_no_switch_when_off();
    test_runner_ip_fallback();
    test_runner_stuck();
    printf("OK: %d checks passed\n", g_checks);
    return 0;
}
