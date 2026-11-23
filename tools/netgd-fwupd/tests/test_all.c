/* Host tests for netgd-fwupd (plain gcc, no KOS): CRC / header / vector checks against the
 * shared vectors of tools/fwimage, image parsing, the 0xF9 protocol against a fake NetGD
 * (success, BadTarget, BadVector, Locked, CRC mismatch, flash error, timeouts, write session,
 * resume), text layout, result text, the chunked result writer.
 *
 *   gcc ... test_all.c ../fwcore.c ../fwmsg.c ../resout.c      (see run_tests.sh)
 *   env FWIMAGE_VECTORS  folder with crc32.txt header.txt vtable.txt (default ../../fwimage/vectors)
 *   env FWUPD_GOLDEN_DIR folder with *.ogfw made by fwimage.py (tests/gen_golden.py); optional
 */
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "../fwcore.h"
#include "../fwconfirm.h"
#include "../resout.h"
#include "fakedev.h"

static int g_checks;
#define CHECK(c) do { g_checks++; if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); exit(1); } } while (0)

/* ---------------------------------------------------------------- helpers */

static void put32(uint8_t *p, uint32_t v)
{
    p[0] = (uint8_t)v; p[1] = (uint8_t)(v >> 8); p[2] = (uint8_t)(v >> 16); p[3] = (uint8_t)(v >> 24);
}

static uint32_t g_rng = 12345;
static uint8_t rnd8(void) { g_rng = g_rng * 1664525u + 1013904223u; return (uint8_t)(g_rng >> 24); }

static uint32_t cidr_of(int part) { return part == FW_PART_2E ? FW_CIDR_2E : FW_CIDR_4E; }

/* header block (512) + payload, as fwimage.py writes them. Returns bytes written. */
static size_t make_image(uint8_t *out, uint32_t kind, int part, uint32_t len, uint32_t version)
{
    uint32_t dest = kind == FW_KIND_CODE ? FW_PLANE0_BASE : FW_PLANE1_BASE, i;
    uint8_t *pay = out + 512;
    memset(out, 0, 512);
    for (i = 0; i < len; i++) pay[i] = rnd8();
    if (kind == FW_KIND_CODE) {
        put32(pay + 0, fw_part_sram0_end(part));
        put32(pay + 4, dest + 0x101);
        put32(pay + 8, dest + 0x103);
        put32(pay + 12, dest + 0x105);
    }
    put32(out + 0, FW_MAGIC); put32(out + 4, FW_FORMAT); put32(out + 8, kind);
    put32(out + 12, cidr_of(part)); put32(out + 16, dest); put32(out + 20, len);
    put32(out + 24, fw_crc32(pay, len)); put32(out + 28, version);
    put32(out + 32, fw_crc32(out, 32));
    return 512 + len;
}

static void refresh_hdr(uint8_t *hdr) { put32(hdr + 32, fw_crc32(hdr, 32)); }

typedef struct { uint8_t buf[FW_MAX_FILE + 4096]; size_t len; fw_file f; } filebuf;

/* a file with a code image (len_code) and, if len_bits, a bitstream image */
static void build_file(filebuf *fb, int part, uint32_t len_code, uint32_t len_bits)
{
    fw_vres res;
    fb->len = make_image(fb->buf, FW_KIND_CODE, part, len_code, 0x12345678);
    if (len_bits) fb->len += make_image(fb->buf + fb->len, FW_KIND_BITSTREAM, part, len_bits, 0x12345678);
    CHECK(fw_parse_file(fb->buf, fb->len, &fb->f, &res) == 0);
}

static size_t read_file(const char *path, uint8_t *buf, size_t cap)
{
    FILE *f = fopen(path, "rb");
    size_t n;
    if (!f) return (size_t)-1;
    n = fread(buf, 1, cap, f);
    fclose(f);
    return n;
}

static int hexval(char c) { return c >= '0' && c <= '9' ? c - '0' : (c | 32) - 'a' + 10; }

static size_t unhex(const char *s, uint8_t *out)
{
    size_t n = 0;
    if (!strcmp(s, "-")) return 0;
    for (; s[0] && s[1]; s += 2) out[n++] = (uint8_t)(hexval(s[0]) * 16 + hexval(s[1]));
    return n;
}

static int next_row(FILE *f, char *tok[], int max, char *line, size_t cap)
{
    while (fgets(line, (int)cap, f)) {
        int n = 0;
        char *p = strtok(line, " \t\r\n");
        if (!p || p[0] == '#') continue;
        while (p && n < max) { tok[n++] = p; p = strtok(NULL, " \t\r\n"); }
        return n;
    }
    return 0;
}

static const char *vec_dir(void)
{
    const char *d = getenv("FWIMAGE_VECTORS");
    return d ? d : "../../fwimage/vectors";
}

static FILE *open_vec(const char *name)
{
    char p[512];
    FILE *f;
    snprintf(p, sizeof p, "%s/%s", vec_dir(), name);
    f = fopen(p, "r");
    if (!f) { printf("cannot open %s\n", p); exit(1); }
    return f;
}

/* ------------------------------------------------------- shared vectors */

static void test_vectors(void)
{
    FILE *f;
    char line[4096], *tok[10];
    int n, rows;

    f = open_vec("crc32.txt");
    rows = 0;
    while ((n = next_row(f, tok, 10, line, sizeof line)) >= 2) {
        static uint8_t d[1024];
        size_t len = unhex(tok[0], d);
        CHECK(fw_crc32(d, len) == (uint32_t)strtoul(tok[1], NULL, 16));
        rows++;
    }
    fclose(f);
    CHECK(rows >= 4);
    CHECK(fw_crc32("123456789", 9) == 0xCBF43926u);
    {   /* incremental */
        uint32_t c = fw_crc32_update(0, "1234", 4);
        CHECK(fw_crc32_update(c, "56789", 5) == 0xCBF43926u);
    }

    f = open_vec("vtable.txt");
    rows = 0;
    while ((n = next_row(f, tok, 10, line, sizeof line)) >= 8) {
        /* part dest len sp reset nmi hardfault expect */
        uint32_t v[4];
        int part = !strcmp(tok[0], "2e") ? FW_PART_2E : FW_PART_4E;
        int ok;
        v[0] = (uint32_t)strtoul(tok[3], NULL, 16); v[1] = (uint32_t)strtoul(tok[4], NULL, 16);
        v[2] = (uint32_t)strtoul(tok[5], NULL, 16); v[3] = (uint32_t)strtoul(tok[6], NULL, 16);
        ok = fw_check_vectors(v, (uint32_t)strtoul(tok[1], NULL, 16), (uint32_t)strtoul(tok[2], NULL, 16), part) == 0;
        CHECK(ok == !strcmp(tok[7], "OK"));
        rows++;
    }
    fclose(f);
    CHECK(rows >= 10);

    f = open_vec("header.txt");
    rows = 0;
    while ((n = next_row(f, tok, 10, line, sizeof line)) >= 3) {
        /* part header-bytes(36, hex) expect: parse, then check against the board's part */
        uint8_t blk[512];
        fw_header h;
        int part = !strcmp(tok[0], "2e") ? FW_PART_2E : FW_PART_4E;
        fw_verr e;
        const char *want = tok[2];
        memset(blk, 0, sizeof blk);
        CHECK(unhex(tok[1], blk) == 36);
        e = fw_header_parse(blk, &h);
        if (e == FWV_OK) e = fw_header_check(&h, part);
        if (!strcmp(want, "OK")) CHECK(e == FWV_OK);
        else if (!strcmp(want, "BadHeader")) CHECK(e == FWV_MAGIC || e == FWV_FORMAT || e == FWV_HDR_CRC || e == FWV_KIND);
        else if (!strcmp(want, "BadTarget")) CHECK(e == FWV_TARGET);
        else if (!strcmp(want, "BadDest")) CHECK(e == FWV_DEST);
        else if (!strcmp(want, "BadLength")) CHECK(e == FWV_LEN);
        else CHECK(0);
        rows++;
    }
    fclose(f);
    CHECK(rows >= 10);
}


