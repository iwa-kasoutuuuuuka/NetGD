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
#include "../toc.h"
#include "../whitelist.h"

static int g_checks;
static char *g_rec;
static void fail_dump(void)
{   /* GDPROBE_FAILDUMP=file: the record of the failing runner test, for debugging */
    FILE *o = getenv("GDPROBE_FAILDUMP") && g_rec ? fopen(getenv("GDPROBE_FAILDUMP"), "wb") : NULL;
    if (o) { fputs(g_rec, o); fclose(o); }
}
#define CHECK(c) do { g_checks++; if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fail_dump(); exit(1); } } while (0)

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
    /* R148 */
    uint64_t mount_at;       /* virtual time of the last 52 82 */
    uint64_t quiet_us;       /* no packet may arrive within this long after a mount (0 = not checked) */
    int quiet_violations;
    int resets;              /* ATA DEVICE RESETs seen */
    uint64_t reset_notready_us; /* after a DEVICE RESET the drive answers NOT READY 2/04/01 for this long */
    uint64_t reset_t[8];     /* their time after the mount, us */
    int reset_flag;          /* registers show the post-reset signature until the next packet */
    int reset_timeout;       /* DEVICE RESET never clears BSY */
    int regs_fail;           /* the register read reports a bus-lock failure */
    int frozen_clock;        /* spin_us does not advance the clock */
    int no_r148_ops;         /* an older transport: no regs / ata_reset / spin_us */
    int mount_ua_asc;        /* ASC of the UNIT ATTENTION a 52 82 leaves (0 = 0x29) */
    int ua_asc;
    int audio_only_folder;   /* this folder holds an audio-only image (2 audio tracks, no data track) */
    /* seq 21 head model: REQ_STAT / GET_SCD(1) report `head`; a multi-sector read leaves it at its end, a 1-sector
     * read does not move it, a CHECK read leaves it at the start (one possible stock rule, to test the record) */
    int head_model;
    uint32_t head;
    int head_cmds[256];      /* commands seen per opcode */
    /* seq 22 edge images (stock head rule: a read leaves the head at start + count, a refused one at its start):
     * cdi_folder = CDI like images 4 / 10: track 1 audio at 150 (data 150..451), track 2 data at 11852, lead-out
     *              15854, area 1 a copy of area 0; gdi_folder = GDI like image 3: track 1 data 150 (readable
     *              150..755), track 2 audio 906, LD lead-out 1506, track 3 at 45150, HD lead-out 65150 */
    int cdi_folder, gdi_folder;
    /* seq 23: stock's readable range per track is [start - 150, data end) (LOG R174): pregaps read like the track */
    int pregap_model;
    /* seq 24: on this folder a multi-sector read that starts readable and runs past the readable end sends the
     * readable sectors and then never completes (stock at XEND, record 23): the drive stays stuck */
    int hang_folder;
    /* seq 25: DMA model. 0 = writes min(bytes, dma_len); 1 = drops the tail of an unaligned length (multiple of 32);
     * 2 = an unaligned length never completes; 3 = corrupts one byte; 4 = no DMA ever completes */
    int dma_model, dma_calls, dma_nonread, no_dma;
    uint32_t dma_max_len;
} fake;

static int f_edge(const fake *f) { return f->mounted && (f->mounted == f->cdi_folder || f->mounted == f->gdi_folder); }
/* sector size at fad on an edge image, 0 = not readable (5/21/00) */
static uint32_t f_edge_ssz(const fake *f, uint32_t fad)
{
    if (f->pregap_model) {
        if (f->mounted == f->cdi_folder)
            return fad <= 451 ? 2352 : (fad >= 11702 && fad < 15854) ? 2048 : 0;
        return fad <= 755 ? 2048 : (fad >= 756 && fad < 1506) ? 2352 : (fad >= 45000 && fad < 65150) ? 2048 : 0;
    }
    if (f->mounted == f->cdi_folder)
        return (fad >= 150 && fad <= 451) ? 2352 : (fad >= 11852 && fad < 15854) ? 2048 : 0;
    return (fad >= 150 && fad <= 755) ? 2048 : (fad >= 906 && fad < 1506) ? 2352 : (fad >= 45150 && fad < 65150) ? 2048 : 0;
}
static void put_toc(uint8_t *toc, int t, uint8_t ctl, uint32_t fad)
{
    toc[4 * t] = ctl; toc[4 * t + 1] = (uint8_t)(fad >> 16); toc[4 * t + 2] = (uint8_t)(fad >> 8); toc[4 * t + 3] = (uint8_t)fad;
}

