/* A fake NetGD secondary device for the host tests: the normal firmware's F9 handling
 * (fwupdate.rs handle/should_enter, ide.rs handle_fw_command) and the RAM updater
 * (mcu/fwupdater/src/main.rs recv_header / recv_data / verify / reset), re-implemented
 * from the Rust, plus fault injection. It answers through the same fw_env.f9 interface
 * the real G1 code implements. */
#ifndef FAKEDEV_H
#define FAKEDEV_H
#include <string.h>
#include "../fwcore.h"

#define FK_MAXCALLS 4096

enum { FK_NORMAL = 0, FK_UPDATER = 1, FK_RESET = 2 };

typedef struct {
    /* ---- configuration ---- */
    int part;                       /* FW_PART_2E / 4E: the board */
    uint32_t sram0_end_override;    /* if non-zero, the board's idea of SRAM0 end */
    int write_session;              /* the normal firmware refuses F9 */
    uint32_t lock_bits;             /* lock regions locked (bit n = 8 KB region n) */
    char model[41];
    int identify_fail;
    /* ---- fault injection ---- */
    int timeout_at_call;            /* 1-based: this call returns FWIO_TIMEOUT */
    int timeout_from_call;          /* every call from this one on times out */
    uint8_t err_feat; int err_nth; uint8_t err_code;    /* nth call of err_feat answers err_code */
    int err_repeat;                 /* keep answering err_code on every later call too */
    int corrupt_flash_next_verify;  /* flip a flash byte before the next verify's CRC */
    int corrupt_always;
    int fail_program_at_off;        /* -1 none; flash write error at this image offset */
    /* ---- state ---- */
    int mode, armed;
    struct { int active, failed; uint32_t kind, base, len, off, crc; int committed; } t;
    int dirty[2], verified_any;
    uint32_t page0[64];
    uint8_t flash[2][FW_PLANE_LEN];
    /* ---- record ---- */
    int ncalls;
    uint8_t feat[FK_MAXCALLS];
    uint8_t cnt[FK_MAXCALLS];
    uint32_t nblk[FK_MAXCALLS];
    int used_key[FK_MAXCALLS];
    uint64_t key[FK_MAXCALLS];
    int prepared;
    uint32_t clock_ms;
    int nprogress;
    int log_lines;
    char last_log[400];
    char all_log[65536];
} fake_t;

static void fk_init(fake_t *k, int part)
{
    memset(k, 0, sizeof *k);
    k->part = part;
    k->fail_program_at_off = -1;
    strcpy(k->model, "OpenGDEMU SD Disk");
    memset(k->flash, 0xA5, sizeof k->flash);      /* "the old firmware" */
}

static int fk_calls_of(const fake_t *k, uint8_t feat)
{
    int i, n = 0;
    for (i = 0; i < k->ncalls; i++) n += k->feat[i] == feat;
    return n;
}

static void fk_ok(fw_reply *r) { r->status = 0x50; r->error = 0; }
static void fk_err(fw_reply *r, uint8_t code) { r->status = 0x51; r->error = code; }