/* ---- format 3 (resident boot region): the shared vectors header3 / desc / trailer / image3 ---- */

static const char *verr_name(fw_verr e)
{
    switch (e) {
    case FWV_OK: return "OK";
    case FWV_MAGIC: case FWV_FORMAT: case FWV_HDR_CRC: case FWV_KIND: return "BadHeader";
    case FWV_TARGET: return "BadTarget";
    case FWV_DEST: return "BadDest";
    case FWV_LEN: return "BadLength";
    case FWV_PAYLOAD_CRC: return "CrcMismatch";
    case FWV_VECTOR: return "BadVector";
    case FWV_DESC: return "BadDescriptor";
    case FWV_UNSEALED: return "Unsealed";
    case FWV_TRAILER: return "BadTrailer";
    default: return "other";
    }
}

static void test_vectors3(void)
{
    FILE *f;
    static char line[40000];
    char *tok[10];
    static uint8_t buf[16384];
    int n, rows;

    f = open_vec("header3.txt");
    rows = 0;
    while ((n = next_row(f, tok, 10, line, sizeof line)) >= 3) {
        /* part header-bytes(40, hex) expect: format 3 only, then check against the board's part */
        uint8_t blk[512];
        fw_header h;
        int part = !strcmp(tok[0], "2e") ? FW_PART_2E : FW_PART_4E;
        fw_verr e;
        memset(blk, 0, sizeof blk);
        CHECK(unhex(tok[1], blk) == 40);
        e = fw_header_parse_ex(blk, &h, FW_ACCEPT_3);
        if (e == FWV_OK) e = fw_header_check(&h, part);
        if (strcmp(verr_name(e), tok[2])) { printf("header3 %s %s: got %s\n", tok[1], tok[2], verr_name(e)); CHECK(0); }
        /* the same header read by a board that takes both formats must not do worse */
        if (!strcmp(tok[2], "OK")) CHECK(fw_header_parse(blk, &h) == FWV_OK && h.format == 3 && h.layout == FW_LAYOUT);
        rows++;
    }
    fclose(f);
    CHECK(rows >= 20);

    /* header.txt (format 2) once more through the strict format-2 reader; its fmt=3 row is BadHeader */
    f = open_vec("header.txt");
    rows = 0;
    while ((n = next_row(f, tok, 10, line, sizeof line)) >= 3) {
        uint8_t blk[512];
        fw_header h;
        int part = !strcmp(tok[0], "2e") ? FW_PART_2E : FW_PART_4E;
        fw_verr e;
        memset(blk, 0, sizeof blk);
        CHECK(unhex(tok[1], blk) == 36);
        e = fw_header_parse_ex(blk, &h, FW_ACCEPT_2);
        if (e == FWV_OK) e = fw_header_check(&h, part);
        if (strcmp(verr_name(e), tok[2])) { printf("header(2) %s %s: got %s\n", tok[1], tok[2], verr_name(e)); CHECK(0); }
        rows++;
    }
    fclose(f);
    CHECK(rows >= 20);

    f = open_vec("desc.txt");
    rows = 0;
    while ((n = next_row(f, tok, 10, line, sizeof line)) >= 3) {
        uint8_t d[64];
        CHECK(unhex(tok[0], d) == 32);
        if (strcmp(verr_name(fw_check_descriptor(d, (uint32_t)strtoul(tok[1], NULL, 16))), tok[2])) {
            printf("desc %s %s: got %s\n", tok[0], tok[2],
                   verr_name(fw_check_descriptor(d, (uint32_t)strtoul(tok[1], NULL, 16))));
            CHECK(0);
        }
        rows++;
    }
    fclose(f);
    CHECK(rows >= 10);

    f = open_vec("trailer.txt");
    rows = 0;
    while ((n = next_row(f, tok, 10, line, sizeof line)) >= 2) {
        size_t len = unhex(tok[0], buf);
        if (strcmp(verr_name(fw_check_trailer(buf, len)), tok[1])) { printf("trailer %s %s\n", tok[0], tok[1]); CHECK(0); }
        rows++;
    }
    fclose(f);
    CHECK(rows >= 8);

    f = open_vec("image3.txt");
    rows = 0;
    while ((n = next_row(f, tok, 10, line, sizeof line)) >= 3) {
        /* board-part file-hex expect: one whole image, read by a format-3-only (resident) reader */
        size_t len = unhex(tok[1], buf);
        int board = !strcmp(tok[0], "2e") ? FW_PART_2E : FW_PART_4E;
        fw_file ff;
        fw_vres res;
        const char *got;
        if (fw_parse_file_ex(buf, len, &ff, &res, FW_ACCEPT_3) != 0) got = verr_name(res.err);
        else got = ff.part != board ? "BadTarget" : "OK";
        if (strcmp(got, tok[2])) { printf("image3 row %d (%s): got %s want %s\n", rows, tok[0], got, tok[2]); CHECK(0); }
        if (rows == 0) {   /* the first row is a good 2E image: the result text shows the format */
            char text[1024];
            fw_describe_text(text, sizeof text, "/pc/x/netgd.ogfw", len, &ff, NULL);
            CHECK(strstr(text, "image0 kind=code dest=0x00082000") && strstr(text, "format=3"));
            CHECK(fw_verr_msgid(FWV_DESC) != fw_verr_msgid(FWV_TRAILER) && fw_verr_msgid(FWV_UNSEALED) != FWM_V_READ);
        }
        /* a board that takes both formats parses everything but the format-2 row the same way */
        if (!strcmp(tok[2], "BadHeader")) {
            fw_file f2;
            CHECK(fw_parse_file(buf, len, &f2, NULL) != 0 || f2.img[0].h.format == 2);
        }
        rows++;
    }
    fclose(f);
    CHECK(rows >= 10);
}

/* ------------------------------------------------------------ parsing */