static int f_trk(const fake *f, uint32_t fad)
{
    if (f->mounted && f->mounted == f->cdi_folder) return fad < 452 ? 1 : 2;
    if (f->mounted && f->mounted == f->gdi_folder) return fad < 756 ? 1 : fad < 1506 ? 2 : 3;
    if (f->mounted == f->audio_only_folder) return fad >= 752 ? 2 : 1;
    return fad >= (f->toc_fad ? f->toc_fad : 45150) ? 3 : 1;
}

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
    f->head_cmds[cdb[0]]++;
    if (!wl_check_cfg(cdb, f->sw_enabled, f->cdflags)) f->sent_bad++;
    if (f->quiet_us && f->n_mounts && !(cdb[0] == 0x52 && cdb[1] == 0x82) && f->now - f->mount_at < f->quiet_us)
        f->quiet_violations++;
    f->reset_flag = 0;
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
        f->mount_at = f->now;
        f->head = 150;
        f->ua = 1;
        f->ua_asc = f->mount_ua_asc ? f->mount_ua_asc : 0x29;
        f->ready_at = f->now + 300000;          /* 300 ms to remount */
        f->not_ready_forever = (n == f->empty_folder);
        return r->rc;
    }
    if (cdb[0] == 0x13) {
        len = 10; d[0] = 0xF0;
        if (f->not_ready_forever || f->now < f->ready_at) { d[2] = 2; d[8] = f->not_ready_forever ? 0x3A : 0x04; d[9] = f->not_ready_forever ? 0 : 1; }
        else if (f->ua_sense) { d[2] = 6; d[8] = (uint8_t)(f->ua_asc ? f->ua_asc : 0x29); f->ua_sense = 0; }
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
    case 0x10:
        len = 10; d[0] = 2; d[2] = 0x41;
        if (f->head_model) {
            d[0] = 1; d[1] = 0x8f; d[3] = (uint8_t)f_trk(f, f->head); d[4] = 1;
            d[5] = (uint8_t)(f->head >> 16); d[6] = (uint8_t)(f->head >> 8); d[7] = (uint8_t)f->head;
        }
        break;
    case 0x11: len = cdb[4]; if (cdb[2] == 18) memcpy(d, "Rev 6.42", 8); break;
    case 0x14:
        len = 408;
        {
            static uint8_t toc[408];
            memset(toc, 0xFF, sizeof(toc));
            if (f_edge(f)) {
                if (f->mounted == f->cdi_folder) {
                    put_toc(toc, 0, 0x01, 150); put_toc(toc, 1, 0x41, 11852); put_toc(toc, 101, 0x41, 15854);
                } else if (cdb[1] == 0) {
                    put_toc(toc, 0, 0x41, 150); put_toc(toc, 1, 0x01, 906); put_toc(toc, 101, 0x01, 1506);
                } else {
                    put_toc(toc, 2, 0x41, 45150); put_toc(toc, 101, 0x41, 65150);
                }
                for (i = 0; i < 408 && i < cap; i++) buf[i] = toc[i];
                r->total = 408; r->stored = cap < 408 ? cap : 408;
                return r->rc;
            }
            if (f->mounted == f->audio_only_folder) {
                /* single-session audio CD: SD area = track 1 at 150, track 2 at 752 (both ctrl 0x01); no HD area */
                if (cdb[1] == 0) {
                    toc[0] = 0x01; toc[1] = 0; toc[2] = 0; toc[3] = 150;
                    toc[4] = 0x01; toc[5] = 0; toc[6] = 0x02; toc[7] = 0xF0;
                    toc[404] = 0x01; toc[405] = 0; toc[406] = 0x06; toc[407] = 0x00;
                }
                for (i = 0; i < 408 && i < cap; i++) buf[i] = toc[i];
                r->total = 408; r->stored = cap < 408 ? cap : 408;
                return r->rc;
            }
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
        { uint32_t fad = cdb[2] == 2 ? (f->mounted && f->mounted == f->cdi_folder ? 11852 : 45150) : 150; d[3] = (uint8_t)(fad >> 16); d[4] = (uint8_t)(fad >> 8); d[5] = (uint8_t)fad; }
        break;
    case 0x40:
        len = cdb[1] ? 14 : 100; d[1] = 0x15;
        if (f->head_model && cdb[1] == 1) {
            d[3] = 14; d[4] = 0x41; d[5] = (uint8_t)f_trk(f, f->head); d[6] = 1;
            d[11] = (uint8_t)(f->head >> 16); d[12] = (uint8_t)(f->head >> 8); d[13] = (uint8_t)f->head;
        }
        break;
    case 0x52: len = 8; d[6] = 0x20; d[7] = 0x05; break;
    case 0x30: {
        uint32_t fad = ((uint32_t)cdb[2] << 16) | ((uint32_t)cdb[3] << 8) | cdb[4];
        uint32_t cnt = ((uint32_t)cdb[8] << 16) | ((uint32_t)cdb[9] << 8) | cdb[10], s, b;
        uint32_t ssz = (cdb[1] & 0x10) ? 2352 : 2048;
        f->cd_reads[cdb[1]]++;
        f->now += 20000 + (uint64_t)cnt * 1000;
        r->us = 21500 + (uint64_t)cnt * 1000;
        if (f_edge(f)) {
            uint32_t z = f_edge_ssz(f, fad), k;
            int bad = !z || !cnt;
            for (k = 1; k < cnt && !bad; k++) bad = f_edge_ssz(f, fad + k) != z;
            if (bad && z && f->hang_folder && f->mounted == f->hang_folder) {
                uint32_t good = 1;
                while (good < cnt && f_edge_ssz(f, fad + good) == z) good++;
                for (s = 0; s < good * z && s < cap; s++) buf[s] = (uint8_t)((fad + s / z) * 7u + s % z);
                f->stuck = 1;
                f->now += (uint64_t)timeout_ms * 1000u;
                r->rc = GD_TIMEOUT; r->us = (uint64_t)timeout_ms * 1000u; r->status = 0x80 | 0x08;
                r->total = good * z; r->stored = r->total < cap ? r->total : cap;
                return r->rc;
            }
            if (bad || (z == 2352 && (cdb[1] & 0x0E))) {
                r->rc = GD_CHECK; r->status = 0x51; r->error = 0x50;
                f->pend_sk = 5; f->pend_asc = bad ? 0x21 : 0x64; f->pend_ascq = 0;
                if (f->head_model) f->head = fad;
                return r->rc;
            }
            if (f->head_model) f->head = fad + cnt;
            for (s = 0; s < cnt * z && s < cap; s++) buf[s] = (uint8_t)((fad + s / z) * 7u + s % z);   /* depends only on the sector FAD */
            r->total = cnt * z; r->stored = r->total < cap ? r->total : cap;
            return r->rc;
        }
        if (f->mounted == f->audio_only_folder) {
            /* stock on audio (R144): 0x24 / 0x28 = 5/64/00, 0x20 / 0x10 = 2352-byte PCM */
            if (cdb[1] & 0x0E) {
                r->rc = GD_CHECK; r->status = 0x51; r->error = 0x50;
                f->pend_sk = 5; f->pend_asc = 0x64; f->pend_ascq = 0;
                if (f->head_model) f->head = fad;
                return r->rc;
            }
            if (f->head_model && cnt > 1) f->head = fad + cnt;
            for (s = 0; s < cnt * 2352 && s < cap; s++) buf[s] = (uint8_t)(fad + s);
            r->total = cnt * 2352; r->stored = r->total < cap ? r->total : cap;
            return r->rc;
        }
        if ((cdb[1] == 0x28 && f->cd_typecheck) || (cdb[1] == 0x10 && f->cd_no_raw) ||
            (cdb[1] != 0x24 && f->cd_no_flags)) {
            r->rc = GD_CHECK; r->status = 0x51; r->error = 0x50;
            f->pend_sk = 5; f->pend_asc = (cdb[1] == 0x28) ? 0x64 : 0x24; f->pend_ascq = 0;
            if (f->head_model) f->head = fad;
            return r->rc;
        }
        if (f->head_model && cnt > 1) f->head = fad + cnt;
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
/* R148 register model of the swap window (the stock sequence of R144 b): SECNR 0x06 (open) for 150 ms,
 * 0x00 (busy) until 300 ms, then 0x21 (pause, CD-ROM XA) or 0x07 (no disc) for an empty folder. */
static uint8_t f_secnr(fake *f)
{
    uint64_t t = f->now - f->mount_at;
    if (!f->n_mounts) return 0x22;
    if (t < 150000) return 0x06;
    if (t < 300000) return 0x00;
    return f->not_ready_forever ? 0x07 : (f->mounted == f->audio_only_folder ? 0x01 : 0x21);
}
static int f_regs(void *ctx, gd_regs *g)
{
    fake *f = ctx;
    memset(g, 0, sizeof(*g));
    if (f->regs_fail) return 1;
    g->altst = 0x50; g->irr = 0x03; g->secnr = f_secnr(f);
    if (f->reset_flag) { g->err = 0x01; g->irr = 0x01; g->bcl = 0x14; g->bch = 0xEB; }
    return 0;
}
static int f_reset(void *ctx, uint32_t timeout_ms, gd_regs *after, uint32_t *us, int *bsy)
{
    fake *f = ctx;
    if (f->resets < 8) f->reset_t[f->resets] = f->now - f->mount_at;
    f->resets++;
    if (f->reset_timeout) {
        f->now += (uint64_t)timeout_ms * 1000u;
        *us = timeout_ms * 1000u; *bsy = 1;
        memset(after, 0, sizeof(*after)); after->altst = 0x80;
        return GD_TIMEOUT;
    }
    f->now += 2000;
    *us = 2000; *bsy = 0;
    f->reset_flag = 1;
    f->ua = 1;                   /* reset: UNIT ATTENTION 6/29/00 for the next command */
    f->ua_asc = 0x29;
    if (f->reset_notready_us) f->ready_at = f->now + f->reset_notready_us;
    f_regs(ctx, after);
    return GD_OK;
}
static int f_dma(void *ctx, const uint8_t cdb[12], uint8_t *buf, uint32_t buflen, uint32_t dma_len, uint32_t timeout_ms,
                 gd_result *r, gd_dma *d)
{
    fake *f = ctx;
    static uint8_t tmp[40000];
    uint32_t w;
    memset(d, 0, sizeof(*d));
    d->req_len = dma_len; d->len_left = 0xFFFFFFFFu;
    CHECK(((uintptr_t)buf & 31u) == 0 && (buflen & 31u) == 0 && dma_len <= buflen);
    if (cdb[0] != 0x30) {
        f->dma_nonread++;
        memset(r, 0, sizeof(*r)); r->rc = GD_REFUSED;
        return r->rc;
    }
    f->dma_calls++;
    if (dma_len > f->dma_max_len) f->dma_max_len = dma_len;
    f_packet(ctx, cdb, tmp, sizeof(tmp), timeout_ms, r);
    if (r->rc != GD_OK)
        return r->rc;
    if (f->dma_model == 4 || (f->dma_model == 2 && (dma_len % 32))) {
        f->now += (uint64_t)timeout_ms * 1000u;
        r->rc = GD_TIMEOUT; r->us = (uint64_t)timeout_ms * 1000u; r->status = 0x58;
        d->aborted = 1; d->first = 'g'; d->dma_st_end = 1; d->len_left = dma_len;
        r->total = r->stored = 0;
        return r->rc;
    }
    if (f->dma_model == 5) {
        /* whole 32-byte blocks only; the drive sends exactly `total` bytes */
        uint32_t blocks = r->total & ~31u, nw = dma_len & ~31u;
        w = nw < blocks ? nw : blocks;
        memcpy(buf, tmp, w);
        d->addr_after = 0x0C100000u; d->len_left = dma_len; d->bch = 8; d->first = 'g'; d->asic_nrm = 0;
        if (dma_len >= r->total && (dma_len != r->total || (r->total % 32 == 0))) {
            if (dma_len == r->total) { d->done = 1; d->us_dma = 1000; d->asic_nrm = 1; memcpy(buf, tmp, w); r->total = r->stored = 0; return r->rc; }
        }
        if (dma_len < r->total) {            /* complete; the rest is still in the drive: DRQ, drained by PIO */
            uint32_t rest = r->total - w, k;
            d->done = 1; d->us_dma = 1000; d->asic_nrm = 1;
            d->drq = 1; d->drq_bc = (uint16_t)rest; d->drain = rest;
            for (k = 0; k < rest && k < sizeof(d->tail); k++) d->tail[k] = tmp[w + k];
            d->tail_n = (uint8_t)(rest < sizeof(d->tail) ? rest : sizeof(d->tail));
            r->total = r->stored = 0;
            return r->rc;
        }
        d->aborted = 1;                       /* waits for the last partial block forever */
        r->rc = GD_PROTO; r->us += 200000; f->now += 200000;
        r->total = r->stored = 0;
        return r->rc;
    }
    w = r->total < dma_len ? r->total : dma_len;
    if (f->dma_model == 1 && (dma_len % 32))
        w &= ~31u;
    memcpy(buf, tmp, w);
    if (f->dma_model == 3 && w)
        buf[w / 2] ^= 0x5A;
    d->done = 1; d->first = 'd'; d->us_dma = 1000 + w / 8; d->len_left = dma_len - w; d->prot_ok = 1; d->asic_nrm = 1;
    d->addr_after = 0x0C100000u + w; d->bcl = 0; d->bch = 8;
    r->total = r->stored = 0;
    return r->rc;
}
static void f_spin(void *ctx, uint32_t us) { fake *f = ctx; if (!f->frozen_clock) f->now += us; }
static uint64_t f_now(void *ctx) { return ((fake *)ctx)->now; }
static void f_sleep(void *ctx, uint32_t ms) { ((fake *)ctx)->now += (uint64_t)ms * 1000u; }

/* sink capturing the record */
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
    gd_ops ops = { f, f_packet, f_identify, f_now, f_sleep, NULL, f_regs, f_reset, f_spin, f_dma };
    if (f->no_dma) ops.packet_dma = NULL;
    if (f->no_r148_ops) { ops.regs = NULL; ops.ata_reset = NULL; ops.spin_us = NULL; }
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

/* ---------------------------------------------------- R148 additions */
static void test_cfg_r148(void)
{
    gp_config c;
    gp_config_defaults(&c);
    CHECK(!c.swap_regpoll && c.swap_regpoll_ms == 1000 && c.swap_regpoll_interval_ms == 2);
    CHECK(!c.swap_devreset && c.n_devreset == 2 && c.devreset_ms[0] == 50 && c.devreset_ms[1] == 200);
    gp_config_parse(&c, "swap_regpoll=on\nswap_regpoll_ms=50\nswap_regpoll_interval_ms=0\nswap_devreset=yes\n"
                        "swap_devreset_ms=10, 300 ,4000\n");
    CHECK(c.swap_regpoll && c.swap_regpoll_ms == 100 && c.swap_regpoll_interval_ms == 1);   /* clamped */
    CHECK(c.swap_devreset && c.n_devreset == 3 && c.devreset_ms[0] == 10 && c.devreset_ms[1] == 300 &&
          c.devreset_ms[2] == 4000);
    CHECK(c.unknown_keys == 0);
    gp_config_parse(&c, "swap_regpoll_ms=99999\nswap_regpoll_interval_ms=99\n");
    CHECK(c.swap_regpoll_ms == 10000 && c.swap_regpoll_interval_ms == 50);
    /* a bad list is rejected whole (the previous list stays) and counted */
    gp_config_parse(&c, "swap_devreset_ms=1,2,3,4,5\n");
    CHECK(c.n_devreset == 3 && c.unknown_keys == 1);
    gp_config_parse(&c, "swap_devreset_ms=6000\n");
    CHECK(c.n_devreset == 3 && c.unknown_keys == 2);
    gp_config_parse(&c, "swap_devreset_ms=50,x\n");
    CHECK(c.n_devreset == 3 && c.unknown_keys == 3);
    gp_config_parse(&c, "swap_devreset_ms=\n");
    CHECK(c.n_devreset == 3 && c.unknown_keys == 4);
    gp_config_parse(&c, "swap_devreset_ms=-1\n");
    CHECK(c.n_devreset == 3 && c.unknown_keys == 5);
    gp_config_parse(&c, "swap_devreset_ms=0\nswap_regpoll=maybe\nswap_devreset=perhaps\n");
    CHECK(c.n_devreset == 1 && c.devreset_ms[0] == 0 && c.unknown_keys == 7);
    CHECK(c.swap_regpoll && c.swap_devreset);    /* unparsable bools leave the value */
}

static void test_whitelist_ata(void)
{
    int i, cmd;
    for (i = 0; i < wl_ata_count; i++)
        CHECK(wl_ata_table[i].why && wl_ata_table[i].why[0]);
    for (cmd = 0; cmd < 256; cmd++) {
        CHECK(wl_ata_check((uint8_t)cmd, 0, 0) == NULL);
        CHECK((wl_ata_check((uint8_t)cmd, 1, 0) != NULL) == (cmd == 0xA1));
        CHECK((wl_ata_check((uint8_t)cmd, 0, 1) != NULL) == (cmd == 0x08));
        CHECK((wl_ata_check((uint8_t)cmd, 1, 1) != NULL) == (cmd == 0xA1 || cmd == 0x08));
    }
    /* DEVICE RESET is not a CDB: the packet whitelist still refuses opcode 0x08 */
    {
        uint8_t c[12] = { 0x08 };
        CHECK(!wl_check(c, 1));
    }
}

/* the value of `key=` in the first line that contains `needle`, or -1 */
static long field_of(const char *needle, const char *key)
{
    const char *p = find_line(needle), *e, *k;
    char pat[40];
    if (!p) return -1;
    e = strchr(p, '\n');
    snprintf(pat, sizeof(pat), " %s=", key);
    k = strstr(p, pat);
    if (!k || (e && k > e)) return -1;
    return strtol(k + strlen(pat), NULL, 0);
}

static void test_runner_regpoll(void)
{
    static fake f;
    gp_config c;
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.empty_folder = 7; f.passive = 1;
    f.quiet_us = 990000;      /* no packet within 990 ms of any mount except where noted below */
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3,7\nreturn_to=2\nswap_regpoll=on\ntp_batches=1\nseek_n=2\nswitch_timeout_ms=3000\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.sent_bad == 0 && f.resets == 0);
    /* the return_to switch has no register poll, so its TUR polls are the only packets in a quiet window */
    CHECK(f.quiet_violations > 0);
    CHECK(!find_line("img=2 probe=SW_REGPOLL"));
    /* image 3: 0x06 -> 0x00 -> 0x21 by time alone, one line per change */
    CHECK(count_of("img=3 probe=SW_REGPOLL ") == 3);
    CHECK(find_line("img=3 probe=SW_REGPOLL t_ms=1 sample=1 altst=0x50 err=0x00 irr=0x03 secnr=0x06"));
    CHECK(field_of("img=3 probe=SW_REGPOLL t_ms=1", "secnr") == 0x06);
    CHECK(find_line("secnr=0x00\n") && find_line("img=3 probe=SW_REGPOLL t_ms=301 ") && strstr(find_line("img=3 probe=SW_REGPOLL t_ms=301 "), "secnr=0x21"));
    CHECK(find_line("img=3 probe=SW_REGPOLL t_ms=151 "));
    CHECK(find_line("img=3 probe=SW_REGPOLL_END t_ms=1001 ") && strstr(find_line("img=3 probe=SW_REGPOLL_END"), "changes=3 stop=time"));
    {
        long n = field_of("img=3 probe=SW_REGPOLL_END", "samples");
        CHECK(n >= 490 && n <= 500);                 /* every 2 ms for ~1 s */
    }
    /* then the usual TUR / REQ_ERROR flow consumes the UNIT ATTENTION; its first poll is after the quiet second */
    CHECK(field_of("img=3 probe=SW_POLL", "t_ms") >= 1000);
    CHECK(find_line("img=3 probe=SW_POLL") > find_line("img=3 probe=SW_REGPOLL_END"));
    CHECK(strstr(find_line("img=3 probe=SW_POLL"), "sense=6/29/00"));
    CHECK(find_line("img=3 probe=SW_RESULT n=3 rc=ok"));
    CHECK(find_line("img=3 probe=IP_READ"));
    /* the empty folder ends in NO DISC */
    CHECK(count_of("img=7 probe=SW_REGPOLL ") == 3 && strstr(g_rec, "irr=0x03 secnr=0x07\n"));
    CHECK(find_line("img=7 probe=SW_RESULT n=7 rc=timeout t_ms=40"));   /* switch_timeout_ms counts after the poll */
    CHECK(find_line("probe=END abort=0"));
    /* without the image-loop poll nothing is quiet-checked: the default run has no violations of a 1 ms window */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.quiet_us = 990000;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=0\nswap_regpoll=on\nswap_regpoll_ms=1000\nthroughput=off\nseek=off\n");
    CHECK(run_fake(&f, &c) == 0 && f.quiet_violations == 0);
    /* swap_regpoll without switch=on does nothing */
    memset(&f, 0, sizeof(f)); f.mounted = 2;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=off\nswap_regpoll=on\nswap_devreset=on\n");
    CHECK(run_fake(&f, &c) == 0 && f.n_mounts == 0 && f.resets == 0 && !find_line("REGPOLL") && !find_line("DEVRESET"));
}

static void test_runner_devreset(void)
{
    static fake f;
    gp_config c;
    const char *p;
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.empty_folder = 7; f.passive = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3,7\nreturn_to=2\nswap_devreset=on\ntp_batches=1\nseek_n=2\nswitch_timeout_ms=2000\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.sent_bad == 0);
    /* 2 images x 2 delays; the plain switches have no poll (swap_regpoll=off) */
    CHECK(f.resets == 4 && f.n_mounts == 3 + 4);
    CHECK(f.mounts[0] == 3 && f.mounts[1] == 3 && f.mounts[2] == 3 && f.mounts[3] == 7 && f.mounts[6] == 2);
    CHECK(f.reset_t[0] >= 49000 && f.reset_t[0] < 53000 && f.reset_t[1] >= 199000 && f.reset_t[1] < 203000);
    CHECK(!find_line("img=3 probe=SW_REGPOLL"));
    CHECK(find_line("img=3 probe=R50_SW_CMD n=3 cdb=528203"));
    p = find_line("img=3 probe=R50_SW_DEVRESET delay_ms=50 t_ms=51 rc=ok reset_us=2000 bsy_seen=0 altst=0x50 err=0x01 irr=0x01 secnr=0x06 bcl=0x14 bch=0xeb");
    CHECK(p != NULL);
    CHECK(find_line("img=3 probe=R200_SW_DEVRESET delay_ms=200 t_ms=201 rc=ok reset_us=2000 bsy_seen=0 altst=0x50 err=0x01 irr=0x01 secnr=0x00 bcl=0x14 bch=0xeb"));
    /* the register poll before (open phase) and right after the reset (always written, then the change at 150 ms) */
    CHECK(find_line("img=3 probe=R50_SW_REGPOLL t_ms=1 sample=1 altst=0x50 err=0x00 irr=0x03 secnr=0x06"));
    CHECK(find_line("img=3 probe=R50_SW_REGPOLL t_ms=53 ") > p);
    CHECK(strstr(find_line("img=3 probe=R50_SW_REGPOLL t_ms=53 "), "err=0x01 irr=0x01 secnr=0x06"));
    CHECK(find_line("img=3 probe=R50_SW_REGPOLL t_ms=151 "));
    CHECK(find_line("img=3 probe=R50_SW_REGPOLL_END t_ms=1001 "));
    /* then the TUR flow: UNIT ATTENTION from the reset, READY; the state block afterwards */
    CHECK(strstr(find_line("img=3 probe=R50_SW_POLL"), "sense=6/29/00"));
    CHECK(find_line("img=3 probe=R50_SW_RESULT n=3 rc=ok"));
    CHECK(find_line("img=3 probe=R50_REQ_STAT cdb=10000000") && find_line("img=3 probe=R50_REQ_ERROR cdb=13000000"));
    CHECK(find_line("img=3 probe=R200_SW_RESULT n=3 rc=ok"));
    /* order: battery of image 3, then its reset switches, then image 7 */
    CHECK(find_line("img=3 probe=SEEK_01") < find_line("img=3 probe=R50_SW_CMD"));
    CHECK(find_line("img=3 probe=R200_SW_RESULT") < find_line("img=7 probe=SW_CMD"));
    /* the empty folder: reset in the window, NO DISC afterwards, the switch times out as a recorded result */
    CHECK(find_line("img=7 probe=R50_SW_DEVRESET delay_ms=50") && find_line("img=7 probe=R200_SW_RESULT n=7 rc=timeout"));
    CHECK(!find_line("img=2 probe=R"));
    CHECK(find_line("probe=END abort=0"));

    /* a delay past swap_regpoll_ms stretches the poll to delay + 300 ms */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=0\nswap_devreset=on\nswap_devreset_ms=1500\nthroughput=off\nseek=off\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.resets == 1 && f.reset_t[0] >= 1499000 && f.reset_t[0] < 1503000);
    CHECK(find_line("img=3 probe=R1500_SW_REGPOLL_END t_ms=1801 "));

    /* a reset that never clears BSY: recorded as rc=timeout, the run goes on (the TUR polls answer again) */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.reset_timeout = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=2\nswap_devreset=on\nthroughput=off\nseek=off\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.resets == 2 && count_of("_SW_DEVRESET") == 2);
    CHECK(find_line("img=3 probe=R50_SW_DEVRESET delay_ms=50 t_ms=51 rc=timeout reset_us=3000000 bsy_seen=1"));
    CHECK(find_line("probe=END abort=0"));

    /* a drive that does not answer the switch: no reset is ever sent after a failed 52 82 */
    memset(&f, 0, sizeof(f)); f.stuck = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3,4\nreturn_to=2\nswap_devreset=on\nswap_regpoll=on\n");
    CHECK(run_fake(&f, &c) == 1);
    CHECK(f.resets == 0 && !find_line("REGPOLL") && find_line("probe=END abort=1"));

    /* an older transport without register access: the poll and the reset are skipped, recorded once */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.no_r148_ops = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=0\nswap_devreset=on\nswap_devreset_ms=50\nswap_regpoll=on\nthroughput=off\nseek=off\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.resets == 0 && !find_line("DEVRESET"));
    CHECK(find_line("img=3 probe=SW_REGPOLL_END t_ms=0 samples=0 changes=0 stop=no-register-access"));
    CHECK(find_line("img=3 probe=R50_SW_REGPOLL_END t_ms=0 samples=0 changes=0 stop=no-register-access"));
}