static uint32_t fk_le32(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

static uint32_t fk_lock_mask(uint32_t len)
{
    uint32_t n = (len + 8191) / 8192;
    return (uint32_t)((1ull << n) - 1);
}

static uint32_t fk_sram_end(const fake_t *k)
{
    return k->sram0_end_override ? k->sram0_end_override : fw_part_sram0_end(k->part);
}

/* crc over flash page 0 ram copy + flash pages 1.. */
static uint32_t fk_crc_ram_then_flash(fake_t *k)
{
    uint8_t p0[256];
    uint32_t c, i;
    for (i = 0; i < 64; i++) {
        p0[4 * i] = (uint8_t)k->page0[i]; p0[4 * i + 1] = (uint8_t)(k->page0[i] >> 8);
        p0[4 * i + 2] = (uint8_t)(k->page0[i] >> 16); p0[4 * i + 3] = (uint8_t)(k->page0[i] >> 24);
    }
    c = fw_crc32_update(0, p0, 256);
    return fw_crc32_update(c, k->flash[k->t.kind] + 256, k->t.len - 256);
}

static int fk_updater(fake_t *k, uint8_t feat, uint8_t count, int use_key, uint64_t key,
                      const uint8_t *data, uint32_t nblocks, fw_reply *r)
{
    (void)use_key; (void)key;
    switch (feat) {
    case FW_FEAT_HEADER: {
        fw_header h;
        fw_verr e;
        memset(&k->t, 0, sizeof k->t);
        if (!data || nblocks < 1) { fk_err(r, FWS_OVERRUN); return 0; }
        r->blocks = 1;
        e = fw_header_parse(data, &h);
        if (e != FWV_OK) { fk_err(r, FWS_BADHEADER); return 0; }
        e = fw_header_check(&h, k->part);
        if (e == FWV_TARGET) { fk_err(r, FWS_BADTARGET); return 0; }
        if (e == FWV_DEST) { fk_err(r, FWS_BADDEST); return 0; }
        if (e == FWV_LEN) { fk_err(r, FWS_BADLENGTH); return 0; }
        if (k->lock_bits & fk_lock_mask(h.len)) { fk_err(r, FWS_LOCKED); return 0; }
        k->t.active = 1; k->t.kind = h.kind; k->t.base = h.dest; k->t.len = h.len; k->t.crc = h.crc;
        fk_ok(r);
        return 0;
    }
    case FW_FEAT_DATA: {
        uint32_t n = count == 0 ? 256 : count, b;
        if (!k->t.active) { fk_err(r, FWS_NOHEADER); return 0; }
        if (k->t.failed) { fk_err(r, FWS_FLASHERROR); return 0; }
        if (n * 512 > k->t.len - k->t.off) { fk_err(r, FWS_OVERRUN); return 0; }
        if (nblocks != n) { k->t.failed = 1; r->blocks = nblocks; fk_err(r, FWS_OVERRUN); return 0; }
        for (b = 0; b < n; b++) {
            int half;
            r->blocks = b + 1;
            for (half = 0; half < 2; half++) {
                const uint8_t *src = data + (size_t)b * 512 + (size_t)half * 256;
                if (k->t.off == 0) {
                    uint32_t i, v[4];
                    for (i = 0; i < 64; i++) k->page0[i] = fk_le32(src + 4 * i);
                    if (k->t.kind == FW_KIND_CODE) {
                        uint32_t sp = k->page0[0], end = k->t.base + k->t.len, j;
                        int bad = 0;
                        v[0] = k->page0[0]; v[1] = k->page0[1]; v[2] = k->page0[2]; v[3] = k->page0[3];
                        if (sp % 4 || sp <= 0x20000000u || sp > fk_sram_end(k)) bad = 1;
                        for (j = 1; j < 4; j++) {
                            uint32_t a = v[j] & ~1u;
                            if (!(v[j] & 1) || a < k->t.base || a >= end) bad = 1;
                        }
                        if (bad) { k->t.failed = 1; fk_err(r, FWS_BADVECTOR); return 0; }
                    }
                } else {
                    k->dirty[k->t.kind] = 1;
                    if (k->fail_program_at_off >= 0 && (uint32_t)k->fail_program_at_off == k->t.off) {
                        k->t.failed = 1;
                        fk_err(r, FWS_FLASHERROR);
                        return 0;
                    }
                    memcpy(k->flash[k->t.kind] + k->t.off, src, 256);
                }
                k->t.off += 256;
            }
        }
        fk_ok(r);
        return 0;
    }
    case FW_FEAT_VERIFY: {
        uint8_t p0[256];
        uint32_t i;
        if (!k->t.active) { fk_err(r, FWS_NOHEADER); return 0; }
        if (k->t.failed) { fk_err(r, FWS_FLASHERROR); return 0; }
        if (k->t.off != k->t.len) { fk_err(r, FWS_BADLENGTH); return 0; }
        if (k->corrupt_flash_next_verify || k->corrupt_always) {
            k->flash[k->t.kind][256 * 3 + 7] ^= 0x40;
            k->corrupt_flash_next_verify = 0;
        }
        if (!k->t.committed) {
            if (fk_crc_ram_then_flash(k) != k->t.crc) { fk_err(r, FWS_CRCMISMATCH); return 0; }
            for (i = 0; i < 64; i++) {
                p0[4 * i] = (uint8_t)k->page0[i]; p0[4 * i + 1] = (uint8_t)(k->page0[i] >> 8);
                p0[4 * i + 2] = (uint8_t)(k->page0[i] >> 16); p0[4 * i + 3] = (uint8_t)(k->page0[i] >> 24);
            }
            memcpy(k->flash[k->t.kind], p0, 256);
            k->dirty[k->t.kind] = 1;
            k->t.committed = 1;
        }
        if (fw_crc32(k->flash[k->t.kind], k->t.len) != k->t.crc) { fk_err(r, FWS_CRCMISMATCH); return 0; }
        k->dirty[k->t.kind] = 0;
        k->verified_any = 1;
        fk_ok(r);
        return 0;
    }
    case FW_FEAT_RESET:
        if (k->verified_any && !k->dirty[0] && !k->dirty[1]) { fk_ok(r); k->mode = FK_RESET; return 0; }
        fk_err(r, FWS_NOTVERIFIED);
        return 0;
    default:
        fk_err(r, FWS_REFUSED);
        return 0;
    }
}

static int fk_f9(void *ctx, uint8_t feat, uint8_t count, int use_key, uint64_t key,
                 const uint8_t *data, uint32_t nblocks, uint32_t tmo, fw_reply *r)
{
    fake_t *k = (fake_t *)ctx;
    int idx = k->ncalls++;
    (void)tmo;
    memset(r, 0, sizeof *r);
    if (idx < FK_MAXCALLS) {
        k->feat[idx] = feat; k->cnt[idx] = count; k->nblk[idx] = nblocks;
        k->used_key[idx] = use_key; k->key[idx] = key;
    }
    k->clock_ms += 7;
    if (k->ncalls == k->timeout_at_call || (k->timeout_from_call && k->ncalls >= k->timeout_from_call)) {
        r->rc = FWIO_TIMEOUT; r->status = 0x80;
        return r->rc;
    }
    if (k->mode == FK_RESET) { r->rc = FWIO_TIMEOUT; r->status = 0x80; return r->rc; }
    r->rc = FWIO_OK;
    if (k->err_code && feat == k->err_feat) {
        int nth = fk_calls_of(k, feat);
        if (nth == k->err_nth || (k->err_repeat && nth >= k->err_nth)) {
            r->blocks = 0;
            fk_err(r, k->err_code);
            return r->rc;
        }
    }
    if (k->mode == FK_NORMAL) {
        if (k->write_session) { fk_err(r, FWS_REFUSED); return r->rc; }
        if (feat == FW_FEAT_ARM && use_key && key == FW_MAGIC_ARM) { k->armed = 1; fk_ok(r); return r->rc; }
        if (feat == FW_FEAT_ENTER && use_key && key == FW_MAGIC_ENTER && k->armed) {
            k->mode = FK_UPDATER;
            k->armed = 0;
            fk_ok(r);
            return r->rc;
        }
        k->armed = 0;
        fk_err(r, FWS_REFUSED);
        return r->rc;
    }
    fk_updater(k, feat, count, use_key, key, data, nblocks, r);
    return r->rc;
}

static int fk_identify(void *ctx, uint8_t *buf, uint32_t tmo, fw_reply *r)
{
    fake_t *k = (fake_t *)ctx;
    int w;
    (void)tmo;
    memset(r, 0, sizeof *r);
    r->rc = FWIO_OK;
    if (k->identify_fail) { r->status = 0x51; r->error = 0x04; return r->rc; }
    memset(buf, 0, 512);
    for (w = 0; w < 40; w++) {
        char c = w < (int)strlen(k->model) ? k->model[w] : ' ';
        int word = 27 + w / 2;
        if (w % 2 == 0) buf[2 * word + 1] = (uint8_t)c; else buf[2 * word] = (uint8_t)c;
    }
    r->status = 0x50;
    return r->rc;
}

static int fk_prepare(void *ctx) { ((fake_t *)ctx)->prepared++; return 0; }
static uint32_t fk_now(void *ctx) { return ((fake_t *)ctx)->clock_ms; }
static void fk_sleep(void *ctx, uint32_t ms) { ((fake_t *)ctx)->clock_ms += ms; }
static void fk_progress(void *ctx, const fw_progress *p) { (void)p; ((fake_t *)ctx)->nprogress++; }
static void fk_log(void *ctx, const char *line)
{
    fake_t *k = (fake_t *)ctx;
    size_t n = strlen(k->all_log), l = strlen(line);
    k->log_lines++;
    strncpy(k->last_log, line, sizeof k->last_log - 1);
    if (n + l + 2 < sizeof k->all_log) { memcpy(k->all_log + n, line, l); k->all_log[n + l] = '\n'; k->all_log[n + l + 1] = 0; }
}

static void fk_env(fake_t *k, fw_env *e)
{
    memset(e, 0, sizeof *e);
    e->ctx = k;
    e->f9 = fk_f9;
    e->identify = fk_identify;
    e->prepare = fk_prepare;
    e->now_ms = fk_now;
    e->sleep_ms = fk_sleep;
    e->progress = fk_progress;
    e->log = fk_log;
}

#endif