static void test_parse(void)
{
    static filebuf fb;
    fw_vres res;
    fw_file f;
    size_t half;

    /* good files */
    build_file(&fb, FW_PART_2E, 0x4000, 0);
    CHECK(fb.f.n == 1 && fb.f.part == FW_PART_2E && fb.f.img[0].h.len == 0x4000 && fb.f.img[0].h.version == 0x12345678);
    build_file(&fb, FW_PART_4E, 0x20000, 0x8000);
    CHECK(fb.f.n == 2 && fb.f.part == FW_PART_4E && fb.f.img[1].h.kind == FW_KIND_BITSTREAM && fb.f.img[1].h.dest == FW_PLANE1_BASE);
    CHECK(fb.f.img[0].payload == fb.buf + 512 && fb.f.img[1].header_block == fb.buf + 512 + 0x20000);
    CHECK(fb.len == FW_MAX_FILE - (0x20000 - 0x8000));

    /* the biggest legal file is accepted, one byte more is not */
    build_file(&fb, FW_PART_4E, 0x20000, 0x20000);
    CHECK(fb.len == FW_MAX_FILE);
    CHECK(fw_parse_file(fb.buf, fb.len + 1, &f, &res) != 0 && res.err == FWV_TOO_BIG);

    /* each way a file can be wrong */
    CHECK(fw_parse_file(fb.buf, 0, &f, &res) != 0 && res.err == FWV_EMPTY);
    build_file(&fb, FW_PART_2E, 0x2000, 0);
    CHECK(fw_parse_file(fb.buf, 100, &f, &res) != 0 && res.err == FWV_SHORT);
    CHECK(fw_parse_file(fb.buf, 511, &f, &res) != 0 && res.err == FWV_SHORT);
    CHECK(fw_parse_file(fb.buf, fb.len - 1, &f, &res) != 0 && res.err == FWV_TRUNC);
    CHECK(fw_parse_file(fb.buf, 512, &f, &res) != 0 && res.err == FWV_TRUNC);
    {   /* trailing bytes: a partial second header */
        CHECK(fw_parse_file(fb.buf, fb.len + 40, &f, &res) != 0 && res.err == FWV_SHORT && res.index == 1);
    }

    build_file(&fb, FW_PART_2E, 0x2000, 0);
    fb.buf[0] ^= 1;
    CHECK(fw_parse_file(fb.buf, fb.len, &f, &res) != 0 && res.err == FWV_MAGIC);
    fb.buf[0] ^= 1;
    put32(fb.buf + 4, 1); refresh_hdr(fb.buf);
    CHECK(fw_parse_file(fb.buf, fb.len, &f, &res) != 0 && res.err == FWV_FORMAT);   /* format 1 is refused */
    put32(fb.buf + 4, 2);
    refresh_hdr(fb.buf);
    CHECK(fw_parse_file(fb.buf, fb.len, &f, &res) == 0);
    fb.buf[28] ^= 0x10;                                   /* a header field changes, CRC does not follow */
    CHECK(fw_parse_file(fb.buf, fb.len, &f, &res) != 0 && res.err == FWV_HDR_CRC);
    fb.buf[28] ^= 0x10;
    put32(fb.buf + 8, 2); refresh_hdr(fb.buf);
    CHECK(fw_parse_file(fb.buf, fb.len, &f, &res) != 0 && res.err == FWV_KIND);
    put32(fb.buf + 8, 0);
    put32(fb.buf + 12, 0x2819A561); refresh_hdr(fb.buf);   /* a SAM3U1E-ish id */
    CHECK(fw_parse_file(fb.buf, fb.len, &f, &res) != 0 && res.err == FWV_TARGET);
    put32(fb.buf + 12, FW_CIDR_2E + 5); refresh_hdr(fb.buf);   /* later silicon revision is fine */
    CHECK(fw_parse_file(fb.buf, fb.len, &f, &res) == 0 && f.part == FW_PART_2E);
    put32(fb.buf + 12, FW_CIDR_2E);
    put32(fb.buf + 16, 0x100000); refresh_hdr(fb.buf);
    CHECK(fw_parse_file(fb.buf, fb.len, &f, &res) != 0 && res.err == FWV_DEST);
    put32(fb.buf + 16, FW_PLANE0_BASE);
    put32(fb.buf + 20, 0); refresh_hdr(fb.buf);
    CHECK(fw_parse_file(fb.buf, fb.len, &f, &res) != 0 && res.err == FWV_LEN);
    put32(fb.buf + 20, 100); refresh_hdr(fb.buf);
    CHECK(fw_parse_file(fb.buf, fb.len, &f, &res) != 0 && res.err == FWV_LEN);
    put32(fb.buf + 20, 0x20200); refresh_hdr(fb.buf);
    CHECK(fw_parse_file(fb.buf, fb.len, &f, &res) != 0 && res.err == FWV_LEN);
    put32(fb.buf + 20, 0x2000); refresh_hdr(fb.buf);
    CHECK(fw_parse_file(fb.buf, fb.len, &f, &res) == 0);
    fb.buf[512 + 100] ^= 1;                              /* payload byte flipped */
    CHECK(fw_parse_file(fb.buf, fb.len, &f, &res) != 0 && res.err == FWV_PAYLOAD_CRC);
    fb.buf[512 + 100] ^= 1;

    /* a vector table that fails, with CRCs fixed up so only the vector check can object */
    put32(fb.buf + 512, 0x20008000);                     /* 4E's SRAM on a 2E image */
    put32(fb.buf + 24, fw_crc32(fb.buf + 512, 0x2000)); refresh_hdr(fb.buf);
    CHECK(fw_parse_file(fb.buf, fb.len, &f, &res) != 0 && res.err == FWV_VECTOR);
    put32(fb.buf + 512, 0x20004000);
    put32(fb.buf + 512 + 4, 0x80100);                    /* not Thumb */
    put32(fb.buf + 24, fw_crc32(fb.buf + 512, 0x2000)); refresh_hdr(fb.buf);
    CHECK(fw_parse_file(fb.buf, fb.len, &f, &res) != 0 && res.err == FWV_VECTOR);

    /* two images: same kind twice, different parts, a bitstream for a 2E, a third image */
    build_file(&fb, FW_PART_4E, 0x2000, 0);
    half = fb.len;
    memcpy(fb.buf + half, fb.buf, half);
    CHECK(fw_parse_file(fb.buf, 2 * half, &f, &res) != 0 && res.err == FWV_DUP && res.index == 1);
    build_file(&fb, FW_PART_4E, 0x2000, 0);
    half = make_image(fb.buf + fb.len, FW_KIND_BITSTREAM, FW_PART_2E, 0x2000, 1);   /* bitstream, but "2E" */
    CHECK(fw_parse_file(fb.buf, fb.len + half, &f, &res) != 0 && res.err == FWV_DEST && res.index == 1);
    build_file(&fb, FW_PART_4E, 0x2000, 0x2000);
    CHECK(fw_parse_file(fb.buf, fb.len, &f, &res) == 0);
    {
        size_t one = 512 + 0x2000;
        put32(fb.buf + one + 12, FW_CIDR_2E); refresh_hdr(fb.buf + one);    /* image 1 names a 2E */
        CHECK(fw_parse_file(fb.buf, fb.len, &f, &res) != 0 && (res.err == FWV_DEST || res.err == FWV_MIXED) && res.index == 1);
    }
    build_file(&fb, FW_PART_4E, 0x2000, 0x2000);
    {
        size_t extra = make_image(fb.buf + fb.len, FW_KIND_CODE, FW_PART_4E, 0x2000, 1);
        CHECK(fw_parse_file(fb.buf, fb.len + extra, &f, &res) != 0 && res.err == FWV_DUP && res.index == 2);
    }
}

/* ------------------------------------------------- reading in chunks */

typedef struct { const uint8_t *d; size_t n, pos, max_read; int calls; } memrd;
static memrd g_mr;
static void *mr_open(const char *p) { (void)p; g_mr.pos = 0; return &g_mr; }
static size_t mr_read(void *f, void *b, size_t n)
{
    memrd *m = f;
    size_t take = n < m->n - m->pos ? n : m->n - m->pos;
    m->calls++;
    if (n > m->max_read) m->max_read = n;
    memcpy(b, m->d + m->pos, take);
    m->pos += take;
    return take;
}
static int mr_err(void *f) { (void)f; return 0; }
static void mr_close(void *f) { (void)f; }
static void *mr_open_fail(const char *p) { (void)p; return NULL; }