static void test_runner_regpoll_safety(void)
{
    static fake f;
    gp_config c;
    /* the clock does not move while spinning: the iteration cap ends the poll */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.frozen_clock = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=0\nswap_regpoll=on\nthroughput=off\nseek=off\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(find_line("img=3 probe=SW_REGPOLL_END") && strstr(find_line("img=3 probe=SW_REGPOLL_END"), "samples=3000 changes=1 stop=iteration-cap"));
    CHECK(find_line("img=3 probe=SW_RESULT n=3 rc=ok") && find_line("probe=END abort=0"));
    /* the bus lock fails: the poll stops at once */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.regs_fail = 1;
    CHECK(run_fake(&f, &c) == 0);
    CHECK(find_line("img=3 probe=SW_REGPOLL_END t_ms=1 samples=0 changes=0 stop=bus-lock"));
    CHECK(find_line("img=3 probe=SW_RESULT n=3 rc=ok"));
}

/* seq 17: swap_reswitch (RS_) and devreset_only (DR_) */
static void test_runner_seq17(void)
{
    static fake f;
    gp_config c;
    const char *p;
    gp_config_defaults(&c);
    CHECK(!c.swap_reswitch && c.swap_reswitch_wait_ms == 1000 && !c.devreset_only && c.devreset_only_wait_ms == 1000);
    gp_config_parse(&c, "swap_reswitch=on\nswap_reswitch_wait_ms=5\ndevreset_only=yes\ndevreset_only_wait_ms=99999\n");
    CHECK(c.swap_reswitch && c.swap_reswitch_wait_ms == 100 && c.devreset_only && c.devreset_only_wait_ms == 10000 && c.unknown_keys == 0);
    gp_config_parse(&c, "swap_reswitch=maybe\n");
    CHECK(c.swap_reswitch && c.unknown_keys == 1);

    /* swap_reswitch only: same folder again, no reset, quiet window, UA 6/28/00 recorded */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.mount_ua_asc = 0x28;
    f.quiet_us = 990000;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=0\nswap_reswitch=on\nthroughput=off\nseek=off\n");
    CHECK(run_fake(&f, &c) == 0 && f.sent_bad == 0);
    CHECK(f.resets == 0 && f.n_mounts == 2 && f.mounts[0] == 3 && f.mounts[1] == 3);
    CHECK(find_line("img=3 probe=RS_SW_CMD n=3 cdb=528203"));
    CHECK(find_line("img=3 probe=RS_SW_REGPOLL_END t_ms=1001 "));
    CHECK(strstr(find_line("img=3 probe=RS_SW_POLL"), "sense=6/28/00"));
    CHECK(find_line("img=3 probe=RS_SW_RESULT n=3 rc=ok") && strstr(find_line("img=3 probe=RS_SW_RESULT"), "ua_28=1"));
    CHECK(find_line("img=3 probe=RS_REQ_STAT") && find_line("img=3 probe=RS_REQ_ERROR"));
    CHECK(!find_line("DEVRESET") && !find_line("probe=DR_"));
    CHECK(find_line("probe=END abort=0"));

    /* devreset_only only: no 52 82, 500 ms idle first, the reset, UA 6/29/00 -> ua_28=0 */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3,7\nreturn_to=2\ndevreset_only=on\ndevreset_only_wait_ms=500\nthroughput=off\nseek=off\nswitch_timeout_ms=2000\n");
    CHECK(run_fake(&f, &c) == 0 && f.sent_bad == 0);
    CHECK(f.resets == 2 && f.n_mounts == 3 && f.mounts[0] == 3 && f.mounts[1] == 7 && f.mounts[2] == 2);
    p = find_line("img=3 probe=DR_IDLE idle_ms=500");
    CHECK(p != NULL);
    CHECK(find_line("img=3 probe=DR_SW_DEVRESET delay_ms=0 t_ms=0 rc=ok reset_us=2000") > p);
    CHECK(!find_line("probe=DR_SW_CMD"));
    CHECK(find_line("img=3 probe=DR_SW_REGPOLL_END t_ms=50"));
    CHECK(strstr(find_line("img=3 probe=DR_SW_POLL"), "sense=6/29/00"));
    CHECK(strstr(find_line("img=3 probe=DR_SW_RESULT n=3 rc=ok"), "ua_28=0"));
    CHECK(find_line("img=3 probe=DR_REQ_STAT") && find_line("img=3 probe=DR_REQ_ERROR"));
    CHECK(find_line("img=3 probe=DR_SW_RESULT") < find_line("img=7 probe=SW_CMD"));
    CHECK(find_line("img=7 probe=DR_SW_DEVRESET"));
    CHECK(!find_line("probe=RS_"));
    CHECK(find_line("probe=END abort=0"));

    /* nothing without switch=on; both together: re-switch first, then the reset */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=off\nswap_reswitch=on\ndevreset_only=on\n");
    CHECK(run_fake(&f, &c) == 0 && f.n_mounts == 0 && f.resets == 0 && !find_line("probe=RS_") && !find_line("probe=DR_"));
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=0\nswap_reswitch=on\ndevreset_only=on\nthroughput=off\nseek=off\n");
    CHECK(run_fake(&f, &c) == 0 && f.resets == 1 && f.n_mounts == 2);
    CHECK(find_line("img=3 probe=RS_SW_RESULT") < find_line("img=3 probe=DR_IDLE"));

    /* a reset that hangs: recorded as a timeout, the run still ends cleanly */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.reset_timeout = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3,4,5\nreturn_to=0\ndevreset_only=on\nthroughput=off\nseek=off\n");
    run_fake(&f, &c);
    CHECK(f.resets >= 1 && f.resets <= 3 && find_line("probe=DR_SW_DEVRESET") && strstr(find_line("probe=DR_SW_DEVRESET"), "rc=timeout"));
    CHECK(find_line("probe=END"));
}

/* seq 20: devreset_cmd (DC<ms>_) */
static void test_runner_seq20(void)
{
    static fake f;
    gp_config c;
    const char *p, *q;
    gp_config_defaults(&c);
    CHECK(!c.devreset_cmd && c.n_devreset_cmd == 3 && c.devreset_cmd_ms[0] == 10 && c.devreset_cmd_ms[1] == 50 && c.devreset_cmd_ms[2] == 100);
    gp_config_parse(&c, "devreset_cmd=on\ndevreset_cmd_ms=0, 20\n");
    CHECK(c.devreset_cmd && c.n_devreset_cmd == 2 && c.devreset_cmd_ms[0] == 0 && c.devreset_cmd_ms[1] == 20 && c.unknown_keys == 0);
    gp_config_parse(&c, "devreset_cmd_ms=1,2,3,4,5\ndevreset_cmd_ms=x\ndevreset_cmd=maybe\n");
    CHECK(c.n_devreset_cmd == 2 && c.unknown_keys == 3);
    CHECK(c.n_devreset == 2 && c.devreset_ms[0] == 50 && c.devreset_ms[1] == 200);   /* the R148 list is untouched */

    /* NOT READY for 150 ms after the reset: early commands see 2/04/01, then UA 6/29/00, then READY */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.reset_notready_us = 150000;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=0\ndevreset_cmd=on\nthroughput=off\nseek=off\ncdread_flags_test=off\n");
    CHECK(run_fake(&f, &c) == 0 && f.sent_bad == 0);
    CHECK(f.resets == 3 && f.n_mounts == 1);
    p = find_line("img=3 probe=DC10_IDLE idle_ms=500");
    CHECK(p != NULL);
    q = find_line("img=3 probe=DC10_SW_DEVRESET delay_ms=0 t_ms=0 rc=ok reset_us=2000");
    CHECK(q > p);
    q = find_line("img=3 probe=DC10_CMD_TUR delay_ms=10 ");
    CHECK(q != NULL && strstr(q, "rc=check") && strstr(q, "sense=2/04/01") && strstr(q, "altst=0x50"));
    CHECK(strstr(q, "secnr=0x81"));
    CHECK(find_line("img=3 probe=DC10_CMD_REQ_STAT") > q);
    CHECK(find_line("img=3 probe=DC10_CMD_REQ_ERROR") > find_line("img=3 probe=DC10_CMD_REQ_STAT"));
    CHECK(find_line("img=3 probe=DC10_CMD_POLL") > find_line("img=3 probe=DC10_CMD_REQ_ERROR"));
    q = find_line("img=3 probe=DC10_CMD_RESULT delay_ms=10 rc=ok ready=1 ready_ms=");
    CHECK(q != NULL);
    if (q) {
        long ms = strtol(strstr(q, "ready_ms=") + 9, NULL, 10);
        CHECK(ms >= 150 && ms < 400);
    }
    CHECK(find_line("img=3 probe=DC50_CMD_RESULT") > q);
    CHECK(find_line("img=3 probe=DC100_CMD_RESULT") > find_line("img=3 probe=DC50_CMD_RESULT"));
    CHECK(find_line("img=3 probe=DC100_CMD_TUR delay_ms=100 ") != NULL);
    CHECK(count_of("probe=DC10_CMD_POLL") >= 1 && !find_line("probe=DR_") && !find_line("probe=RS_") && !find_line("probe=R50_"));
    CHECK(find_line("probe=END abort=0"));

    /* READY at once (delay 0 works): the first TUR reports the UNIT ATTENTION, the next poll is READY */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=0\ndevreset_cmd=on\ndevreset_cmd_ms=0\nthroughput=off\nseek=off\ncdread_flags_test=off\n");
    CHECK(run_fake(&f, &c) == 0 && f.resets == 1);
    q = find_line("img=3 probe=DC0_CMD_TUR delay_ms=0 ");
    CHECK(q != NULL && strstr(q, "sense=6/29/00"));
    CHECK(find_line("img=3 probe=DC0_CMD_RESULT") && strstr(find_line("img=3 probe=DC0_CMD_RESULT"), "ready=1"));

    /* an empty folder never gets READY: bounded by 2 s, recorded as a timeout, the run ends cleanly */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.empty_folder = 7;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=7\nreturn_to=0\ndevreset_cmd=on\ndevreset_cmd_ms=10\nswitch_timeout_ms=1000\nthroughput=off\nseek=off\ncdread_flags_test=off\n");
    CHECK(run_fake(&f, &c) == 0 && f.sent_bad == 0);
    CHECK(find_line("img=7 probe=DC10_CMD_RESULT delay_ms=10 rc=timeout ready=0 ready_ms=- ") != NULL);
    CHECK(find_line("img=7 probe=DC10_CMD_TUR") && strstr(find_line("img=7 probe=DC10_CMD_TUR"), "sense=2/3a/00"));
    CHECK(find_line("probe=END abort=0"));

    /* a reset that hangs: recorded as a timeout, the commands still go out, nothing loops */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.reset_timeout = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=0\ndevreset_cmd=on\nthroughput=off\nseek=off\ncdread_flags_test=off\n");
    run_fake(&f, &c);
    CHECK(find_line("probe=DC10_SW_DEVRESET") && strstr(find_line("probe=DC10_SW_DEVRESET"), "rc=timeout") && find_line("probe=END"));

    /* the drive stops answering: abort after 3 timeouts */
    memset(&f, 0, sizeof(f)); f.mounted = 2; f.stuck = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3,4\nreturn_to=0\ndevreset_cmd=on\nthroughput=off\nseek=off\ncdread_flags_test=off\n");
    CHECK(run_fake(&f, &c) == 1 && find_line("probe=END abort=1"));

    /* switch=off: nothing */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=off\ndevreset_cmd=on\n");
    CHECK(run_fake(&f, &c) == 0 && f.resets == 0 && !find_line("probe=DC"));
}

/* The shipped seq 20 run configuration, end to end against the fake drive */
static void test_seq20_example_cfg(void)
{
    static fake f;
    static char text[4096];
    gp_config c;
    size_t n;
    FILE *in = fopen("../gdprobe.cfg.seq20.example", "rb");
    CHECK(in != NULL);
    if (!in) return;
    n = fread(text, 1, sizeof(text) - 1, in);
    fclose(in);
    text[n] = 0;
    gp_config_defaults(&c);
    gp_config_parse(&c, text);
    CHECK(c.unknown_keys == 0 && !strcmp(c.label, "stock") && c.seq == 20);
    CHECK(c.do_switch && c.n_images == 3 && c.images[0] == 4 && c.images[1] == 3 && c.images[2] == 7 && c.return_to == 2);
    CHECK(c.switch_timeout_ms == 5000 && !c.swap_regpoll && !c.swap_devreset && !c.swap_reswitch && !c.devreset_only);
    CHECK(c.devreset_cmd && c.n_devreset_cmd == 3 && c.devreset_cmd_ms[0] == 10 && c.devreset_cmd_ms[2] == 100);
    CHECK(!c.do_cdread_flags && !c.do_throughput && !c.do_seek && !c.do_identify);
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.empty_folder = 7; f.passive = 1; f.reset_notready_us = 150000;
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.sent_bad == 0 && f.resets == 9 && f.n_mounts == 3 + 1);
    CHECK(count_of("probe=DC10_CMD_RESULT") == 3 && count_of("probe=DC50_CMD_RESULT") == 3 && count_of("probe=DC100_CMD_RESULT") == 3);
    CHECK(!find_line("probe=SW_REGPOLL") && !find_line("probe=DR_") && !find_line("probe=RS_"));
    CHECK(find_line("probe=END abort=0"));
}

/* The shipped seq 17 / 18 run configurations, end to end against the fake drive */
static void test_seq17_example_cfg(void)
{
    static fake f;
    static char text[4096];
    gp_config c;
    size_t n;
    int k;
    for (k = 17; k <= 18; k++) {
        FILE *in = fopen(k == 17 ? "../gdprobe.cfg.seq17.example" : "../gdprobe.cfg.seq18.example", "rb");
        CHECK(in != NULL);
        if (!in) return;
        n = fread(text, 1, sizeof(text) - 1, in);
        fclose(in);
        text[n] = 0;
        gp_config_defaults(&c);
        gp_config_parse(&c, text);
        CHECK(c.unknown_keys == 0 && !strcmp(c.label, "stock") && c.seq == k);
        CHECK(c.do_switch && c.n_images == 3 && c.images[0] == 4 && c.images[1] == 3 && c.images[2] == 7 && c.return_to == 2);
        CHECK(c.switch_timeout_ms == 5000 && c.swap_regpoll && !c.swap_devreset && c.swap_reswitch && c.devreset_only);
        CHECK(!c.do_cdread_flags && !c.do_throughput && !c.do_seek && !c.do_identify);
        memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.empty_folder = 7; f.passive = 1;
        CHECK(run_fake(&f, &c) == 0);
        CHECK(f.sent_bad == 0 && f.resets == 3 && f.n_mounts == 3 + 3 + 1);
        CHECK(count_of("probe=RS_SW_RESULT") == 3 && count_of("probe=DR_SW_DEVRESET") == 3 && count_of("probe=R50_") == 0);
        CHECK(find_line("probe=END abort=0"));
    }
}

static void test_runner_audio_only(void)
{
    static fake f;
    gp_config c;
    /* image 11 = audio-only (REQ_SES 2 still answers 45150 in the fake: the TOC decides) */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.audio_only_folder = 11; f.passive = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=4,11,3\nreturn_to=2\nswap_regpoll=on\ntp_batches=1\nseek_n=2\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.sent_bad == 0);
    CHECK(find_line("img=11 probe=NO_DATA_TRACK audio_fad=150 reads=skipped"));
    CHECK(!find_line("img=11 probe=IP_READ") && !find_line("img=11 probe=TP_") && !find_line("img=11 probe=SEEK_"));
    CHECK(!find_line("img=11 probe=HASH"));
    CHECK(count_of("img=11 probe=CDREAD_FLAGS") == 4);
    CHECK(find_line("img=11 probe=CDREAD_FLAGS flags=0x20 fad=150 count=1 ssz=2352"));
    CHECK(find_line("img=11 probe=CDREAD_FLAGS flags=0x24 fad=150 count=1 ssz=- crc=- sense=5/64/00"));
    CHECK(find_line("img=11 probe=CDREAD_FLAGS flags=0x28 fad=150 count=1 ssz=- crc=- sense=5/64/00"));
    CHECK(find_line("img=11 probe=CDREAD_FLAGS flags=0x10 fad=150 count=1 ssz=2352"));
    CHECK(find_line("img=11 probe=REQ_STAT") && find_line("img=11 probe=GET_TOC_0"));
    CHECK(strstr(g_rec, "img=11 probe=SW_REGPOLL t_ms=301 sample=151 altst=0x50 err=0x00 irr=0x03 secnr=0x01"));
    /* the images around it are unaffected */
    CHECK(find_line("img=4 probe=IP_READ") && find_line("img=3 probe=IP_READ") && !find_line("img=3 probe=NO_DATA_TRACK"));
    CHECK(find_line("probe=END abort=0"));
    /* an explicit ip_fad still forces the reads (the drive's answers are recorded, nothing hangs) */
    memset(&f, 0, sizeof(f)); f.mounted = 11; f.audio_only_folder = 11;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=off\nip_fad=150\ntp_batches=1\nseek_n=2\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(!find_line("NO_DATA_TRACK") && find_line("img=0 probe=IP_READ fad=150 count=7"));
    CHECK(strstr(find_line("img=0 probe=IP_READ"), "rc=check"));
    /* switch=off on the audio-only image, flag battery off: no reads at all */
    memset(&f, 0, sizeof(f)); f.mounted = 11; f.audio_only_folder = 11;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=off\ncdread_flags_test=off\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(find_line("img=0 probe=NO_DATA_TRACK audio_fad=150") && !find_line("CDREAD_FLAGS") && f.cd_reads[0x24] == 0);
    if (getenv("GDPROBE_DUMP_R148")) {
        FILE *o = fopen(getenv("GDPROBE_DUMP_R148"), "wb");
        if (o) { fwrite(g_rec, 1, g_rec_len, o); fclose(o); }
    }
}

/* The shipped R148 run configuration, end to end against the fake drive (run from tests/, as run_tests.sh does) */
static void test_r148_example_cfg(void)
{
    static fake f;
    static char text[4096];
    gp_config c;
    size_t n;
    FILE *in = fopen("../gdprobe.cfg.r148.example", "rb");
    CHECK(in != NULL);
    n = fread(text, 1, sizeof(text) - 1, in);
    fclose(in);
    text[n] = 0;
    gp_config_defaults(&c);
    gp_config_parse(&c, text);
    CHECK(c.unknown_keys == 0 && !strcmp(c.label, "stock") && c.seq == 15);
    CHECK(c.do_switch && c.n_images == 4 && c.images[0] == 4 && c.images[1] == 3 && c.images[2] == 11 && c.images[3] == 7);
    CHECK(c.return_to == 2 && c.swap_regpoll && c.swap_devreset && c.n_devreset == 2 && c.cdread_flags == 0x20);
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.empty_folder = 7; f.audio_only_folder = 11; f.passive = 1;
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.sent_bad == 0 && f.resets == 8 && f.n_mounts == 4 + 8 + 1);
    CHECK(count_of("probe=SW_REGPOLL_END") == 4 && count_of("probe=R50_SW_DEVRESET") == 4 && count_of("probe=R200_SW_DEVRESET") == 4);
    CHECK(find_line("img=11 probe=NO_DATA_TRACK") && find_line("img=4 probe=IP_READ") && find_line("img=3 probe=IP_READ"));
    CHECK(find_line("img=2 probe=SW_RESULT n=2 rc=ok") && find_line("probe=END abort=0"));
    CHECK(f.now < 120ull * 1000000u);        /* about a minute and a half of drive time in the fake */
}

/* seq 21: the head-position cases */
static const char *line_of(const char *needle)
{
    static char buf[REC_LINE_MAX];
    const char *p = find_line(needle), *e;
    if (!p) return "";
    e = strchr(p, '\n');
    memcpy(buf, p, (size_t)(e - p)); buf[e - p] = 0;
    return buf;
}

static void test_runner_seq21(void)
{
    static fake f;
    gp_config c;
    const char *l;
    char want[200];
    uint32_t d = 45150, e = 550174, len = e - d;   /* the fake TOC lead-out 0x08651e */
    gp_config_defaults(&c);
    CHECK(!c.head_probe && c.head_flags == 0x20 && c.n_head_counts == 4 && c.head_counts[3] == 16);
    CHECK(c.head_late_ms == 300 && c.head_idle_ms == 2000 && c.head_scd == 1);
    gp_config_parse(&c, "head_probe=on\nhead_flags=0x29\nhead_counts=1, 3\nhead_late_ms=99999\nhead_idle_ms=0\nhead_scd=off\n");
    CHECK(c.head_probe && c.head_flags == 0x28 && c.n_head_counts == 2 && c.head_counts[1] == 3 && c.head_late_ms == 5000);
    CHECK(c.head_idle_ms == 0 && !c.head_scd && c.unknown_keys == 0);
    gp_config_parse(&c, "head_counts=1,17\nhead_counts=1,2,3,4,5,6,7\nhead_counts=0\nhead_probe=maybe\n");
    CHECK(c.n_head_counts == 2 && c.unknown_keys == 4);

    /* GDI-like image 3 + audio-only 11, with the head model and a type-checking drive */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.head_model = 1; f.cd_typecheck = 1;
    f.audio_only_folder = 11;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3,11\nreturn_to=2\nhead_probe=on\nthroughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 0 && f.sent_bad == 0);
    CHECK(f.head_cmds[0x20] == 0 && f.head_cmds[0x21] == 0 && f.head_cmds[0x22] == 0 && f.head_cmds[0x12] == 0);
    CHECK(find_line("img=3 probe=HD_START data_fad=45150 data_end=550174 audio_fad=0 audio_end=0 data_cases=1 audio_cases=0"));
    l = line_of("img=3 probe=HD_BASE_POS ");
    CHECK(strstr(l, "stat_rc=ok st=pause trk=3 idx=1 pos=45157 moved=- scd_rc=ok scd_trk=3 scd_idx=1 scd_pos=45157"));
    CHECK(strstr(l, "late_rc=ok late_st=pause late_pos=45157 stat=018f41030100b0650000 scd=0015000e410301"));
    snprintf(want, sizeof(want), "img=3 probe=HD_R1 flags=0x20 fad=%lu count=1 ssz=2048 sense=none cdb=3020", (unsigned long)(d + len / 16));
    CHECK(find_line(want));
    CHECK(strstr(line_of("img=3 probe=HD_R1_POS "), "pos=45157 d_start=") && strstr(line_of("img=3 probe=HD_R1_POS "), " moved=0 "));
    snprintf(want, sizeof(want), "img=3 probe=HD_R7 flags=0x20 fad=%lu count=7 ssz=2048", (unsigned long)(d + len * 3 / 16));
    CHECK(find_line(want));
    CHECK(strstr(line_of("img=3 probe=HD_R7_POS "), "d_start=+7 d_end=+0 moved=1"));
    CHECK(strstr(line_of("img=3 probe=HD_R16_POS "), "d_start=+16 d_end=+0 moved=1"));
    snprintf(want, sizeof(want), "img=3 probe=HD_HIT flags=0x20 fad=%lu count=1", (unsigned long)(d + len * 4 / 16 + 8));
    CHECK(find_line(want) && strstr(line_of("img=3 probe=HD_HIT_POS "), "d_start=+8 d_end=+7 moved=0"));
    snprintf(want, sizeof(want), "img=3 probe=HD_END1 flags=0x20 fad=%lu count=1", (unsigned long)(e - 2));
    CHECK(find_line(want) && find_line("img=3 probe=HD_END7 flags=0x20 fad=550134 count=7"));
    CHECK(find_line("img=3 probe=HD_START7 flags=0x20 fad=45150 count=7") && strstr(line_of("img=3 probe=HD_START7_POS "), "pos=45157 d_start=+7"));
    l = line_of("img=3 probe=HD_CHK28 ");
    CHECK(strstr(l, "flags=0x28") && strstr(l, "count=7 ssz=- sense=5/64/00") && strstr(l, "rc=check"));
    CHECK(strstr(line_of("img=3 probe=HD_CHK28_POS "), "d_start=+0 d_end=-7 moved=1"));
    CHECK(strstr(line_of("img=3 probe=HD_CHK24 "), "ssz=2048 sense=none") && strstr(line_of("img=3 probe=HD_CHK24_POS "), "d_end=+0"));
    CHECK(strstr(line_of("img=3 probe=HD_TUR "), "cdb=000000000000000000000000 rc=ok") && strstr(line_of("img=3 probe=HD_TUR_POS "), "moved=0"));
    CHECK(strstr(line_of("img=3 probe=HD_TOC "), "cdb=140000019800") && strstr(line_of("img=3 probe=HD_TOC_POS "), "moved=0"));
    CHECK(strstr(line_of("img=3 probe=HD_SES "), "cdb=150001000600") && strstr(line_of("img=3 probe=HD_SES_POS "), "moved=0"));
    CHECK(!find_line("img=3 probe=HD_AUD"));
    CHECK(strstr(line_of("img=3 probe=HD_IDLE_POS "), "idle_ms=2000 stat_rc=ok st=pause"));
    CHECK(find_line("img=3 probe=HD_TUR ") > find_line("img=3 probe=HD_CHK28_POS") && find_line("img=3 probe=HD_IDLE_POS") > find_line("img=3 probe=HD_SES_POS"));
    /* audio-only image: no data cases, audio reads return 2352-byte sectors */
    CHECK(find_line("img=11 probe=HD_START data_fad=0 data_end=0 audio_fad=150 audio_end=752 data_cases=0 audio_cases=1"));
    CHECK(!find_line("img=11 probe=HD_R1") && !find_line("img=11 probe=HD_CHK"));
    CHECK(find_line("img=11 probe=HD_AUD1 flags=0x20 fad=300 count=1 ssz=2352 sense=none"));
    CHECK(find_line("img=11 probe=HD_AUD7 flags=0x20 fad=451 count=7 ssz=2352 sense=none"));
    CHECK(strstr(line_of("img=11 probe=HD_AUD7_POS "), "trk=1 idx=1 pos=458 d_start=+7 d_end=+0 moved=1"));
    CHECK(!find_line("img=2 probe=HD_") && !find_line("img=0 probe=HD_"));
    CHECK(count_of("probe=HD_START ") == 2 && find_line("probe=END abort=0"));

    /* a drive whose head never moves (no head model, NetGD-like answer): everything moved=0; no scd / late fields */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=off\nhead_probe=on\nhead_scd=off\nhead_late_ms=0\nhead_idle_ms=0\nthroughput=off\nseek=off\ncdread_flags_test=off\n");
    CHECK(run_fake(&f, &c) == 0 && f.sent_bad == 0);
    CHECK(find_line("img=0 probe=HD_START data_fad=45150"));
    CHECK(count_of("moved=1") == 0 && count_of("moved=0") >= 14 && !find_line("scd_rc=") && !find_line("late_rc=") && !find_line("HD_IDLE"));
    CHECK(strstr(line_of("img=0 probe=HD_BASE_POS "), "stat_rc=ok st=standby trk=0 idx=0 pos=0 moved=- stat=0200410000"));

    /* head_probe off: no HD_ lines */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2;
    gp_config_defaults(&c);
    gp_config_parse(&c, "throughput=off\nseek=off\n");
    CHECK(run_fake(&f, &c) == 0 && !find_line("probe=HD_"));

    /* a flags value outside the whitelist is refused and recorded; nothing is sent */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2;
    gp_config_defaults(&c);
    gp_config_parse(&c, "head_probe=on\nhead_flags=0x30\nhead_idle_ms=0\nthroughput=off\nseek=off\ncdread_flags_test=off\n");
    CHECK(run_fake(&f, &c) == 0 && f.sent_bad == 0 && f.cd_reads[0x30] == 0);
    CHECK(strstr(line_of("img=0 probe=HD_R1 "), "rc=refused"));

    /* the drive stops answering: abort, no HD_ cases, the run ends */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.head_model = 1; f.stuck = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "head_probe=on\nthroughput=off\nseek=off\ncdread_flags_test=off\n");
    CHECK(run_fake(&f, &c) == 1 && find_line("probe=END abort=1") && !find_line("probe=HD_START"));
}

/* The shipped seq 21 run configuration, end to end against the fake drive */
static void test_seq21_example_cfg(void)
{
    static fake f;
    static char text[4096];
    gp_config c;
    size_t n;
    FILE *in = fopen("../gdprobe.cfg.seq21.example", "rb");
    CHECK(in != NULL);
    if (!in) return;
    n = fread(text, 1, sizeof(text) - 1, in);
    fclose(in);
    text[n] = 0;
    gp_config_defaults(&c);
    gp_config_parse(&c, text);
    CHECK(c.unknown_keys == 0 && !strcmp(c.label, "stock") && c.seq == 21);
    CHECK(c.do_switch && c.n_images == 4 && c.images[0] == 4 && c.images[1] == 3 && c.images[2] == 10 && c.images[3] == 11);
    CHECK(c.return_to == 2 && c.head_probe && c.head_flags == 0x20 && c.n_head_counts == 4 && c.head_scd);
    CHECK(!c.swap_regpoll && !c.swap_devreset && !c.swap_reswitch && !c.devreset_only && !c.devreset_cmd);
    CHECK(!c.do_cdread_flags && !c.do_throughput && !c.do_seek && !c.do_identify && c.cdread_flags == 0x20);
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.audio_only_folder = 11; f.passive = 1; f.head_model = 1;
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.sent_bad == 0 && f.resets == 0 && f.n_mounts == 4 + 1);
    CHECK(count_of("probe=HD_START ") == 4 && count_of("probe=HD_IDLE_POS ") == 4 && count_of("probe=HD_CHK28_POS ") == 3);
    CHECK(count_of("probe=HD_AUD7_POS ") == 1 && find_line("probe=END abort=0"));
    CHECK(f.now < 120ull * 1000000u);
}

/* seq 22: TOC parsing, the FAD plan and the gap search */
static void mk_toc(uint8_t *toc, const uint8_t *ctl, const uint32_t *fad, const int *tno, int n, uint8_t lo_ctl, uint32_t lo)
{
    int i;
    memset(toc, 0xFF, 408);
    for (i = 0; i < n; i++) put_toc(toc, tno[i] - 1, ctl[i], fad[i]);
    if (lo) put_toc(toc, 101, lo_ctl, lo);
}