static void test_read(void)
{
    static uint8_t src[100000], dst[100000];
    fw_rd io = { mr_open, mr_read, mr_err, mr_close };
    size_t len;
    size_t i;
    for (i = 0; i < sizeof src; i++) src[i] = (uint8_t)i;
    g_mr.d = src; g_mr.n = sizeof src; g_mr.max_read = 0; g_mr.calls = 0;
    CHECK(fw_read_all(&io, "x", dst, sizeof dst, &len) == 0 && len == sizeof src && !memcmp(src, dst, len));
    CHECK(g_mr.max_read <= FW_READ_CHUNK && g_mr.calls >= 12);     /* modest chunks */
    g_mr.max_read = 0;
    CHECK(fw_read_all(&io, "x", dst, 5000, &len) == 0 && len == 5000);   /* bounded by the buffer */
    io.open = mr_open_fail;
    CHECK(fw_read_all(&io, "x", dst, sizeof dst, &len) == -1 && len == 0);
}

/* ------------------------------------------------------ device check */

static void test_identify(void)
{
    fake_t *k = calloc(1, sizeof *k);
    fw_env env;
    char model[48];
    fk_init(k, FW_PART_2E);
    fk_env(k, &env);
    CHECK(fw_check_device(&env, model, sizeof model) == 0 && !strcmp(model, "OpenGDEMU SD Disk"));
    strcpy(k->model, "NetGD SD Disk");
    CHECK(fw_check_device(&env, model, sizeof model) == 0);
    strcpy(k->model, "SanDisk Ultra 32GB");
    CHECK(fw_check_device(&env, model, sizeof model) == 1 && !strcmp(model, "SanDisk Ultra 32GB"));
    k->identify_fail = 1;
    CHECK(fw_check_device(&env, model, sizeof model) == -1);
    k->identify_fail = 0;
    k->timeout_at_call = 0;
    CHECK(fw_model_is_netgd("openGDEMU x") && fw_model_is_netgd("My NETGD") && !fw_model_is_netgd("GDEMU"));
    free(k);
}

/* ------------------------------------------------------ protocol runs */

static fake_t *g_k;

static void no_force_reset(const fake_t *k)
{
    int i;
    for (i = 0; i < k->ncalls; i++) CHECK(k->feat[i] != 0x59);     /* never the force reset */
}

static int index_of(const fake_t *k, uint8_t feat, int nth)
{
    int i, n = 0;
    for (i = 0; i < k->ncalls; i++)
        if (k->feat[i] == feat && ++n == nth) return i;
    return -1;
}

/* the exact feature sequence of the calls, collapsing consecutive F9/56 into one 'D' */
static void seq_of(const fake_t *k, char *out)
{
    int i, n = 0;
    for (i = 0; i < k->ncalls; i++) {
        char c;
        switch (k->feat[i]) {
        case FW_FEAT_ARM: c = 'A'; break;
        case FW_FEAT_ENTER: c = 'E'; break;
        case FW_FEAT_HEADER: c = 'H'; break;
        case FW_FEAT_DATA: c = 'D'; break;
        case FW_FEAT_VERIFY: c = 'V'; break;
        case FW_FEAT_RESET: c = 'R'; break;
        default: c = '?';
        }
        if (c == 'D' && n && out[n - 1] == 'D') continue;
        out[n++] = c;
    }
    out[n] = 0;
}

static fake_t *run_update(int board, const filebuf *fb, fw_report *rep, const fw_options *o)
{
    static fake_t *k;
    fw_env env;
    free(k);
    k = calloc(1, sizeof *k);
    fk_init(k, board);
    g_k = k;
    fk_env(k, &env);
    fw_update(&env, &fb->f, o, rep);
    no_force_reset(k);
    return k;
}

static void test_success(void)
{
    static filebuf fb;
    fw_report rep;
    char seq[64];
    fake_t *k;
    uint32_t sum = 0;
    int i;
    fw_options o;

    /* 2E: one image, 0x6000 bytes = 48 blocks (one data command with the default chunk of 64) */
    build_file(&fb, FW_PART_2E, 0x6000, 0);
    k = run_update(FW_PART_2E, &fb, &rep, NULL);
    CHECK(rep.outcome == FWR_OK && rep.handed_over && !rep.resumed && rep.images_verified == 1 && rep.retries_used == 0);
    seq_of(k, seq);
    CHECK(!strcmp(seq, "AEHDVR"));
    /* the wire values, written out (fwupdate.rs:42-43, main.rs:110-114): not through the constants */
    CHECK(k->feat[0] == 0x4F && k->feat[1] == 0x53 && k->feat[2] == 0x55 && k->feat[3] == 0x56 &&
          k->feat[4] == 0x57 && k->feat[5] == 0x58);
    CHECK(!memcmp(k->flash[0], fb.f.img[0].payload, 0x6000));
    CHECK(k->flash[0][0x6000] == 0xA5 && k->mode == FK_RESET && k->prepared == 1);
    CHECK(fk_calls_of(k, FW_FEAT_DATA) == 1 && k->nblk[index_of(k, FW_FEAT_DATA, 1)] == 48 && k->cnt[index_of(k, FW_FEAT_DATA, 1)] == 48);
    /* the keys, as fwupdate.rs defines them, and nothing else carries one */
    CHECK(k->used_key[0] && k->key[0] == 0x4F4744555041ULL && k->feat[0] == 0x4F);
    CHECK(k->used_key[1] && k->key[1] == 0x4744454D5521ULL && k->feat[1] == 0x53);
    for (i = 2; i < k->ncalls; i++) CHECK(!k->used_key[i]);
    CHECK(fw_outcome_msgid(&rep) == FWM_OUT_OK);
    CHECK(k->nprogress >= 4 && k->log_lines >= 6);

    /* 4E: code then bitstream, each with its own header / data / verify; reset once at the end */
    build_file(&fb, FW_PART_4E, 0x20000, 0x7000);
    k = run_update(FW_PART_4E, &fb, &rep, NULL);
    CHECK(rep.outcome == FWR_OK && rep.images_verified == 2);
    seq_of(k, seq);
    CHECK(!strcmp(seq, "AEHDVHDVR"));
    CHECK(!memcmp(k->flash[0], fb.f.img[0].payload, 0x20000));
    CHECK(!memcmp(k->flash[1], fb.f.img[1].payload, 0x7000));
    CHECK(k->flash[1][0x7000] == 0xA5);
    /* 0x20000 bytes = 256 blocks in chunks of 64 = 4 data commands, then 0x7000 = 56 blocks = 1 */
    CHECK(fk_calls_of(k, FW_FEAT_DATA) == 5);
    for (i = 0; i < k->ncalls; i++) if (k->feat[i] == FW_FEAT_DATA) sum += k->nblk[i];
    CHECK(sum == 256 + 56);
    CHECK(index_of(k, FW_FEAT_RESET, 1) == k->ncalls - 1 && fk_calls_of(k, FW_FEAT_RESET) == 1);

    /* a chunk of 256 blocks goes out as sector count 0 (ATA: 0 means 256) */
    fw_options_default(&o);
    o.chunk_blocks = 256;
    k = run_update(FW_PART_4E, &fb, &rep, &o);
    CHECK(rep.outcome == FWR_OK);
    CHECK(k->nblk[index_of(k, FW_FEAT_DATA, 1)] == 256 && k->cnt[index_of(k, FW_FEAT_DATA, 1)] == 0);
    CHECK(!memcmp(k->flash[0], fb.f.img[0].payload, 0x20000));
    /* ... and 1-block chunks work too */
    o.chunk_blocks = 1;
    build_file(&fb, FW_PART_2E, 0x1000, 0);
    k = run_update(FW_PART_2E, &fb, &rep, &o);
    CHECK(rep.outcome == FWR_OK && fk_calls_of(k, FW_FEAT_DATA) == 8 && !memcmp(k->flash[0], fb.f.img[0].payload, 0x1000));
    /* a code-only image for a 4E board is legal (the updater takes it) */
    build_file(&fb, FW_PART_4E, 0x4000, 0);
    k = run_update(FW_PART_4E, &fb, &rep, NULL);
    CHECK(rep.outcome == FWR_OK);
}