static void test_toc_plan(void)
{
    uint8_t t0[408], t1[408];
    gp_toc_area a[2];
    gp_gap_plan p;
    uint32_t lo, hi, x;
    int n;

    /* CDI (images 4 / 10): audio track 1, data track 2, area 1 = copy */
    {
        static const uint8_t ctl[2] = { 0x01, 0x41 };
        static const uint32_t fad[2] = { 150, 11852 };
        static const int tno[2] = { 1, 2 };
        mk_toc(t0, ctl, fad, tno, 2, 0x41, 15854);
    }
    CHECK(gp_toc_parse(t0, &a[0]) && a[0].n == 2 && a[0].num[1] == 2 && a[0].ctl[0] == 0x01 && a[0].fad[1] == 11852 && a[0].leadout == 15854);
    CHECK(gp_toc_parse(t0, &a[1]));
    gp_gap_plan_make(a, &p);
    CHECK(p.trk_ok && p.a_num == 1 && p.a_fad == 150 && p.a_ctl == 0x01 && p.b_num == 2 && p.b_fad == 11852 && p.b_ctl == 0x41);
    CHECK(p.lo_ok && p.leadout == 15854 && p.last_fad == 11852 && p.last_num == 2);
    CHECK(!p.hd_ok && !strcmp(p.hd_why, "no_hd_area") && p.ref_fad == 11868);

    /* GDI (image 3): area 0 = tracks 1 / 2 + lead-out 1506, area 1 = track 3 at 45150 + lead-out 65150 */
    {
        static const uint8_t c0[2] = { 0x41, 0x01 }, c1[1] = { 0x41 };
        static const uint32_t f0[2] = { 150, 906 }, f1[1] = { 45150 };
        static const int n0[2] = { 1, 2 }, n1[1] = { 3 };
        mk_toc(t0, c0, f0, n0, 2, 0x01, 1506);
        mk_toc(t1, c1, f1, n1, 1, 0x41, 65150);
    }
    gp_toc_parse(t0, &a[0]);
    CHECK(gp_toc_parse(t1, &a[1]) && a[1].n == 1 && a[1].num[0] == 3);
    gp_gap_plan_make(a, &p);
    CHECK(p.trk_ok && p.a_fad == 150 && p.b_fad == 906 && p.b_num == 2);
    CHECK(p.lo_ok && p.leadout == 65150 && p.last_fad == 45150 && p.last_num == 3);
    CHECK(p.hd_ok && p.ld_leadout == 1506 && p.hd_fad == 45150 && p.hd_num == 3 && p.ref_fad == 166);

    /* audio-only CDI (image 11): two audio tracks, area 1 copy; reference = first track + 16 */
    {
        static const uint8_t ctl[2] = { 0x01, 0x01 };
        static const uint32_t fad[2] = { 150, 602 };
        static const int tno[2] = { 1, 2 };
        mk_toc(t0, ctl, fad, tno, 2, 0x01, 1004);
    }
    gp_toc_parse(t0, &a[0]); gp_toc_parse(t0, &a[1]);
    gp_gap_plan_make(a, &p);
    CHECK(p.trk_ok && p.b_fad == 602 && p.lo_ok && p.leadout == 1004 && p.last_fad == 602 && !p.hd_ok && p.ref_fad == 166);

    /* one track, no lead-out, area 1 unreadable */
    {
        static const uint8_t ctl[1] = { 0x41 };
        static const uint32_t fad[1] = { 150 };
        static const int tno[1] = { 1 };
        mk_toc(t0, ctl, fad, tno, 1, 0, 0);
    }
    gp_toc_parse(t0, &a[0]);
    memset(t1, 0xFF, sizeof(t1));
    CHECK(!gp_toc_parse(t1, &a[1]));
    gp_gap_plan_make(a, &p);
    CHECK(!p.trk_ok && !strcmp(p.trk_why, "one_track") && !p.lo_ok && !strcmp(p.lo_why, "no_leadout"));
    CHECK(!p.hd_ok && !strcmp(p.hd_why, "no_hd_area") && p.ref_fad == 150);
    /* nothing at all */
    memset(a, 0, sizeof(a));
    gp_gap_plan_make(a, &p);
    CHECK(!p.trk_ok && !strcmp(p.trk_why, "no_toc") && !p.lo_ok && !p.hd_ok && p.ref_fad == 0);

    /* the search: last readable 451 in 150..11851 */
    lo = 150; hi = 11851; n = 0;
    while ((x = gp_bsearch_next(lo, hi)) != 0) { if (x <= 451) lo = x; else hi = x; n++; }
    CHECK(lo == 451 && hi == 452 && n <= 14);
    CHECK(gp_bsearch_next(10, 11) == 0 && gp_bsearch_next(10, 12) == 11 && gp_bsearch_next(10, 10) == 0);
}

static void test_runner_seq22(void)
{
    static fake f;
    gp_config c;
    const char *l;

    gp_config_defaults(&c);
    CHECK(!c.gap_probe && c.gap_flags == 0x20 && c.gap_late_ms == 300 && c.gap_scd && c.gap_track_end == 0 && c.gap_count0);
    CHECK(c.n_gap_leadout == 3 && c.gap_leadout[2] == 100 && c.n_gap_low == 2 && c.gap_low[1] == 100);
    gp_config_parse(&c, "gap_probe=on\ngap_flags=0x25\ngap_late_ms=9000\ngap_scd=off\ngap_track_end=452\ngap_leadout=5\ngap_low=0,149\ngap_count0=off\n");
    CHECK(c.gap_probe && c.gap_flags == 0x24 && c.gap_late_ms == 5000 && !c.gap_scd && c.gap_track_end == 452 && !c.gap_count0);
    CHECK(c.n_gap_leadout == 1 && c.gap_leadout[0] == 5 && c.n_gap_low == 2 && c.gap_low[1] == 149 && c.unknown_keys == 0);
    gp_config_parse(&c, "gap_low=150\ngap_leadout=1,2,3,4,5\ngap_track_end=-1\ngap_probe=sometimes\n");
    CHECK(c.n_gap_low == 2 && c.n_gap_leadout == 1 && c.gap_track_end == 452 && c.unknown_keys == 4);

    /* CDI 4 + GDI 3 with the stock head rule */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.head_model = 1; f.cdi_folder = 4; f.gdi_folder = 3;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=4,3\nreturn_to=2\ngap_probe=on\nthroughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 0 && f.sent_bad == 0 && find_line("probe=END abort=0"));
    CHECK(f.head_cmds[0x20] == 0 && f.head_cmds[0x21] == 0 && f.head_cmds[0x22] == 0 && f.head_cmds[0x12] == 0);
    CHECK(!find_line("probe=HD_"));
    CHECK(find_line("img=4 probe=GP_START toc0=2/15854 toc1=2/15854 trk_a=1/150/0x01 trk_b=2/11852/0x41 leadout=15854 last=2/11852 "
                    "ld_leadout=0 hd=0/0 ref=11868 trk=ok lo=ok hdg=no_hd_area"));
    l = line_of("img=4 probe=GP_BASE_POS ");
    CHECK(strstr(l, "stat_rc=ok st=pause trk=2 idx=1 pos=11859 ctl=0x41 moved=- scd_rc=ok scd_trk=2 scd_idx=1 scd_pos=11859 scd_ctl=0x41 scd_rel=0 late_rc=ok"));
    CHECK(find_line("img=4 probe=GP_SCAN_R n=1 flags=0x20 fad=150 count=1 ssz=2352 sense=none"));
    CHECK(find_line("img=4 probe=GP_SCAN_R n=2 flags=0x20 fad=11851 count=1 ssz=- sense=5/21/00"));
    CHECK(find_line("img=4 probe=GP_SCAN a_fad=150 b_fad=11852 last_ok=451 gap_lo=452 gap_hi=11851 gap_len=11400 gap_mid=6151 src=scan reads="));
    CHECK(field_of("img=4 probe=GP_SCAN ", "reads") >= 3 && field_of("img=4 probe=GP_SCAN ", "reads") <= 16);
    CHECK(find_line("img=4 probe=GP_TRK_T1 flags=0x20 fad=200 count=1 ssz=2352 sense=none"));
    CHECK(strstr(line_of("img=4 probe=GP_TRK_T1_POS "), "trk=1 idx=1 pos=201 ctl=0x41 d_start=+1 d_end=+0 moved=1"));
    CHECK(find_line("img=4 probe=GP_TRK_GAPMID flags=0x20 fad=6151 count=1 ssz=- sense=5/21/00"));
    CHECK(strstr(line_of("img=4 probe=GP_TRK_GAPMID_POS "), "trk=2 idx=1 pos=6151 ctl=0x41 d_start=+0 d_end=-1 moved=1"));
    CHECK(find_line("img=4 probe=GP_TRK_T1END flags=0x20 fad=451 count=1 ssz=2352"));
    CHECK(find_line("img=4 probe=GP_TRK_GAPLO flags=0x20 fad=452 count=1 ssz=- sense=5/21/00"));
    CHECK(find_line("img=4 probe=GP_TRK_T2 flags=0x20 fad=11868 count=1 ssz=2048"));
    CHECK(find_line("img=4 probe=GP_TRK_GAPHI flags=0x20 fad=11851 count=1 ssz=- sense=5/21/00"));
    CHECK(find_line("img=4 probe=GP_TRK_T2B flags=0x20 fad=11884") && find_line("img=4 probe=GP_TRK_GAPMID2 flags=0x20 fad=6151"));
    CHECK(find_line("img=4 probe=GP_LO_REF flags=0x20 fad=15852 count=1 ssz=2048"));
    CHECK(find_line("img=4 probe=GP_LO_0 flags=0x20 fad=15854 count=1 ssz=- sense=5/21/00"));
    CHECK(find_line("img=4 probe=GP_LO_1 flags=0x20 fad=15855") && find_line("img=4 probe=GP_LO_100 flags=0x20 fad=15954"));
    CHECK(strstr(line_of("img=4 probe=GP_LO_100_POS "), "pos=15954 ctl=0x41 d_start=+0 d_end=-1 moved=1"));
    CHECK(find_line("img=4 probe=GP_HDG_SKIP reason=no_hd_area") && !find_line("img=4 probe=GP_HDG_REF"));
    CHECK(find_line("img=4 probe=GP_LOW_REF flags=0x20 fad=11868 count=1 ssz=2048"));
    CHECK(find_line("img=4 probe=GP_LOW_0 flags=0x20 fad=0 count=1 ssz=- sense=5/21/00 cdb=302000000000000000000100"));
    CHECK(find_line("img=4 probe=GP_LOW_100 flags=0x20 fad=100 count=1"));
    CHECK(find_line("img=4 probe=GP_CNT0_SKIP fad=11868 cdb=3020002e5c00000000000000 reason=whitelist_count_1_64"));
    CHECK(!find_line("probe=GP_CNT0 ") && f.cd_reads[0x20] > 0);
    /* GDI */
    CHECK(find_line("img=3 probe=GP_START toc0=2/1506 toc1=1/65150 trk_a=1/150/0x41 trk_b=2/906/0x01 leadout=65150 last=3/45150 "
                    "ld_leadout=1506 hd=3/45150 ref=166 trk=ok lo=ok hdg=ok"));
    CHECK(find_line("img=3 probe=GP_SCAN a_fad=150 b_fad=906 last_ok=755 gap_lo=756 gap_hi=905 gap_len=150 gap_mid=830 src=scan"));
    CHECK(strstr(line_of("img=3 probe=GP_TRK_GAPMID_POS "), "trk=2 idx=1 pos=830"));
    CHECK(find_line("img=3 probe=GP_HDG ld_leadout=1506 hd_fad=45150 gap_lo=1506 gap_hi=45149 gap_mid=23328"));
    CHECK(find_line("img=3 probe=GP_HDG_REF flags=0x20 fad=1505 count=1 ssz=2352 sense=none"));
    CHECK(find_line("img=3 probe=GP_HDG_L0 flags=0x20 fad=1506 count=1 ssz=- sense=5/21/00"));
    CHECK(find_line("img=3 probe=GP_HDG_MID flags=0x20 fad=23328") && find_line("img=3 probe=GP_HDG_HI flags=0x20 fad=45149"));
    CHECK(find_line("img=3 probe=GP_HDG_HD flags=0x20 fad=45166 count=1 ssz=2048") && find_line("img=3 probe=GP_HDG_MID2 flags=0x20 fad=23328"));
    CHECK(strstr(line_of("img=3 probe=GP_HDG_MID_POS "), "trk=3 idx=1 pos=23328"));
    CHECK(find_line("img=3 probe=GP_LO_REF flags=0x20 fad=65148") && find_line("img=3 probe=GP_LO_0 flags=0x20 fad=65150"));
    CHECK(find_line("img=3 probe=GP_LOW_REF flags=0x20 fad=166 count=1 ssz=2048"));
    CHECK(count_of("probe=GP_START ") == 2 && !find_line("img=2 probe=GP_") && !find_line("img=0 probe=GP_"));

    /* gap_track_end from the cfg (no scan); no scd / late; one lead-out offset; audio-only image: no gap -> skip */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.head_model = 1; f.cdi_folder = 4; f.audio_only_folder = 11;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=4,11\ngap_probe=on\ngap_track_end=1000\ngap_scd=off\ngap_late_ms=0\ngap_leadout=7\n"
                        "gap_count0=off\nthroughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 0 && f.sent_bad == 0);
    CHECK(!find_line("img=4 probe=GP_SCAN_R") && find_line("img=4 probe=GP_SCAN a_fad=150 b_fad=11852 last_ok=999 gap_lo=1000 gap_hi=11851"));
    CHECK(find_line("src=cfg reads=0") && !find_line("scd_rc=") && !find_line("late_rc="));
    CHECK(find_line("img=4 probe=GP_LO_7 flags=0x20 fad=15861") && !find_line("img=4 probe=GP_LO_0 ") && !find_line("GP_CNT0"));
    /* image 11 with gap_track_end=1000: outside track 1..2 (602) */
    CHECK(find_line("img=11 probe=GP_TRK_SKIP a_fad=150 b_fad=752 src=cfg reads=0 reason=gap_track_end_outside"));
    CHECK(find_line("img=11 probe=GP_HDG_SKIP reason=no_hd_area"));

    /* audio-only with the search: the whole track is readable -> no_gap */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.head_model = 1; f.audio_only_folder = 11;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=11\ngap_probe=on\nthroughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 0 && f.sent_bad == 0);
    CHECK(find_line("img=11 probe=GP_TRK_SKIP a_fad=150 b_fad=752 src=scan reads=2 reason=no_gap"));
    CHECK(find_line("img=11 probe=GP_LO_REF flags=0x20 fad=1534"));

    /* gap_probe off: no GP_ lines; a drive that stops answering: no GP_ cases */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2;
    gp_config_defaults(&c);
    gp_config_parse(&c, "throughput=off\nseek=off\n");
    CHECK(run_fake(&f, &c) == 0 && !find_line("probe=GP_"));
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.stuck = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "gap_probe=on\nthroughput=off\nseek=off\ncdread_flags_test=off\n");
    CHECK(run_fake(&f, &c) == 1 && !find_line("probe=GP_START"));
    /* switch=off: on the mounted (fake default) image, after head_probe */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.head_model = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "head_probe=on\nhead_idle_ms=0\ngap_probe=on\nthroughput=off\nseek=off\ncdread_flags_test=off\n");
    CHECK(run_fake(&f, &c) == 0 && f.sent_bad == 0 && find_line("img=0 probe=GP_START") > find_line("img=0 probe=HD_SES_POS"));
}

/* The shipped seq 22 run configuration, end to end against the fake drive */
static void test_seq22_example_cfg(void)
{
    static fake f;
    static char text[4096];
    gp_config c;
    size_t n;
    FILE *in = fopen("../gdprobe.cfg.seq22.example", "rb");
    CHECK(in != NULL);
    if (!in) return;
    n = fread(text, 1, sizeof(text) - 1, in);
    fclose(in);
    text[n] = 0;
    CHECK(n < 2048);   /* main.c reads at most cfgbuf - 1 bytes */
    gp_config_defaults(&c);
    gp_config_parse(&c, text);
    CHECK(c.unknown_keys == 0 && !strcmp(c.label, "stock") && c.seq == 22);
    CHECK(c.do_switch && c.n_images == 3 && c.images[0] == 4 && c.images[1] == 3 && c.images[2] == 10 && c.return_to == 2);
    CHECK(c.gap_probe && c.gap_flags == 0x20 && c.gap_late_ms == 300 && c.gap_scd && !c.gap_track_end && c.gap_count0);
    CHECK(c.n_gap_leadout == 3 && c.n_gap_low == 2 && !c.head_probe);
    CHECK(!c.swap_regpoll && !c.swap_devreset && !c.swap_reswitch && !c.devreset_only && !c.devreset_cmd);
    CHECK(!c.do_cdread_flags && !c.do_throughput && !c.do_seek && !c.do_identify && c.cdread_flags == 0x20);
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.head_model = 1; f.cdi_folder = 4; f.gdi_folder = 3;
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.sent_bad == 0 && f.resets == 0 && f.n_mounts == 3 + 1 && find_line("probe=END abort=0"));
    CHECK(count_of("probe=GP_START ") == 3 && count_of("probe=GP_SCAN ") == 2 && count_of("probe=GP_HDG ") == 1);
    CHECK(count_of("probe=GP_CNT0_SKIP ") == 3 && count_of("probe=GP_LO_100_POS ") == 3);
    CHECK(f.now < 180ull * 1000000u);
}

/* seq 23: pregap reads, crossing reads, CRCs */
static uint32_t edge_sector_crc(uint32_t fad, uint32_t z)
{
    static uint8_t b[2352];
    uint32_t i;
    for (i = 0; i < z; i++) b[i] = (uint8_t)(fad * 7u + i);
    return crc32_calc(b, z);
}

static void test_runner_seq23(void)
{
    static fake f;
    gp_config c;
    gp_track t[198];
    gp_toc_area a[2];
    uint8_t t0[408];
    uint32_t fa[8];
    char want[300];
    const char *l;
    static const int offs[8] = { -156, -151, -150, -149, -100, -2, -1, 0 };

    /* pure helpers */
    CHECK(gp_offset_fads(906, offs, 8, fa) == 8 && fa[0] == 750 && fa[1] == 755 && fa[2] == 756 && fa[4] == 806 && fa[7] == 906);
    CHECK(gp_offset_fads(100, offs, 8, fa) == 4 && fa[0] == 0 && fa[1] == 98 && fa[3] == 100);
    {
        static const uint8_t ctl[2] = { 0x01, 0x41 };
        static const uint32_t fad[2] = { 150, 11852 };
        static const int tno[2] = { 1, 2 };
        mk_toc(t0, ctl, fad, tno, 2, 0x41, 15854);
    }
    gp_toc_parse(t0, &a[0]); gp_toc_parse(t0, &a[1]);
    CHECK(gp_toc_tracks(a, t) == 2 && t[1].num == 2 && t[1].fad == 11852 && t[1].ctl == 0x41);

    /* cfg */
    gp_config_defaults(&c);
    CHECK(!c.gap_crc && !c.pregap_probe && c.pregap_cross == 16 && c.n_pregap_low == 4 && c.pregap_low[3] == 149);
    CHECK(c.n_pregap_offs == 8 && c.pregap_offs[0] == -156 && c.pregap_offs[7] == 0);
    gp_config_parse(&c, "gap_crc=on\npregap_probe=on\npregap_cross=99\npregap_low=5\npregap_offs=-300,300\n");
    CHECK(c.gap_crc && c.pregap_probe && c.pregap_cross == 16 && c.n_pregap_low == 1 && c.n_pregap_offs == 2 && c.unknown_keys == 0);
    gp_config_parse(&c, "pregap_offs=-301\npregap_offs=1,2,3,4,5,6,7,8,9\npregap_low=150\npregap_cross=1\n");
    CHECK(c.n_pregap_offs == 2 && c.n_pregap_low == 1 && c.pregap_cross == 2 && c.unknown_keys == 3);

    /* CDI 4 + GDI 3 with stock's pregap rule; only the seq 23 cases, with CRCs */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.head_model = 1; f.cdi_folder = 4; f.gdi_folder = 3;
    f.pregap_model = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=4,3\nreturn_to=2\npregap_probe=on\ngap_crc=on\nthroughput=off\nseek=off\n"
                        "cdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 0 && f.sent_bad == 0 && find_line("probe=END abort=0"));
    CHECK(count_of("probe=GP_START ") == 2 && !find_line("GP_TRK_") && !find_line("GP_LO_") && !find_line("GP_CNT0"));
    CHECK(find_line("img=4 probe=GP_PG_START cross=16 tracks=1/150/0x01,2/11852/0x41 leadout=15854 ld_leadout=0"));
    snprintf(want, sizeof(want), "img=4 probe=GP_PGL_100 flags=0x20 fad=100 count=1 ssz=2352 sense=none crc=%08lx cdb=",
             (unsigned long)edge_sector_crc(100, 2352));
    CHECK(find_line(want));
    CHECK(find_line("img=4 probe=GP_PGL_0 ") && find_line("img=4 probe=GP_PGL_1 ") && find_line("img=4 probe=GP_PGL_149 "));
    CHECK(strstr(line_of("img=4 probe=GP_PGL_149_POS "), "pos=150 ctl=0x41 d_start=+1"));
    CHECK(find_line("img=4 probe=GP_PGB trk=2 start=11852 prev=1/150 fads=11696,11701,11702,11703,11752,11850,11851,11852"));
    CHECK(find_line("img=4 probe=GP_PG2_M156 flags=0x20 fad=11696 count=1 ssz=- sense=5/21/00 crc=- cdb="));
    CHECK(find_line("img=4 probe=GP_PG2_M151 flags=0x20 fad=11701 count=1 ssz=- sense=5/21/00"));
    CHECK(find_line("img=4 probe=GP_PG2_M150 flags=0x20 fad=11702 count=1 ssz=2048 sense=none crc="));
    CHECK(find_line("img=4 probe=GP_PG2_M100 flags=0x20 fad=11752") && find_line("img=4 probe=GP_PG2_P0 flags=0x20 fad=11852"));
    l = line_of("img=4 probe=GP_XPRE1 ");
    CHECK(strstr(l, "flags=0x20 fad=140 count=16 ssz=2352 sense=none crc=") && strstr(l, " bytes=37632"));
    snprintf(want, sizeof(want), " scrc=%08lx,%08lx,", (unsigned long)edge_sector_crc(140, 2352), (unsigned long)edge_sector_crc(141, 2352));
    CHECK(strstr(l, want) && !strstr(l, "stored="));
    CHECK(strstr(line_of("img=4 probe=GP_XPRE1_POS "), "pos=156 ctl=0x41 d_start=+16 d_end=+0"));
    CHECK(find_line("img=4 probe=GP_XPRE2 flags=0x20 fad=11842 count=16 ssz=2048 sense=none crc="));
    CHECK(find_line("img=4 probe=GP_XGAP2 flags=0x20 fad=11694 count=16 ssz=- sense=5/21/00 crc=- cdb="));
    CHECK(find_line("img=4 probe=GP_DSCAN_R n=2 flags=0x20 fad=11701 count=1 ssz=- sense=5/21/00"));
    CHECK(find_line("img=4 probe=GP_DSCAN a_fad=150 b_fad=11852 hi=11701 last_ok=451 gap_lo=452 src=scan reads="));
    CHECK(find_line("img=4 probe=GP_XEND flags=0x20 fad=444 count=16 ssz=- sense=5/21/00"));
    CHECK(!find_line("img=4 probe=GP_XLD") && find_line("img=4 probe=GP_XLO flags=0x20 fad=15846 count=16 ssz=- sense=5/21/00"));
    /* GDI */
    CHECK(find_line("img=3 probe=GP_PG_START cross=16 tracks=1/150/0x41,2/906/0x01,3/45150/0x41 leadout=65150 ld_leadout=1506"));
    CHECK(find_line("img=3 probe=GP_PGB trk=2 start=906 prev=1/150 fads=750,755,756,757,806,904,905,906"));
    CHECK(find_line("img=3 probe=GP_PGB trk=3 start=45150 prev=2/906 fads=44994,44999,45000,45001,45050,45148,45149,45150"));
    CHECK(find_line("img=3 probe=GP_PG2_M150 flags=0x20 fad=756 count=1 ssz=2352"));
    CHECK(find_line("img=3 probe=GP_PG3_M151 flags=0x20 fad=44999 count=1 ssz=- sense=5/21/00"));
    CHECK(find_line("img=3 probe=GP_PG3_M150 flags=0x20 fad=45000 count=1 ssz=2048 sense=none crc="));
    CHECK(find_line("img=3 probe=GP_XPRE3 flags=0x20 fad=45140 count=16 ssz=2048 sense=none crc="));
    CHECK(find_line("img=3 probe=GP_XPRE2 flags=0x20 fad=896 count=16 ssz=2352"));
    CHECK(find_line("img=3 probe=GP_XGAP3 flags=0x20 fad=44992 count=16 ssz=- sense=5/21/00"));
    CHECK(find_line("img=3 probe=GP_XPRE1 flags=0x20 fad=140 count=16 ssz=2048"));
    CHECK(find_line("img=3 probe=GP_XEND_SKIP a_fad=150 b_fad=906 hi=755 src=scan reads=2 reason=no_gap"));
    CHECK(find_line("img=3 probe=GP_XLD flags=0x20 fad=1498 count=16 ssz=- sense=5/21/00"));
    CHECK(find_line("img=3 probe=GP_XLO flags=0x20 fad=65142 count=16"));

    /* gap_track_end from the cfg; crc off: no crc= fields */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.head_model = 1; f.cdi_folder = 4; f.pregap_model = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=4\npregap_probe=on\ngap_track_end=452\npregap_cross=8\npregap_low=0\npregap_offs=-1\n"
                        "throughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 0 && f.sent_bad == 0 && !find_line("count=1 ssz=2352 sense=none crc=") && !find_line("sense=5/21/00 crc="));
    CHECK(!find_line("GP_DSCAN_R") && find_line("img=4 probe=GP_DSCAN a_fad=150 b_fad=11852 hi=11701 last_ok=451 gap_lo=452 src=cfg reads=0"));
    CHECK(find_line("img=4 probe=GP_XEND flags=0x20 fad=448 count=8") && find_line("img=4 probe=GP_PGB trk=2 start=11852 prev=1/150 fads=11851"));
    CHECK(count_of("probe=GP_PGL_") == 2 && find_line("img=4 probe=GP_XLO flags=0x20 fad=15850 count=8"));

    /* both seq 22 and seq 23 with crc: the seq 22 read lines get crc= too */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.head_model = 1; f.cdi_folder = 4; f.pregap_model = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=4\ngap_probe=on\npregap_probe=on\ngap_crc=on\nthroughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 0 && f.sent_bad == 0);
    CHECK(strstr(line_of("img=4 probe=GP_LO_REF "), " crc=") && strstr(line_of("img=4 probe=GP_SCAN_R n=1 "), " crc="));
    CHECK(find_line("img=4 probe=GP_TRK_SKIP a_fad=150 b_fad=11852 src=scan reads=2 reason=no_gap"));   /* as in record 22 */
    CHECK(find_line("img=4 probe=GP_CNT0_SKIP") < find_line("img=4 probe=GP_PG_START"));
}

static void test_seq23_example_cfg(void)
{
    static fake f;
    static char text[4096];
    gp_config c;
    size_t n;
    FILE *in = fopen("../gdprobe.cfg.seq23.example", "rb");
    CHECK(in != NULL);
    if (!in) return;
    n = fread(text, 1, sizeof(text) - 1, in);
    fclose(in);
    text[n] = 0;
    CHECK(n < 2048);   /* main.c reads at most cfgbuf - 1 bytes */
    gp_config_defaults(&c);
    gp_config_parse(&c, text);
    CHECK(c.unknown_keys == 0 && !strcmp(c.label, "stock") && c.seq == 23);
    CHECK(c.do_switch && c.n_images == 3 && c.images[0] == 4 && c.images[1] == 3 && c.images[2] == 10 && c.return_to == 2);
    CHECK(c.gap_probe && c.gap_crc && c.pregap_probe && c.pregap_cross == 16 && c.n_pregap_low == 4 && c.n_pregap_offs == 8);
    CHECK(!c.head_probe && !c.swap_regpoll && !c.swap_devreset && !c.swap_reswitch && !c.devreset_only && !c.devreset_cmd);
    CHECK(!c.do_cdread_flags && !c.do_throughput && !c.do_seek && !c.do_identify && c.cdread_flags == 0x20);
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.head_model = 1; f.cdi_folder = 4; f.gdi_folder = 3;
    f.pregap_model = 1;
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.sent_bad == 0 && f.resets == 0 && f.n_mounts == 3 + 1 && find_line("probe=END abort=0"));
    CHECK(count_of("probe=GP_PG_START ") == 3 && count_of("probe=GP_XPRE1 ") == 3 && count_of("probe=GP_XLO ") == 3);
    CHECK(count_of("probe=GP_CNT0_SKIP ") == 3 && find_line("img=3 probe=GP_XPRE3 flags=0x20 fad=45140 count=16"));
    CHECK(f.now < 240ull * 1000000u);
}

/* seq 24: CTRL regions, deferred risky reads */
static void test_ctl_regions(void)
{
    uint8_t t0[408], t1[408];
    gp_toc_area a[2];
    gp_gap_plan p;
    gp_ctl_region g[3];
    /* GDI */
    {
        static const uint8_t c0[2] = { 0x41, 0x01 }, c1[1] = { 0x41 };
        static const uint32_t f0[2] = { 150, 906 }, f1[1] = { 45150 };
        static const int n0[2] = { 1, 2 }, n1[1] = { 3 };
        mk_toc(t0, c0, f0, n0, 2, 0x01, 1506);
        mk_toc(t1, c1, f1, n1, 1, 0x41, 65150);
    }
    gp_toc_parse(t0, &a[0]); gp_toc_parse(t1, &a[1]);
    CHECK(a[0].leadout_ctl == 0x01 && a[1].leadout_ctl == 0x41);
    gp_gap_plan_make(a, &p);
    CHECK(gp_ctl_regions(a, &p, g) == 3);
    CHECK(!strcmp(g[0].name, "GAP") && g[0].prev_num == 1 && g[0].in_fad == 166 && g[0].next_ctl == 0x01 && g[0].off[0] == 755);
    CHECK(!strcmp(g[1].name, "HDG") && g[1].prev_num == 2 && g[1].prev_ctl == 0x01 && g[1].in_fad == 922);
    CHECK(g[1].has_next && g[1].next_num == 3 && g[1].next_ctl == 0x41 && g[1].next_in == 45166 && g[1].rev_in == 45166);
    CHECK(g[1].n_off == 3 && g[1].off[0] == 1506 && g[1].off[1] == 23252 && g[1].off[2] == 44999);
    CHECK(!strcmp(g[2].name, "LO") && g[2].prev_num == 3 && g[2].prev_ctl == 0x41 && g[2].in_fad == 45166 && !g[2].has_next);
    CHECK(g[2].next_ctl == 0x41 && g[2].off[0] == 65151 && g[2].rev_num == 1 && g[2].rev_ctl == 0x41 && g[2].rev_in == 166);
    /* audio-only CD: lead-out after an audio track, lead-out entry CTRL 0x01 */
    {
        static const uint8_t ctl[2] = { 0x01, 0x01 };
        static const uint32_t fad[2] = { 150, 602 };
        static const int tno[2] = { 1, 2 };
        mk_toc(t0, ctl, fad, tno, 2, 0x01, 1004);
    }
    gp_toc_parse(t0, &a[0]); gp_toc_parse(t0, &a[1]);
    gp_gap_plan_make(a, &p);
    CHECK(gp_ctl_regions(a, &p, g) == 2 && !strcmp(g[0].name, "GAP") && g[0].off[0] == 451);
    CHECK(!strcmp(g[1].name, "LO") && g[1].prev_ctl == 0x01 && g[1].next_ctl == 0x01 && g[1].off[0] == 1005);
    memset(a, 0, sizeof(a));
    gp_gap_plan_make(a, &p);
    CHECK(gp_ctl_regions(a, &p, g) == 0);
}