static void test_rejections(void)
{
    static filebuf fb;
    fw_report rep;
    char seq[64];
    fake_t *k;

    /* BadTarget: a 4E image sent to a 2E board. Nothing is written, no reset */
    build_file(&fb, FW_PART_4E, 0x4000, 0x2000);
    k = run_update(FW_PART_2E, &fb, &rep, NULL);
    CHECK(rep.outcome == FWR_REJECTED && rep.dev_error == FWS_BADTARGET && !rep.flash_touched && rep.handed_over);
    seq_of(k, seq);
    CHECK(!strcmp(seq, "AEH"));
    CHECK(k->flash[0][0] == 0xA5 && k->flash[1][0] == 0xA5);
    CHECK(fw_outcome_msgid(&rep) == FWM_OUT_REJECTED_CLEAN && rep.failed_image == 0);
    CHECK(strstr(k->all_log, "name=BadTarget") != NULL);

    /* and the other way round */
    build_file(&fb, FW_PART_2E, 0x4000, 0);
    k = run_update(FW_PART_4E, &fb, &rep, NULL);
    CHECK(rep.outcome == FWR_REJECTED && rep.dev_error == FWS_BADTARGET);

    /* BadVector: the board has less SRAM than the image's part. Raised before any page is written,
     * not retried, flash untouched */
    build_file(&fb, FW_PART_4E, 0x4000, 0);
    {
        fw_env env;
        free(g_k);
        g_k = calloc(1, sizeof *g_k);
        fk_init(g_k, FW_PART_4E);
        g_k->sram0_end_override = 0x20002000;
        fk_env(g_k, &env);
        fw_update(&env, &fb.f, NULL, &rep);
        k = g_k;
        no_force_reset(k);
    }
    CHECK(rep.outcome == FWR_REJECTED && rep.dev_error == FWS_BADVECTOR && !rep.flash_touched && rep.retries_used == 0);
    seq_of(k, seq);
    CHECK(!strcmp(seq, "AEHD"));
    CHECK(k->flash[0][300] == 0xA5);
    CHECK(fw_outcome_msgid(&rep) == FWM_OUT_REJECTED_CLEAN);

    /* Locked: refused at the header */
    build_file(&fb, FW_PART_2E, 0x4000, 0);
    {
        fw_env env;
        free(g_k);
        g_k = calloc(1, sizeof *g_k);
        fk_init(g_k, FW_PART_2E);
        g_k->lock_bits = 1;
        fk_env(g_k, &env);
        fw_update(&env, &fb.f, NULL, &rep);
        k = g_k;
    }
    CHECK(rep.outcome == FWR_REJECTED && rep.dev_error == FWS_LOCKED && !rep.flash_touched);
    seq_of(k, seq);
    CHECK(!strcmp(seq, "AEH"));

    /* an image 1 refused after image 0 went in: reported as touched, and no reset */
    build_file(&fb, FW_PART_4E, 0x4000, 0x2000);
    {
        fw_env env;
        free(g_k);
        g_k = calloc(1, sizeof *g_k);
        fk_init(g_k, FW_PART_4E);
        g_k->err_feat = FW_FEAT_HEADER; g_k->err_nth = 2; g_k->err_code = FWS_LOCKED;
        fk_env(g_k, &env);
        fw_update(&env, &fb.f, NULL, &rep);
        k = g_k;
    }
    CHECK(rep.outcome == FWR_REJECTED && rep.flash_touched && rep.failed_image == 1 && rep.images_verified == 1);
    CHECK(fw_outcome_msgid(&rep) == FWM_OUT_REJECTED_DIRTY);
    CHECK(fk_calls_of(k, FW_FEAT_RESET) == 0);
}

static void test_retries(void)
{
    static filebuf fb;
    fw_report rep;
    char seq[64];
    fake_t *k;
    fw_env env;

    build_file(&fb, FW_PART_2E, 0x6000, 0);

    /* the CRC check fails once (a flipped bit in flash): the image is sent again from the header */
    free(g_k);
    g_k = calloc(1, sizeof *g_k);
    fk_init(g_k, FW_PART_2E);
    g_k->corrupt_flash_next_verify = 1;
    fk_env(g_k, &env);
    fw_update(&env, &fb.f, NULL, &rep);
    k = g_k;
    CHECK(rep.outcome == FWR_OK && rep.retries_used == 1);
    seq_of(k, seq);
    CHECK(!strcmp(seq, "AEHDVHDVR"));
    CHECK(!memcmp(k->flash[0], fb.f.img[0].payload, 0x6000));
    CHECK(strstr(k->all_log, "name=CrcMismatch") != NULL && strstr(k->all_log, "retry image=0 attempt=1") != NULL);
    no_force_reset(k);

    /* it keeps failing: 1 try + 2 retries, then give up with flash half-written, never a reset */
    free(g_k);
    g_k = calloc(1, sizeof *g_k);
    fk_init(g_k, FW_PART_2E);
    g_k->corrupt_always = 1;
    fk_env(g_k, &env);
    fw_update(&env, &fb.f, NULL, &rep);
    k = g_k;
    CHECK(rep.outcome == FWR_FAILED && rep.retries_used == 2 && rep.flash_touched && rep.dev_error == FWS_CRCMISMATCH);
    seq_of(k, seq);
    CHECK(!strcmp(seq, "AEHDVHDVHDV"));
    CHECK(fk_calls_of(k, FW_FEAT_RESET) == 0);
    CHECK(fw_outcome_msgid(&rep) == FWM_OUT_FAILED_DIRTY);
    no_force_reset(k);

    /* a flash write error mid-image: the next command is refused by the board until a new header, so
     * the retry starts at the header */
    free(g_k);
    g_k = calloc(1, sizeof *g_k);
    fk_init(g_k, FW_PART_2E);
    g_k->fail_program_at_off = 0x1000;
    fk_env(g_k, &env);
    {
        fw_options o;
        fw_options_default(&o);
        o.chunk_blocks = 4;
        fw_update(&env, &fb.f, &o, &rep);
    }
    k = g_k;
    CHECK(rep.outcome == FWR_FAILED && rep.dev_error == FWS_FLASHERROR && rep.retries_used == 2);
    CHECK(fk_calls_of(k, FW_FEAT_RESET) == 0 && fk_calls_of(k, FW_FEAT_HEADER) == 3);
    CHECK(fk_calls_of(k, FW_FEAT_VERIFY) == 0);

    /* an injected Overrun at the second data command recovers on the retry */
    free(g_k);
    g_k = calloc(1, sizeof *g_k);
    fk_init(g_k, FW_PART_2E);
    g_k->err_feat = FW_FEAT_DATA; g_k->err_nth = 2; g_k->err_code = FWS_OVERRUN;
    fk_env(g_k, &env);
    {
        fw_options o;
        fw_options_default(&o);
        o.chunk_blocks = 8;
        fw_update(&env, &fb.f, &o, &rep);
    }
    k = g_k;
    CHECK(rep.outcome == FWR_OK && rep.retries_used == 1 && !memcmp(k->flash[0], fb.f.img[0].payload, 0x6000));
}

static void test_io_and_refusal(void)
{
    static filebuf fb;
    fw_report rep;
    char seq[64];
    fake_t *k;
    fw_env env;
    int i;

    build_file(&fb, FW_PART_2E, 0x4000, 0);

    /* a timeout at every single step: always reported, never retried, never a reset or a later step */
    for (i = 1; i <= 5; i++) {
        free(g_k);
        g_k = calloc(1, sizeof *g_k);
        fk_init(g_k, FW_PART_2E);
        g_k->timeout_at_call = i;
        fk_env(g_k, &env);
        fw_update(&env, &fb.f, NULL, &rep);
        k = g_k;
        no_force_reset(k);
        seq_of(k, seq);
        CHECK(rep.outcome == FWR_IO && rep.io_rc == FWIO_TIMEOUT);
        CHECK((int)strlen(seq) == i);                           /* stopped right there */
        CHECK(fk_calls_of(k, FW_FEAT_RESET) == 0);
        CHECK(rep.retries_used == 0);
        /* flash_touched: from the first data command on */
        if (i == 4 || i == 5) CHECK(rep.flash_touched && fw_outcome_msgid(&rep) == FWM_OUT_IO_DIRTY);
        if (i <= 3) CHECK(!rep.flash_touched && fw_outcome_msgid(&rep) == FWM_OUT_IO_CLEAN);
    }

    /* the reset command itself times out: the board has most likely just rebooted */
    free(g_k);
    g_k = calloc(1, sizeof *g_k);
    fk_init(g_k, FW_PART_2E);
    g_k->timeout_at_call = 6;
    fk_env(g_k, &env);
    fw_update(&env, &fb.f, NULL, &rep);
    CHECK(rep.outcome == FWR_OK_UNCONFIRMED && rep.images_verified == 1);
    CHECK(fw_outcome_msgid(&rep) == FWM_OUT_OK_UNCONFIRMED);

    /* the reset is refused: reported, nothing forced */
    free(g_k);
    g_k = calloc(1, sizeof *g_k);
    fk_init(g_k, FW_PART_2E);
    g_k->err_feat = FW_FEAT_RESET; g_k->err_nth = 1; g_k->err_code = FWS_NOTVERIFIED;
    fk_env(g_k, &env);
    fw_update(&env, &fb.f, NULL, &rep);
    k = g_k;
    CHECK(rep.outcome == FWR_NO_RESET && rep.dev_error == FWS_NOTVERIFIED && fk_calls_of(k, FW_FEAT_RESET) == 1);
    no_force_reset(k);

    /* a write session (or an old firmware): arm is refused with plain ABRT; the probe is refused too;
     * nothing else is sent */
    free(g_k);
    g_k = calloc(1, sizeof *g_k);
    fk_init(g_k, FW_PART_2E);
    g_k->write_session = 1;
    fk_env(g_k, &env);
    fw_update(&env, &fb.f, NULL, &rep);
    k = g_k;
    CHECK(rep.outcome == FWR_REFUSED_START && !rep.handed_over && !rep.flash_touched);
    CHECK(k->ncalls == 2 && k->feat[0] == FW_FEAT_ARM && k->feat[1] == FW_FEAT_VERIFY);
    CHECK(fw_outcome_msgid(&rep) == FWM_OUT_REFUSED_START);

    /* armed but the enter step is refused */
    free(g_k);
    g_k = calloc(1, sizeof *g_k);
    fk_init(g_k, FW_PART_2E);
    g_k->err_feat = FW_FEAT_ENTER; g_k->err_nth = 1; g_k->err_code = FWS_REFUSED;
    fk_env(g_k, &env);
    fw_update(&env, &fb.f, NULL, &rep);
    k = g_k;
    CHECK(rep.outcome == FWR_ENTER_FAILED && k->ncalls == 2 && !rep.flash_touched);

    /* every call from the third on is silent: a header that never answers */
    free(g_k);
    g_k = calloc(1, sizeof *g_k);
    fk_init(g_k, FW_PART_2E);
    g_k->timeout_from_call = 3;
    fk_env(g_k, &env);
    fw_update(&env, &fb.f, NULL, &rep);
    CHECK(rep.outcome == FWR_IO && g_k->ncalls == 3);
}

static void test_resume(void)
{
    static filebuf fb;
    fw_report rep;
    char seq[64];
    fake_t *k;
    fw_env env;

    build_file(&fb, FW_PART_4E, 0x8000, 0x4000);

    /* An earlier run failed half-way and left the updater running with an image half received. A new
     * run's arm is refused by the updater (unknown step), the probe F9/57 answers with an updater
     * code, so the run continues from the header instead of giving up. */
    free(g_k);
    g_k = calloc(1, sizeof *g_k);
    fk_init(g_k, FW_PART_4E);
    g_k->mode = FK_UPDATER;
    g_k->t.active = 1; g_k->t.kind = 0; g_k->t.base = FW_PLANE0_BASE; g_k->t.len = 0x8000; g_k->t.off = 0x1000;
    g_k->dirty[0] = 1;
    fk_env(g_k, &env);
    fw_update(&env, &fb.f, NULL, &rep);
    k = g_k;
    CHECK(rep.outcome == FWR_OK && rep.resumed && rep.handed_over);
    seq_of(k, seq);
    CHECK(!strcmp(seq, "AVHDVHDVR"));
    CHECK(!memcmp(k->flash[0], fb.f.img[0].payload, 0x8000) && !memcmp(k->flash[1], fb.f.img[1].payload, 0x4000));
    CHECK(fk_calls_of(k, FW_FEAT_ENTER) == 0);
    no_force_reset(k);

    /* a fresh updater (no header yet) answers the probe NoHeader: also "running" */
    free(g_k);
    g_k = calloc(1, sizeof *g_k);
    fk_init(g_k, FW_PART_4E);
    g_k->mode = FK_UPDATER;
    fk_env(g_k, &env);
    fw_update(&env, &fb.f, NULL, &rep);
    CHECK(rep.outcome == FWR_OK && rep.resumed);

    /* a fully received but uncommitted image (the run died before F9/57): the probe verifies and
     * commits it, which is the one side effect of the probe, and the run goes on from the header */
    build_file(&fb, FW_PART_2E, 0x4000, 0);
    free(g_k);
    g_k = calloc(1, sizeof *g_k);
    fk_init(g_k, FW_PART_2E);
    fk_env(g_k, &env);
    {
        fw_reply r;
        uint32_t off;
        CHECK(fk_f9(g_k, FW_FEAT_ARM, 0, 1, FW_MAGIC_ARM, NULL, 0, 1, &r) == FWIO_OK && !(r.status & 1));
        CHECK(fk_f9(g_k, FW_FEAT_ENTER, 0, 1, FW_MAGIC_ENTER, NULL, 0, 1, &r) == FWIO_OK && !(r.status & 1));
        CHECK(fk_f9(g_k, FW_FEAT_HEADER, 0, 0, 0, fb.f.img[0].header_block, 1, 1, &r) == FWIO_OK && !(r.status & 1));
        for (off = 0; off < 0x4000; off += 0x4000 / 2)
            CHECK(fk_f9(g_k, FW_FEAT_DATA, 8, 0, 0, fb.f.img[0].payload + off, 8, 1, &r) == FWIO_OK && !(r.status & 1));
        CHECK(!g_k->verified_any && g_k->dirty[0]);
    }
    g_k->ncalls = 0;
    fw_update(&env, &fb.f, NULL, &rep);
    k = g_k;
    CHECK(rep.outcome == FWR_OK && rep.resumed);
    seq_of(k, seq);
    CHECK(!strcmp(seq, "AVHDVR"));
    CHECK(!memcmp(k->flash[0], fb.f.img[0].payload, 0x4000));
}