static void test_runner_seq24(void)
{
    static fake f;
    gp_config c;
    const char *l, *hang;

    gp_config_defaults(&c);
    CHECK(c.risky_cross == GP_RISKY_INLINE && !c.ctl_probe);
    gp_config_parse(&c, "risky_cross=end\nctl_probe=on\n");
    CHECK(c.risky_cross == GP_RISKY_END && c.ctl_probe && c.unknown_keys == 0);
    gp_config_parse(&c, "risky_cross=off\nrisky_cross=later\nctl_probe=2\n");
    CHECK(c.risky_cross == GP_RISKY_OFF && c.unknown_keys == 2);

    /* GDI 3, CDI 4 (the one that hangs on a read past the data end), deferred risky reads */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.head_model = 1; f.cdi_folder = 4; f.gdi_folder = 3;
    f.pregap_model = 1; f.hang_folder = 4;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3,4\nreturn_to=2\npregap_probe=on\ngap_crc=on\nrisky_cross=end\nctl_probe=on\n"
                        "throughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 1 && f.sent_bad == 0 && find_line("probe=END abort=1"));
    CHECK(count_of("probe=GP_RISKY_SKIP cases=XEND,XLD,XLO reason=deferred_to_end") == 2);
    /* the normal cases of both images ran before the risky phase */
    hang = find_line("img=4 probe=GP_RISKY case=XLO");
    CHECK(hang && find_line("img=4 probe=GP_CTL_REGION region=LO") && find_line("img=4 probe=GP_CTL_REGION region=LO") < hang);
    CHECK(find_line("img=3 probe=GP_RISKY_START") > find_line("img=4 probe=GP_CTLR_LO_IN_POS"));
    /* GDI XGAP2 starts on track 1 data: checked with one sector, deferred; XGAP3 starts in the gap: sent inline */
    CHECK(find_line("img=3 probe=GP_XGAP2_CHK flags=0x20 fad=748 count=1 ssz=2048 sense=none"));
    CHECK(find_line("img=3 probe=GP_XGAP2_SKIP fad=748 reason=starts_on_data_deferred_to_end"));
    CHECK(find_line("img=3 probe=GP_XGAP3_CHK flags=0x20 fad=44992 count=1 ssz=- sense=5/21/00"));
    CHECK(find_line("img=3 probe=GP_XGAP3 flags=0x20 fad=44992 count=16") < find_line("img=3 probe=GP_RISKY_START"));
    /* image 3's risky phase: XGAP2, XLD, XLO (CHECK on that folder), then XEND (no gap below the pregap) */
    l = find_line("img=3 probe=GP_RISKY case=XGAP2 fad=748 count=16 edge=756 crosses=data_into_pregap note=may_hang");
    CHECK(l && l > find_line("img=3 probe=GP_RISKY_START"));
    CHECK(find_line("img=3 probe=GP_RISKY case=XLD fad=1498 count=16 edge=1506 crosses=ld_leadout note=may_hang") > l);
    CHECK(find_line("img=3 probe=GP_XLD flags=0x20 fad=1498 count=16 ssz=- sense=5/21/00"));
    CHECK(find_line("img=3 probe=GP_RISKY case=XLO fad=65142 count=16 edge=65150 crosses=leadout"));
    CHECK(find_line("img=3 probe=GP_XEND_SKIP a_fad=150 b_fad=906 hi=755 src=scan reads=2 reason=no_gap") > find_line("img=3 probe=GP_XLO "));
    /* image 4: XGAP2 starts in the gap (no risky read), no HD area, then XLO never completes */
    CHECK(find_line("img=4 probe=GP_XGAP2_CHK flags=0x20 fad=11694 count=1 ssz=- sense=5/21/00") && !find_line("img=4 probe=GP_RISKY case=XGAP2"));
    CHECK(find_line("img=4 probe=GP_RISKY case=XLO fad=15846 count=16 edge=15854 crosses=leadout note=may_hang"));
    l = line_of("img=4 probe=GP_XLO ");
    CHECK(l > hang && strstr(l, "rc=timeout") && strstr(l, "bytes=16384"));
    CHECK(find_line("probe=ABORT reason=drive-not-answering") > hang && !find_line("img=4 probe=GP_RISKY case=XEND"));
    CHECK(f.n_mounts == 2 + 2);   /* 3, 4, then 3, 4 again; no return_to after the abort */

    /* CTRL lines (the fake reports 0x41 everywhere) */
    CHECK(find_line("img=3 probe=GP_CTL_REGION region=HDG prev=2/0x01 in=922 next=3/0x41 rev=3/0x41/45166 off=1506,23252,44999"));
    CHECK(find_line("img=3 probe=GP_CTLF_HDG_IN flags=0x20 fad=922 count=1 ssz=2352"));
    CHECK(find_line("img=3 probe=GP_CTLF_HDG_OFF0 flags=0x20 fad=1506 count=1 ssz=- sense=5/21/00"));
    l = line_of("img=3 probe=GP_CTL region=HDG dir=fwd off_fad=1506 ");
    CHECK(strstr(l, "off_rc=check trk=3 idx=1 ctl=0x41 scd_trk=3 scd_ctl=0x41 scd_rel=0 last_read=2/0x01 prev_query=2/0x41 next=3/0x41 match=const41,prev_query,next"));
    CHECK(find_line("img=3 probe=GP_CTL region=HDG dir=fwd off_fad=44999 ") && find_line("img=3 probe=GP_CTLR_HDG_PRE flags=0x20 fad=45166"));
    CHECK(find_line("img=3 probe=GP_CTL region=HDG dir=rev off_fad=23252 off_rc=check") && find_line("img=3 probe=GP_CTLR_HDG_IN flags=0x20 fad=922"));
    CHECK(strstr(line_of("img=3 probe=GP_CTL region=GAP dir=fwd off_fad=755 "), "off_rc=ok off_track=0"));
    CHECK(strstr(line_of("img=3 probe=GP_CTL region=LO dir=fwd off_fad=65151 "), "next=- leadout_entry=0x41 match="));
    CHECK(find_line("img=4 probe=GP_CTL_REGION region=GAP prev=1/0x01 in=166 next=2/0x41 rev=2/0x41/11868 off=11701"));
    CHECK(strstr(line_of("img=4 probe=GP_CTL region=GAP dir=fwd off_fad=11701 "), "last_read=1/0x01"));
    CHECK(strstr(line_of("img=4 probe=GP_CTL_REGION region=LO "), "rev=1/0x01/166 off=15855"));

    /* risky_cross=off: nothing risky is sent; switch=off + end: the risky phase on the mounted image */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.head_model = 1; f.cdi_folder = 4; f.pregap_model = 1; f.hang_folder = 4;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=4\npregap_probe=on\nrisky_cross=off\nthroughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 0 && find_line("img=4 probe=GP_RISKY_SKIP cases=XEND,XLD,XLO reason=risky_cross_off"));
    CHECK(!find_line("GP_XEND") && !find_line("GP_XLO") && !find_line("GP_DSCAN") && !find_line("GP_RISKY ") && !find_line("GP_CTL"));
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.head_model = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "pregap_probe=on\nrisky_cross=end\nthroughput=off\nseek=off\ncdread_flags_test=off\n");
    CHECK(run_fake(&f, &c) == 0 && find_line("img=0 probe=GP_RISKY_START") > find_line("img=0 probe=GP_RISKY_SKIP"));
    CHECK(find_line("img=0 probe=GP_RISKY case=XLO") && find_line("probe=END abort=0"));
    /* inline (seq 23 default): no GP_RISKY lines */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.head_model = 1; f.cdi_folder = 4; f.pregap_model = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=4\npregap_probe=on\nthroughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 0 && find_line("img=4 probe=GP_XEND ") && !find_line("GP_RISKY"));
}