/* --------------------------------------------------------- text, layout */

static void test_text(void)
{
    char lines[12][112];
    int id, n, i;
    const char *euc;

    /* every message exists in the three forms */
    for (id = 1; id < FWM__COUNT; id++) {
        CHECK(fw_msg_en(id)[0] && fw_msg_ja(id)[0] && fw_msg_euc(id)[0]);
        CHECK(strlen(fw_msg_euc(id)) < strlen(fw_msg_ja(id)) + 1);       /* EUC-JP is never longer than UTF-8 */
    }
    CHECK(fw_msg_en(0)[0] == 0 && fw_msg_en(99999)[0] == 0);
    /* status codes map to distinct messages; unknown ones to ST_UNKNOWN */
    CHECK(fw_status_msgid(0x84) == FWM_ST_BADTARGET && fw_status_msgid(0x94) == FWM_ST_LOCKED);
    CHECK(fw_status_msgid(0xA4) == FWM_ST_BADVECTOR && fw_status_msgid(0xB4) == FWM_ST_NOTVERIFIED);
    CHECK(fw_status_msgid(0x74) == FWM_ST_CRCMISMATCH && fw_status_msgid(0x44) == FWM_ST_NOHEADER);
    CHECK(fw_status_msgid(0x55) == FWM_ST_UNKNOWN);
    CHECK(!strcmp(fw_status_name(0x84), "BadTarget") && !strcmp(fw_status_name(0xA4), "BadVector"));
    {
        static const uint8_t codes[] = { 0x00, 0x04, 0x14, 0x24, 0x34, 0x44, 0x54, 0x64, 0x74, 0x84, 0x94, 0xA4, 0xB4 };
        size_t a, b;
        for (a = 0; a < sizeof codes; a++)
            for (b = a + 1; b < sizeof codes; b++) CHECK(fw_status_msgid(codes[a]) != fw_status_msgid(codes[b]));
    }
    /* the Japanese of a status is Japanese (UTF-8 lead bytes) */
    CHECK((unsigned char)fw_msg_ja(FWM_ST_BADTARGET)[0] >= 0xE0);
    CHECK((unsigned char)fw_msg_euc(FWM_ST_BADTARGET)[0] >= 0xA1);

    /* wrapping: ASCII breaks at spaces, wide characters count 2, '\n' breaks */
    n = fw_text_wrap("hello world foo", 11, &lines[0][0], sizeof lines[0], 12);
    CHECK(n == 2 && !strcmp(lines[0], "hello world") && !strcmp(lines[1], "foo"));
    n = fw_text_wrap("hello wonderful world", 12, &lines[0][0], sizeof lines[0], 12);
    CHECK(n == 3 && !strcmp(lines[0], "hello") && !strcmp(lines[1], "wonderful") && !strcmp(lines[2], "world"));
    n = fw_text_wrap("ab\ncd", 10, &lines[0][0], sizeof lines[0], 12);
    CHECK(n == 2 && !strcmp(lines[0], "ab") && !strcmp(lines[1], "cd"));
    n = fw_text_wrap("", 10, &lines[0][0], sizeof lines[0], 12);
    CHECK(n == 1 && lines[0][0] == 0);
    n = fw_text_wrap("abcdefghij", 4, &lines[0][0], sizeof lines[0], 12);
    CHECK(n == 3 && !strcmp(lines[0], "abcd") && !strcmp(lines[2], "ij"));
    n = fw_text_wrap("\xa4\xa2\xa4\xa4\xa4\xa6", 4, &lines[0][0], sizeof lines[0], 12);   /* three hiragana, 4 cells a line */
    CHECK(n == 2 && fw_text_cells(lines[0], strlen(lines[0])) == 4 && fw_text_cells(lines[1], strlen(lines[1])) == 2);
    CHECK(fw_text_cells("\x8e\xb1", 2) == 1);                                              /* half-width kana */

    /* every message wraps within the screen, and the ones the screens give a fixed number of rows to fit */
    for (id = 1; id < FWM__COUNT; id++) {
        euc = fw_msg_euc(id);
        n = fw_text_wrap(euc, 50, &lines[0][0], sizeof lines[0], 12);
        CHECK(n >= 1 && n < 12);
        for (i = 0; i < n; i++) CHECK(fw_text_cells(lines[i], strlen(lines[i])) <= 50);
    }
    n = fw_text_wrap(fw_msg_euc(FWM_UI_CONFIRM_HOLD), 50, &lines[0][0], sizeof lines[0], 12); CHECK(n <= 2);
    n = fw_text_wrap(fw_msg_euc(FWM_UI_WARN_POWER), 50, &lines[0][0], sizeof lines[0], 12); CHECK(n <= 2);
    n = fw_text_wrap(fw_msg_euc(FWM_UI_WARN_SWD), 50, &lines[0][0], sizeof lines[0], 12); CHECK(n <= 2);
    n = fw_text_wrap(fw_msg_euc(FWM_UI_WARN_UNTESTED), 50, &lines[0][0], sizeof lines[0], 12); CHECK(n == 1);
    n = fw_text_wrap(fw_msg_euc(FWM_UI_CONFIRM_TITLE), 50, &lines[0][0], sizeof lines[0], 12); CHECK(n == 1);
    n = fw_text_wrap(fw_msg_euc(FWM_UI_BTN_HELD), 50, &lines[0][0], sizeof lines[0], 12); CHECK(n == 1);     /* one row: ROWS - 2 */
    n = fw_text_wrap(fw_msg_euc(FWM_UI_WARN_NO_BITSTREAM), 50, &lines[0][0], sizeof lines[0], 12); CHECK(n <= 2);
    for (id = FWM_OUT_OK; id <= FWM_OUT_IO_DIRTY; id++) {          /* result screen: a handful of rows */
        n = fw_text_wrap(fw_msg_euc(id), 50, &lines[0][0], sizeof lines[0], 12);
        CHECK(n <= 6);
    }
}

static void test_result_text(void)
{
    static filebuf fb;
    fw_report rep;
    char buf[2048], small[40];
    size_t n;

    build_file(&fb, FW_PART_4E, 0x4000, 0x2000);
    n = fw_describe_text(buf, sizeof buf, "/pc/x/netgd.ogfw", fb.len, &fb.f, "OpenGDEMU SD Disk");
    CHECK(n == strlen(buf));
    CHECK(strstr(buf, "file=/pc/x/netgd.ogfw size=") && strstr(buf, "target=SAM3U4E") && strstr(buf, "device_model=OpenGDEMU SD Disk"));
    CHECK(strstr(buf, " format=2"));
    CHECK(strstr(buf, "image0 kind=code dest=0x00080000 len=16384") && strstr(buf, "image1 kind=bitstream dest=0x00100000 len=8192"));
    CHECK(strstr(buf, "version=305419896"));

    run_update(FW_PART_2E, &fb, &rep, NULL);      /* BadTarget */
    n = fw_summary_text(buf, sizeof buf, &rep);
    CHECK(n == strlen(buf));
    CHECK(strstr(buf, "outcome=rejected") && strstr(buf, "name=BadTarget") && strstr(buf, "error=0x84"));
    CHECK(strstr(buf, "message_en=") && strstr(buf, "message_ja=") && (unsigned char)strstr(buf, "message_ja=")[11] >= 0xE0);
    /* the log lines carry the Japanese reason, in UTF-8 */
    CHECK(strstr(g_k->all_log, "ja=") != NULL);

    /* truncation is safe and always NUL-terminated */
    n = fw_summary_text(small, sizeof small, &rep);
    CHECK(n < sizeof small && small[n] == 0);
    n = fw_describe_text(small, 5, "/pc/x", 1, &fb.f, NULL);
    CHECK(n < 5 && small[n] == 0);

    /* the configuration file */
    {
        char p[64];
        CHECK(fw_cfg_image_path("# c\nimage=/pc/a/b.ogfw\n", p, sizeof p) && !strcmp(p, "/pc/a/b.ogfw"));
        CHECK(fw_cfg_image_path("x=1\r\n  image=/pc/q.ogfw  \r\n", p, sizeof p) && !strcmp(p, "/pc/q.ogfw"));
        CHECK(!fw_cfg_image_path("#image=/pc/a\nfoo\n", p, sizeof p));
        CHECK(!fw_cfg_image_path("image=\n", p, sizeof p));
        CHECK(!fw_cfg_image_path("image=/pc/a/very/long/path/that/will/not/fit/here\n", p, 10));
        CHECK(fw_cfg_image_path("image=/pc/last.ogfw", p, sizeof p) && !strcmp(p, "/pc/last.ogfw"));
    }
}

/* ------------------------------------------------ chunked result writer */

typedef struct { size_t max_write; int writes, flushes, closes; size_t total; int open_fail; char fake; } wio;
static wio g_w;
static void *w_open(const char *p, const char *m) { (void)p; (void)m; return g_w.open_fail ? NULL : &g_w; }
static size_t w_write(const void *p, size_t n, void *f) { (void)p; (void)f; g_w.writes++; g_w.total += n; if (n > g_w.max_write) g_w.max_write = n; return n; }
static int w_flush(void *f) { (void)f; g_w.flushes++; return 0; }
static int w_close(void *f) { (void)f; g_w.closes++; return 0; }

/* the update confirmation: no stuck state of a pad may ever start a flash */
static void test_confirm_gate(void)
{
    enum { COMBO = 0x0F, CANCEL = 0x10, HOLD = 3000 };
    ng_holdconfirm_t c;
    uint32_t t = 0;
    int i, go = 0;
    /* the combination stuck since before the screen */
    memset(&c, 0, sizeof c);
    for (i = 0; i < 2000; i++, t += 30) go |= fw_confirm_step(&c, 1, COMBO, COMBO, CANCEL, t, HOLD);
    CHECK(!go);
    /* no pad / an implausible status (pad 0), whatever the bits */
    memset(&c, 0, sizeof c);
    for (i = 0; i < 2000; i++, t += 30) go |= fw_confirm_step(&c, 0, 0xFFFF, COMBO, CANCEL, t, HOLD);
    CHECK(!go);
    /* released, then the combination with the cancel button also down */
    memset(&c, 0, sizeof c);
    for (i = 0; i < 10; i++, t += 30) go |= fw_confirm_step(&c, 1, 0, COMBO, CANCEL, t, HOLD);
    for (i = 0; i < 2000; i++, t += 30) go |= fw_confirm_step(&c, 1, COMBO | CANCEL, COMBO, CANCEL, t, HOLD);
    CHECK(!go);
    /* a pad change during the hold: the new pad must release first */
    memset(&c, 0, sizeof c);
    for (i = 0; i < 10; i++, t += 30) go |= fw_confirm_step(&c, 1, 0, COMBO, CANCEL, t, HOLD);
    for (i = 0; i < 50; i++, t += 30) go |= fw_confirm_step(&c, 1, COMBO, COMBO, CANCEL, t, HOLD);
    for (i = 0; i < 2000; i++, t += 30) go |= fw_confirm_step(&c, 2, COMBO, COMBO, CANCEL, t, HOLD);
    CHECK(!go);
    /* a real release and a full hold still works, and not before the hold is over */
    memset(&c, 0, sizeof c);
    for (i = 0; i < 10; i++, t += 30) go |= fw_confirm_step(&c, 1, 0, COMBO, CANCEL, t, HOLD);
    for (i = 0; i < 100; i++, t += 30) go |= fw_confirm_step(&c, 1, COMBO, COMBO, CANCEL, t, HOLD);
    CHECK(!go);
    CHECK(fw_confirm_step(&c, 1, COMBO, COMBO, CANCEL, t, HOLD));
}

static void test_resout(void)
{
    static resout o;
    resout_io io = { w_open, w_write, w_flush, w_close };
    static char big[10000];
    memset(&g_w, 0, sizeof g_w);
    memset(big, 'x', sizeof big);
    CHECK(resout_open(&o, &io, "p") == 0);
    CHECK(resout_write(&o, big, sizeof big) == 0);
    CHECK(g_w.writes == 2 && g_w.max_write == 4096 && g_w.flushes == 2);       /* never over 4 KB, flush after each */
    CHECK(resout_sync(&o) == 0 && g_w.total == sizeof big && g_w.max_write <= 4096 && g_w.flushes == 3);
    CHECK(resout_close(&o) == 0 && g_w.closes == 1);
    memset(&g_w, 0, sizeof g_w);
    g_w.open_fail = 1;
    CHECK(resout_open(&o, &io, "p") != 0);
}

/* ------------------------------------------- images built by fwimage.py */

static void test_golden(void)
{
    const char *dir = getenv("FWUPD_GOLDEN_DIR");
    static const char *names[] = { "golden_2e.ogfw", "golden_4e.ogfw", "real_2e.ogfw", "real_4e.ogfw" };
    static const int parts[] = { FW_PART_2E, FW_PART_4E, FW_PART_2E, FW_PART_4E };
    static filebuf fb;
    int i, ran = 0;
    if (!dir) { printf("  (no FWUPD_GOLDEN_DIR: skipping fwimage.py cross-check)\n"); return; }
    for (i = 0; i < 4; i++) {
        char p[600];
        fw_vres res;
        fw_report rep;
        size_t n;
        snprintf(p, sizeof p, "%s/%s", dir, names[i]);
        n = read_file(p, fb.buf, sizeof fb.buf);
        if (n == (size_t)-1) continue;
        CHECK(fw_parse_file(fb.buf, n, &fb.f, &res) == 0);
        CHECK(fb.f.part == parts[i]);
        fb.len = n;
        run_update(parts[i], &fb, &rep, NULL);
        CHECK(rep.outcome == FWR_OK);
        CHECK(!memcmp(g_k->flash[0], fb.f.img[0].payload, fb.f.img[0].h.len));
        if (fb.f.n > 1) CHECK(!memcmp(g_k->flash[1], fb.f.img[1].payload, fb.f.img[1].h.len));
        printf("  %s: %d image(s), %lu bytes: parsed and sent through the fake updater\n", names[i], fb.f.n, (unsigned long)n);
        ran++;
    }
    CHECK(ran >= 2);       /* the golden files must exist when the directory is given */
}

int main(void)
{
    test_vectors();
    test_vectors3();
    test_parse();
    test_read();
    test_identify();
    test_success();
    test_rejections();
    test_retries();
    test_io_and_refusal();
    test_resume();
    test_text();
    test_result_text();
    test_confirm_gate();
    test_resout();
    test_golden();
    printf("netgd-fwupd host tests: %d checks passed\n", g_checks);
    return 0;
}