/* ------------------------------------------------------- seq 25: DMA reads */
static void test_runner_seq25(void)
{
    static fake f;
    gp_config c;
    uint8_t cdb[12];
    const char *l;
    int op, i;

    /* config */
    gp_config_defaults(&c);
    CHECK(!c.dma_probe && c.dma_data && !c.dma_audio && c.n_dma_counts == 4 && c.dma_counts[3] == 16 && c.n_dma_modes == 3);
    CHECK(c.dma_mode_list[0] == GP_DMA_EXACT && c.dma_mode_list[1] == GP_DMA_FLOOR && c.dma_mode_list[2] == GP_DMA_PAD && c.dma_flags == 0x20 && c.dma_pio_ref);
    CHECK(c.dma_timeout_ms == 3000 && c.dma_late_ms == 300 && c.dma_scd && !c.dma_throughput && c.dma_tp_batches == 64);
    gp_config_parse(&c, "dma_probe=on\ndma_audio=on\ndma_counts=7, 3\ndma_modes=exact\ndma_flags=0x29\ndma_timeout_ms=1\n"
                        "dma_late_ms=99999\ndma_throughput=on\ndma_tp_batches=999\ndma_tp_max_ms=5\ndma_pio_ref=off\ndma_data=off\n");
    CHECK(c.dma_probe && c.dma_audio && !c.dma_data && c.n_dma_counts == 2 && c.dma_counts[0] == 7 && c.dma_counts[1] == 3);
    CHECK(c.n_dma_modes == 1 && c.dma_mode_list[0] == GP_DMA_EXACT && c.dma_flags == 0x28 && c.dma_timeout_ms == 200);
    CHECK(c.dma_late_ms == 5000 && c.dma_throughput && c.dma_tp_batches == 256 && c.dma_tp_max_ms == 1000 && !c.dma_pio_ref);
    CHECK(c.unknown_keys == 0);
    gp_config_parse(&c, "dma_modes=pad,pad\ndma_modes=fast\ndma_counts=17\ndma_probe=maybe\n");
    CHECK(c.n_dma_modes == 1 && c.unknown_keys == 4);
    gp_config_parse(&c, "dma_modes=exact,pad\n");
    CHECK(c.n_dma_modes == 2 && c.dma_mode_list[0] == GP_DMA_EXACT && c.dma_mode_list[1] == GP_DMA_PAD);
    gp_config_parse(&c, "dma_modes=floor, exact ,pad\n");
    CHECK(c.n_dma_modes == 3 && c.dma_mode_list[0] == GP_DMA_FLOOR && c.dma_mode_list[1] == GP_DMA_EXACT && c.dma_mode_list[2] == GP_DMA_PAD);
    gp_config_parse(&c, "dma_modes=floor,exact,floor\ndma_modes=exact,pad,floor,exact\ndma_modes=\n");
    CHECK(c.n_dma_modes == 3 && c.unknown_keys == 7);

    /* whitelist: the DMA feature is for CD_READ only */
    memset(cdb, 0, 12); cdb[0] = 0x30; cdb[1] = 0x20; cdb[3] = 0xB0; cdb[10] = 7;
    CHECK(wl_check_dma(cdb, 0, 0x24) && wl_check_cfg(cdb, 0, 0x24));
    cdb[10] = 16; CHECK(wl_check_dma(cdb, 0, 0x24));
    cdb[10] = 17; CHECK(wl_check_cfg(cdb, 0, 0x24) && !wl_check_dma(cdb, 0, 0x24));   /* 64 sectors pass PIO, not DMA */
    cdb[10] = 0; CHECK(!wl_check_dma(cdb, 0, 0x24));
    cdb[10] = 7; cdb[1] = 0x30; CHECK(!wl_check_dma(cdb, 0, 0x24));
    cdb[1] = 0x21; CHECK(!wl_check_dma(cdb, 0, 0x24));
    for (op = 0; op < 256; op++) {
        for (i = 0; i < 4; i++) {
            memset(cdb, 0, 12);
            cdb[0] = (uint8_t)op; cdb[1] = (i & 1) ? 0x82 : 0x20; cdb[2] = 1; cdb[3] = (i & 2) ? 1 : 0; cdb[4] = 10; cdb[10] = 7;
            if (op != 0x30)
                CHECK(!wl_check_dma(cdb, 1, 0x24));
        }
    }
    for (i = 0; i < wl_count; i++)
        CHECK((wl_table[i].opcode == 0x30) == (wl_table[i].dma_ok != 0));

    /* GDI image 3 (data 45150.., audio track 2 at 906..1506), a drive that DMAs like PIO */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.head_model = 1; f.gdi_folder = 3;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=2\ndma_probe=on\ndma_audio=on\ndma_counts=1,7,16\ndma_modes=pad,exact\n"
                        "throughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 0 && f.sent_bad == 0 && f.dma_nonread == 0);
    CHECK(find_line("img=3 probe=DM_START data_fad=45150 data_end=65150 audio_fad=906 audio_end=1506 data_cases=1 audio_cases=1 flags=0x20"));
    CHECK(find_line("img=3 probe=DM_BASE_POS "));
    /* data: 1, 7, 16 sectors are multiples of 32 bytes (2048 each): one run, mode al */
    l = line_of("img=3 probe=DM_D7_al ");
    CHECK(strstr(l, "kind=data flags=0x20 fad=") && strstr(l, "count=7 mode=al exp=14336 dma_len=14336 extent=14336 ext_ok=1 pad_written=0 over=0 sense=none dma_done=1"));
    CHECK(strstr(l, "rc=ok") && strstr(l, " bytes=14336 ") && strstr(l, " scrc=") && !find_line("DM_D7_pad") && !find_line("DM_D7_exact"));
    l = line_of("img=3 probe=DM_D7_al_PIO ");
    CHECK(strstr(l, "ref=pio") && strstr(l, "same=1") && strstr(l, " crc="));
    CHECK(strstr(line_of("img=3 probe=DM_D7_al_POS "), "stat_rc=ok st=pause trk=3") && strstr(line_of("img=3 probe=DM_D7_al_POS "), "d_start=+7 d_end=+0"));
    CHECK(find_line("img=3 probe=DM_D7_al_PIO_POS ") > find_line("img=3 probe=DM_D7_al_PIO "));
    /* audio: 1 and 7 sectors are not multiples of 32 (2352 = 73.5 x 32): pad then exact; 16 sectors are */
    l = line_of("img=3 probe=DM_A7_pad ");
    CHECK(strstr(l, "kind=audio flags=0x20") && strstr(l, "count=7 mode=pad exp=16464 dma_len=16480 extent=16464 ext_ok=1 pad_written=0 over=0"));
    l = line_of("img=3 probe=DM_A7_exact ");
    CHECK(strstr(l, "mode=exact exp=16464 dma_len=16464 extent=16464 ext_ok=1") && strstr(l, " bytes=16464 ") && strstr(l, "dma_done=1"));
    CHECK(strstr(line_of("img=3 probe=DM_A7_exact_PIO "), "same=1"));
    CHECK(strstr(line_of("img=3 probe=DM_A1_pad "), "exp=2352 dma_len=2368") && find_line("img=3 probe=DM_A1_exact "));
    CHECK(strstr(line_of("img=3 probe=DM_A16_al "), "dma_len=37632 extent=37632") && !find_line("DM_A16_pad") && !find_line("DM_A16_exact"));
    CHECK(strstr(line_of("img=3 probe=DM_A7_pad_POS "), "trk=2") && !find_line("DM_STOP") && find_line("probe=END abort=0"));
    CHECK(f.dma_calls == 8 && f.dma_max_len == 37632);   /* data 1,7,16 once each; audio 1 and 7 twice, 16 once */
    /* no DMA probe lines when it is off */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.gdi_folder = 3;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=2\nthroughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 0 && !find_line("probe=DM_") && f.dma_calls == 0);

    /* a drive that truncates an unaligned DMA to a multiple of 32 bytes: the extent shows it, nothing is stopped */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.head_model = 1; f.gdi_folder = 3; f.dma_model = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=2\ndma_probe=on\ndma_data=off\ndma_audio=on\ndma_counts=7\ndma_modes=exact\n"
                        "throughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 0);
    l = line_of("img=3 probe=DM_A7_exact ");
    CHECK(strstr(l, "dma_len=16464 extent=16448 ext_ok=0") && strstr(line_of("img=3 probe=DM_A7_exact_PIO "), "cmp_n=16448 same=1"));

    /* a drive whose unaligned DMA never completes: timeout, only that (kind, mode) stops, other modes / kinds go on */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.head_model = 1; f.gdi_folder = 3; f.dma_model = 2;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=2\ndma_probe=on\ndma_data=off\ndma_audio=on\ndma_counts=7,2\ndma_modes=pad,exact\n"
                        "throughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 0 && f.sent_bad == 0);
    CHECK(strstr(line_of("img=3 probe=DM_A7_exact "), "rc=timeout") && strstr(line_of("img=3 probe=DM_A7_exact "), "dma_done=0") &&
          strstr(line_of("img=3 probe=DM_A7_exact "), "aborted=1"));
    CHECK(find_line("img=3 probe=DM_STOP reason=dma-incomplete rc=timeout done=0 aborted=1 slot=audio_exact fails=1") && count_of("probe=DM_STOP") == 1);
    CHECK(find_line("img=3 probe=DM_A7_exact_POS ") && find_line("img=3 probe=DM_A2_al ") && !find_line("DM_A2_al_SKIP"));
    CHECK(strstr(line_of("img=3 probe=DM_A7_pad "), "dma_done=1") && f.dma_calls == 3 && find_line("probe=END abort=0"));
    /* the same slot on a later image is skipped */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.head_model = 1; f.gdi_folder = 3; f.cdi_folder = 4; f.dma_model = 2;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3,4\nreturn_to=2\ndma_probe=on\ndma_data=off\ndma_audio=on\ndma_counts=7\ndma_modes=exact,pad\n"
                        "throughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 0 && find_line("img=4 probe=DM_A7_exact_SKIP reason=dma-stopped") && find_line("img=4 probe=DM_A7_pad ") && count_of("probe=DM_STOP") == 1);

    /* stock-like engine (record 25): whole 32-byte blocks only; an unaligned or padded length never completes (extent =
     * floor32 of the sector), a rounded-down length completes and leaves the rest of the sector in the drive (DRQ) */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.head_model = 1; f.gdi_folder = 3; f.dma_model = 5;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=2\ndma_probe=on\ndma_audio=on\ndma_counts=1,7\n"
                        "dma_throughput=on\ndma_tp_batches=4\nthroughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 0 && f.sent_bad == 0 && f.dma_nonread == 0);
    l = line_of("img=3 probe=DM_A1_exact ");
    CHECK(strstr(l, "mode=exact exp=2352 dma_len=2352 extent=2336 ext_ok=0 ") && strstr(l, "dma_done=0") && strstr(l, "rc=proto") && strstr(l, "aborted=1"));
    l = line_of("img=3 probe=DM_A1_floor ");
    CHECK(strstr(l, "mode=floor exp=2352 dma_len=2336 extent=2336 ext_ok=0 ") && strstr(l, "dma_done=1") && strstr(l, "rc=ok") &&
          strstr(l, "drq=1 drq_bc=16 drain=16 ") && strstr(l, " tail="));
    CHECK(strstr(line_of("img=3 probe=DM_A1_floor_PIO "), "cmp_n=2336 same=1 tail_ok=1"));
    CHECK(strstr(line_of("img=3 probe=DM_A1_floor_POS "), "d_start=+1 d_end=+0"));
    l = line_of("img=3 probe=DM_A1_pad ");
    CHECK(strstr(l, "dma_len=2368 extent=2336") && strstr(l, "dma_done=0"));
    CHECK(count_of("probe=DM_STOP") == 2 && find_line("slot=audio_exact") && find_line("slot=audio_pad"));
    /* the 7-sector cases: exact and pad are skipped (their slots stopped), floor runs: 16464 -> 16448, 16 bytes left */
    CHECK(find_line("img=3 probe=DM_A7_exact_SKIP reason=dma-stopped") && find_line("img=3 probe=DM_A7_pad_SKIP reason=dma-stopped"));
    CHECK(strstr(line_of("img=3 probe=DM_A7_floor "), "dma_len=16448 extent=16448") && strstr(line_of("img=3 probe=DM_A7_floor "), "drain=16 "));
    /* audio and data: aligned 16 audio sectors not asked here; data cases and the throughput are untouched by the audio stops */
    CHECK(strstr(line_of("img=3 probe=DM_D7_al "), "dma_done=1") && strstr(line_of("img=3 probe=DM_TP_SUMMARY "), "ok=4 ") && find_line("probe=END abort=0"));

    /* a drive that answers a DMA read with CHECK (no DMA ran): recorded, DMA goes on */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.head_model = 1; f.gdi_folder = 3; f.cd_no_raw = 0;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=2\ndma_probe=on\ndma_data=off\ndma_audio=on\ndma_counts=2\ndma_flags=0x28\n"
                        "throughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x28\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(strstr(line_of("img=3 probe=DM_A2_al "), "rc=check") && strstr(line_of("img=3 probe=DM_A2_al "), "sense=5/64/00") && !find_line("DM_STOP"));
    CHECK(!find_line("DM_A2_al_PIO "));

    /* refused: flags outside the whitelist, and a transport without DMA */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.gdi_folder = 3;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=2\ndma_probe=on\ndma_flags=0x30\nthroughput=off\nseek=off\ncdread_flags_test=off\n");
    CHECK(run_fake(&f, &c) == 0 && f.dma_calls == 0 && strstr(line_of("img=3 probe=DM_D1_pad "), "rc=refused"));   /* 0x30 has bit 4: 2352-byte sectors, not a multiple of 32 */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.gdi_folder = 3; f.no_dma = 1;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=2\ndma_probe=on\nthroughput=off\nseek=off\ncdread_flags_test=off\n");
    CHECK(run_fake(&f, &c) == 0 && find_line("img=3 probe=DM_SKIP reason=transport-has-no-dma") && !find_line("DM_D1"));

    /* throughput: DMA batches, PIO batches, CRCs equal */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.head_model = 1; f.gdi_folder = 3;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=2\ndma_probe=on\ndma_counts=1\ndma_throughput=on\ndma_tp_batches=8\n"
                        "throughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 0 && f.sent_bad == 0);
    CHECK(find_line("img=3 probe=DM_TP_START fad=45150 count=16 dma_len=32768 batches=8 max_ms=10000"));
    CHECK(find_line("img=3 probe=DM_TP_007 fad=45262 rc=ok done=1 extent=32768"));
    l = line_of("img=3 probe=DM_TP_SUMMARY ");
    CHECK(strstr(l, "ok=8 bytes=262144 us=") && strstr(l, " kib_s=") && strstr(l, " kib_s_dma="));
    l = line_of("img=3 probe=DM_TPP_SUMMARY ");
    CHECK(strstr(l, "ok=8 bytes=262144") && strstr(l, "crc_mismatch=0"));
    /* a throughput that is cut by the time limit: virtual clock, 20+16 ms per batch */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.head_model = 1; f.gdi_folder = 3;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=2\ndma_probe=on\ndma_data=off\ndma_throughput=on\ndma_tp_batches=256\ndma_tp_max_ms=1000\n"
                        "throughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 0);
    l = line_of("img=3 probe=DM_TP_SUMMARY ");
    CHECK(l && !strstr(l, "ok=256 ") && !strstr(l, "ok=0 "));
    /* DMA corrupts one batch: the PIO pass reports it */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.head_model = 1; f.gdi_folder = 3; f.dma_model = 3;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=on\nimages=3\nreturn_to=2\ndma_probe=on\ndma_data=off\ndma_throughput=on\ndma_tp_batches=4\n"
                        "throughput=off\nseek=off\ncdread_flags_test=off\ncdread_flags=0x20\n");
    CHECK(run_fake(&f, &c) == 0);
    CHECK(strstr(line_of("img=3 probe=DM_TPP_SUMMARY "), "crc_mismatch=4"));

    /* a drive whose DMA never completes at all (even aligned): one timeout stops the DMA probes, the run goes on */
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.dma_model = 4;
    gp_config_defaults(&c);
    gp_config_parse(&c, "switch=off\ndma_probe=on\nthroughput=off\nseek=off\ncdread_flags_test=off\n");
    CHECK(run_fake(&f, &c) == 0 && f.dma_calls == 1 && find_line("img=0 probe=DM_STOP reason=dma-incomplete") && find_line("probe=END abort=0"));
}

static void test_seq25_example_cfg(void)
{
    static fake f;
    static char text[4096];
    gp_config c;
    size_t n;
    FILE *in = fopen("../gdprobe.cfg.seq25b.example", "rb");
    CHECK(in != NULL);
    if (!in) return;
    n = fread(text, 1, sizeof(text) - 1, in);
    fclose(in);
    text[n] = 0;
    gp_config_defaults(&c);
    gp_config_parse(&c, text);
    CHECK(c.unknown_keys == 0 && !strcmp(c.label, "stock") && c.seq == 26);
    CHECK(c.do_switch && c.n_images == 3 && c.images[0] == 3 && c.images[1] == 4 && c.images[2] == 11 && c.return_to == 2);
    CHECK(c.dma_probe && c.dma_data && c.dma_audio && c.dma_throughput && c.n_dma_counts == 4 && c.n_dma_modes == 3);
    CHECK(c.dma_mode_list[0] == GP_DMA_EXACT && c.dma_mode_list[1] == GP_DMA_FLOOR && c.dma_mode_list[2] == GP_DMA_PAD && c.dma_flags == 0x20 && c.dma_pio_ref && c.dma_tp_batches == 64);
    CHECK(!c.swap_regpoll && !c.swap_devreset && !c.head_probe && !c.gap_probe && !c.pregap_probe && !c.ctl_probe);
    CHECK(!c.do_cdread_flags && !c.do_throughput && !c.do_seek && !c.do_identify && c.cdread_flags == 0x20);
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.audio_only_folder = 11; f.passive = 1; f.head_model = 1;
    f.cdi_folder = 4; f.gdi_folder = 3;
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.sent_bad == 0 && f.resets == 0 && f.n_mounts == 3 + 1 && f.dma_nonread == 0);
    CHECK(count_of("probe=DM_START ") == 3 && count_of("probe=DM_TP_SUMMARY ") == 2 && !find_line("probe=DM_STOP"));
    CHECK(find_line("img=11 probe=DM_START data_fad=0 data_end=0 audio_fad=150 audio_end=752 data_cases=0 audio_cases=1"));
    CHECK(find_line("img=3 probe=DM_A7_exact ") && find_line("img=4 probe=DM_A7_exact ") && find_line("img=11 probe=DM_A7_exact_PIO "));
    CHECK(find_line("probe=END abort=0") && f.now < 300ull * 1000000u);
}

static void test_seq24_example_cfg(void)
{
    static fake f;
    static char text[4096];
    gp_config c;
    size_t n;
    FILE *in = fopen("../gdprobe.cfg.seq24.example", "rb");
    CHECK(in != NULL);
    if (!in) return;
    n = fread(text, 1, sizeof(text) - 1, in);
    fclose(in);
    text[n] = 0;
    CHECK(n < 2048);   /* main.c reads at most cfgbuf - 1 bytes */
    gp_config_defaults(&c);
    gp_config_parse(&c, text);
    CHECK(c.unknown_keys == 0 && !strcmp(c.label, "stock") && c.seq == 24);
    CHECK(c.do_switch && c.n_images == 3 && c.images[0] == 3 && c.images[1] == 10 && c.images[2] == 4 && c.return_to == 2);
    CHECK(c.gap_probe && c.gap_crc && c.pregap_probe && c.ctl_probe && c.risky_cross == GP_RISKY_OFF);
    CHECK(!c.head_probe && !c.swap_regpoll && !c.swap_devreset && !c.swap_reswitch && !c.devreset_only && !c.devreset_cmd);
    CHECK(!c.do_cdread_flags && !c.do_throughput && !c.do_seek && !c.do_identify && c.cdread_flags == 0x20);
    memset(&f, 0, sizeof(f)); f.ua = 1; f.mounted = 2; f.passive = 1; f.head_model = 1; f.cdi_folder = 4; f.gdi_folder = 3;
    f.pregap_model = 1; f.hang_folder = 4;
    CHECK(run_fake(&f, &c) == 0);
    CHECK(f.sent_bad == 0 && f.resets == 0 && f.n_mounts == 3 + 1 && find_line("probe=END abort=0"));
    CHECK(count_of("probe=GP_PG_START ") == 3 && count_of("probe=GP_RISKY_SKIP ") == 3 && !find_line("probe=GP_RISKY "));
    CHECK(count_of("probe=GP_CTL_REGION region=HDG") == 1 && count_of("probe=GP_CTL ") >= 10);
    CHECK(find_line("img=3 probe=GP_START") < find_line("img=10 probe=GP_START") && find_line("img=10 probe=GP_START") < find_line("img=4 probe=GP_START"));
    CHECK(f.now < 300ull * 1000000u);
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
    test_cfg_r148();
    test_whitelist_ata();
    test_runner_regpoll();
    test_runner_devreset();
    test_runner_regpoll_safety();
    test_runner_seq17();
    test_seq17_example_cfg();
    test_runner_seq20();
    test_seq20_example_cfg();
    test_runner_audio_only();
    test_r148_example_cfg();
    test_runner_seq21();
    test_seq21_example_cfg();
    test_toc_plan();
    test_runner_seq22();
    test_seq22_example_cfg();
    test_runner_seq23();
    test_seq23_example_cfg();
    test_ctl_regions();
    test_runner_seq24();
    test_seq24_example_cfg();
    test_runner_seq25();
    test_seq25_example_cfg();
    printf("OK: %d checks passed\n", g_checks);
    return 0;
}
