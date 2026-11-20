/* netgd-fwupd: pure core. See fwcore.h. */
#include <ctype.h>
#include <stdarg.h>
#include <stdio.h>
#include <string.h>
#include "fwcore.h"

/* ================================================================== CRC-32 ==== */

static uint32_t g_crc_tab[256];
static int g_crc_ready;

uint32_t fw_crc32_update(uint32_t crc, const void *data, size_t n)
{
    const uint8_t *p = (const uint8_t *)data;
    if (!g_crc_ready) {
        uint32_t i, k;
        for (i = 0; i < 256; i++) {
            uint32_t c = i;
            for (k = 0; k < 8; k++)
                c = (c & 1) ? (c >> 1) ^ 0xEDB88320u : c >> 1;
            g_crc_tab[i] = c;
        }
        g_crc_ready = 1;
    }
    crc = ~crc;
    while (n--)
        crc = g_crc_tab[(crc ^ *p++) & 0xFF] ^ (crc >> 8);
    return ~crc;
}

/* ============================================================ image files ==== */

static uint32_t le32(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

int fw_part_from_cidr(uint32_t cidr)
{
    uint32_t c = cidr & ~FW_CIDR_VERSION_MASK;
    if (c == (FW_CIDR_2E & ~FW_CIDR_VERSION_MASK)) return FW_PART_2E;
    if (c == (FW_CIDR_4E & ~FW_CIDR_VERSION_MASK)) return FW_PART_4E;
    return FW_PART_NONE;
}

const char *fw_part_name(int part)
{
    return part == FW_PART_2E ? "SAM3U2E" : part == FW_PART_4E ? "SAM3U4E" : "unknown";
}

uint32_t fw_part_sram0_end(int part)
{
    return 0x20000000u + (part == FW_PART_2E ? 0x4000u : 0x8000u);
}

/* logic.rs check_vectors */
int fw_check_vectors(const uint32_t v[4], uint32_t dest, uint32_t len, int part)
{
    uint32_t sp = v[0], end = dest + len;
    int i;
    if (end < dest) end = 0xFFFFFFFFu;               /* saturating_add */
    if (sp % 4 != 0 || sp <= 0x20000000u || sp > fw_part_sram0_end(part)) return -1;
    for (i = 1; i < 4; i++) {
        uint32_t h = v[i], a = h & ~1u;
        if (!(h & 1u) || a < dest || a >= end) return -1;
    }
    return 0;
}

/* Intrinsic checks of one header block (logic.rs parse_header_v2 / parse_header_v3): magic, format,
 * CRC, kind. `formats` is the FW_ACCEPT_* mask this reader takes. Format 3: the header CRC covers
 * bytes 0..31 and then the layout word (bytes 36..39). */
fw_verr fw_header_parse_ex(const uint8_t *b, fw_header *h, unsigned formats)
{
    uint32_t w[FW_HDR_WORDS_V3], crc;
    int k;
    for (k = 0; k < (int)FW_HDR_WORDS_V3; k++) w[k] = le32(b + 4 * k);
    if (w[0] != FW_MAGIC) return FWV_MAGIC;
    if (!((w[1] == FW_FORMAT && (formats & FW_ACCEPT_2)) || (w[1] == FW_FORMAT_3 && (formats & FW_ACCEPT_3))))
        return FWV_FORMAT;
    crc = fw_crc32(b, 32);
    if (w[1] == FW_FORMAT_3) crc = fw_crc32_update(crc, b + 36, 4);
    if (crc != w[8]) return FWV_HDR_CRC;
    if (w[2] != FW_KIND_CODE && w[2] != FW_KIND_BITSTREAM) return FWV_KIND;
    h->kind = w[2]; h->target_cidr = w[3]; h->dest = w[4];
    h->len = w[5]; h->crc = w[6]; h->version = w[7];
    h->format = w[1];
    h->layout = w[1] == FW_FORMAT_3 ? w[9] : 0;
    return FWV_OK;
}

fw_verr fw_header_parse(const uint8_t *b, fw_header *h)
{
    return fw_header_parse_ex(b, h, FW_ACCEPT_ANY);
}

/* Is this image for `part` and aimed where it may go (logic.rs check_header)? Format 3: built for
 * this layout, code at exactly APP_BASE, at most PLANE_LEN - LAYOUT long. */
fw_verr fw_header_check(const fw_header *h, int part)
{
    int v3 = h->format == FW_FORMAT_3;
    uint32_t limit = v3 && h->kind == FW_KIND_CODE ? FW_PLANE_LEN - FW_LAYOUT : FW_PLANE_LEN;
    if (part == FW_PART_NONE || fw_part_from_cidr(h->target_cidr) != part) return FWV_TARGET;
    if (v3 && h->layout != FW_LAYOUT) return FWV_DEST;
    if (h->kind == FW_KIND_CODE ? h->dest != (v3 ? FW_APP_BASE : FW_PLANE0_BASE)
                                : !(h->dest == FW_PLANE1_BASE && part == FW_PART_4E))
        return FWV_DEST;
    if (h->len == 0 || h->len % FW_BLOCK != 0 || h->len > limit) return FWV_LEN;
    return FWV_OK;
}

/* logic.rs check_descriptor: magic, version, length 0 -> unsealed, CRC over the first 28 bytes,
 * length == the image's and a multiple of 512. */
fw_verr fw_check_descriptor(const uint8_t *d, uint32_t image_len)
{
    uint32_t length;
    if (memcmp(d, "NGAP", 4) != 0 || le32(d + 4) != FW_DESC_VERSION) return FWV_DESC;
    length = le32(d + 8);
    if (length == 0) return FWV_UNSEALED;
    if (fw_crc32(d, 28) != le32(d + 28)) return FWV_DESC;
    if (length != image_len || length % FW_BLOCK != 0) return FWV_DESC;
    return FWV_OK;
}

/* logic.rs check_trailer: the last 4 bytes (LE) are the CRC-32 of everything before them. */
fw_verr fw_check_trailer(const uint8_t *image, size_t n)
{
    if (n < FW_TRAILER_LEN) return FWV_TRAILER;
    return fw_crc32(image, n - FW_TRAILER_LEN) == le32(image + n - FW_TRAILER_LEN) ? FWV_OK : FWV_TRAILER;
}

static int fail(fw_vres *res, fw_verr e, int index)
{
    res->err = e;
    res->index = index;
    return -1;
}

int fw_parse_file(const uint8_t *d, size_t n, fw_file *f, fw_vres *res)
{
    return fw_parse_file_ex(d, n, f, res, FW_ACCEPT_ANY);
}

int fw_parse_file_ex(const uint8_t *d, size_t n, fw_file *f, fw_vres *res, unsigned formats)
{
    size_t off = 0;
    fw_vres dummy;
    if (!res) res = &dummy;
    memset(f, 0, sizeof(*f));
    res->err = FWV_OK;
    res->index = 0;
    if (n == 0) return fail(res, FWV_EMPTY, 0);
    if (n > FW_MAX_FILE) return fail(res, FWV_TOO_BIG, 0);

    while (off < n) {
        fw_image *im;
        fw_header h;
        const uint8_t *b = d + off;
        int k, part, idx = f->n;
        fw_verr e;

        if (idx >= FW_MAX_IMAGES) return fail(res, FWV_DUP, idx);
        if (n - off < FW_BLOCK) return fail(res, FWV_SHORT, idx);
        e = fw_header_parse_ex(b, &h, formats);
        if (e != FWV_OK) return fail(res, e, idx);
        part = fw_part_from_cidr(h.target_cidr);
        e = fw_header_check(&h, part);
        if (e != FWV_OK) return fail(res, e, idx);

        off += FW_BLOCK;
        if (n - off < h.len) return fail(res, FWV_TRUNC, idx);
        if (fw_crc32(d + off, h.len) != h.crc) return fail(res, FWV_PAYLOAD_CRC, idx);
        if (h.kind == FW_KIND_CODE) {
            uint32_t v[4];
            for (k = 0; k < 4; k++) v[k] = le32(d + off + 4 * k);
            if (fw_check_vectors(v, h.dest, h.len, part) != 0) return fail(res, FWV_VECTOR, idx);
            if (h.format == FW_FORMAT_3) {
                e = fw_check_descriptor(d + off + FW_DESC_OFFSET, h.len);
                if (e == FWV_OK) e = fw_check_trailer(d + off, h.len);
                if (e != FWV_OK) return fail(res, e, idx);
            }
        }
        if (f->n > 0 && f->img[0].h.format != h.format) return fail(res, FWV_MIXFMT, idx);
        for (k = 0; k < f->n; k++)
            if (f->img[k].h.kind == h.kind) return fail(res, FWV_DUP, idx);
        if (f->n > 0 && f->part != part) return fail(res, FWV_MIXED, idx);

        im = &f->img[f->n++];
        im->h = h;
        im->header_block = b;
        im->payload = d + off;
        f->part = part;
        off += h.len;
    }
    return 0;
}

int fw_read_all(const fw_rd *io, const char *path, uint8_t *buf, size_t cap, size_t *len)
{
    void *f = io->open(path);
    size_t n = 0;
    int rc = 0;
    *len = 0;
    if (!f) return -1;
    while (n < cap) {
        size_t want = cap - n, got;
        if (want > FW_READ_CHUNK) want = FW_READ_CHUNK;
        got = io->read(f, buf + n, want);
        n += got;
        if (got < want) break;
    }
    if (io->error(f)) rc = -2;
    io->close(f);
    *len = n;
    return rc;
}

/* ====================================================== status / messages ==== */

int fw_status_msgid(uint8_t e)
{
    switch (e) {
    case FWS_OK: return FWM_ST_OK;
    case FWS_REFUSED: return FWM_ST_REFUSED;
    case FWS_BADHEADER: return FWM_ST_BADHEADER;
    case FWS_BADDEST: return FWM_ST_BADDEST;
    case FWS_BADLENGTH: return FWM_ST_BADLENGTH;
    case FWS_NOHEADER: return FWM_ST_NOHEADER;
    case FWS_OVERRUN: return FWM_ST_OVERRUN;
    case FWS_FLASHERROR: return FWM_ST_FLASHERROR;
    case FWS_CRCMISMATCH: return FWM_ST_CRCMISMATCH;
    case FWS_BADTARGET: return FWM_ST_BADTARGET;
    case FWS_LOCKED: return FWM_ST_LOCKED;
    case FWS_BADVECTOR: return FWM_ST_BADVECTOR;
    case FWS_NOTVERIFIED: return FWM_ST_NOTVERIFIED;
    default: return FWM_ST_UNKNOWN;
    }
}

const char *fw_status_name(uint8_t e)
{
    switch (e) {
    case FWS_OK: return "Ok";
    case FWS_REFUSED: return "Refused";
    case FWS_BADHEADER: return "BadHeader";
    case FWS_BADDEST: return "BadDest";
    case FWS_BADLENGTH: return "BadLength";
    case FWS_NOHEADER: return "NoHeader";
    case FWS_OVERRUN: return "Overrun";
    case FWS_FLASHERROR: return "FlashError";
    case FWS_CRCMISMATCH: return "CrcMismatch";
    case FWS_BADTARGET: return "BadTarget";
    case FWS_LOCKED: return "Locked";
    case FWS_BADVECTOR: return "BadVector";
    case FWS_NOTVERIFIED: return "NotVerified";
    default: return "unknown";
    }
}

int fw_verr_msgid(fw_verr e)
{
    switch (e) {
    case FWV_EMPTY: return FWM_V_EMPTY;
    case FWV_TOO_BIG: return FWM_V_TOO_BIG;
    case FWV_SHORT: return FWM_V_SHORT;
    case FWV_MAGIC: return FWM_V_MAGIC;
    case FWV_FORMAT: return FWM_V_FORMAT;
    case FWV_HDR_CRC: return FWM_V_HDR_CRC;
    case FWV_KIND: return FWM_V_KIND;
    case FWV_TARGET: return FWM_V_TARGET;
    case FWV_DEST: return FWM_V_DEST;
    case FWV_LEN: return FWM_V_LEN;
    case FWV_TRUNC: return FWM_V_TRUNC;
    case FWV_PAYLOAD_CRC: return FWM_V_PAYLOAD_CRC;
    case FWV_VECTOR: return FWM_V_VECTOR;
    case FWV_MIXED: return FWM_V_MIXED;
    case FWV_DUP: return FWM_V_DUP;
    case FWV_DESC: return FWM_V_DESC;
    case FWV_UNSEALED: return FWM_V_UNSEALED;
    case FWV_TRAILER: return FWM_V_TRAILER;
    case FWV_MIXFMT: return FWM_V_MIXFMT;
    default: return FWM_V_READ;
    }
}

int fw_io_msgid(int rc)
{
    return rc == FWIO_BUSY ? FWM_IO_BUSY : rc == FWIO_PROTO ? FWM_IO_PROTO : FWM_IO_TIMEOUT;
}

int fw_outcome_msgid(const fw_report *r)
{
    switch (r->outcome) {
    case FWR_OK: return FWM_OUT_OK;
    case FWR_OK_UNCONFIRMED: return FWM_OUT_OK_UNCONFIRMED;
    case FWR_NO_RESET: return FWM_OUT_NO_RESET;
    case FWR_NOT_NETGD: return FWM_OUT_NOT_NETGD;
    case FWR_IDENT_FAIL: return FWM_OUT_IDENT_FAIL;
    case FWR_REFUSED_START: return FWM_OUT_REFUSED_START;
    case FWR_ENTER_FAILED: return FWM_OUT_ENTER_FAILED;
    case FWR_REJECTED: return r->flash_touched ? FWM_OUT_REJECTED_DIRTY : FWM_OUT_REJECTED_CLEAN;
    case FWR_FAILED: return r->flash_touched ? FWM_OUT_FAILED_DIRTY : FWM_OUT_REJECTED_CLEAN;
    case FWR_IO: return r->flash_touched ? FWM_OUT_IO_DIRTY : FWM_OUT_IO_CLEAN;
    case FWR_CANCELLED: return FWM_OUT_CANCELLED;
    }
    return FWM_OUT_CANCELLED;
}

/* ================================================================ identify ==== */

int fw_model_is_netgd(const char *model)
{
    char up[64];
    size_t i;
    for (i = 0; model[i] && i + 1 < sizeof up; i++) up[i] = (char)toupper((unsigned char)model[i]);
    up[i] = 0;
    return strstr(up, "OPENGDEMU") != NULL || strstr(up, "NETGD") != NULL;
}

int fw_check_device(const fw_env *env, char *model, size_t cap)
{
    uint8_t b[512];
    fw_reply r;
    char m[41];
    int w, n = 0;
    if (cap) model[0] = 0;
    if (!env->identify) return 0;                       /* not checkable: caller decides */
    memset(b, 0, sizeof b);
    memset(&r, 0, sizeof r);
    if (env->identify(env->ctx, b, 3000, &r) != FWIO_OK || (r.status & FW_ST_ERR)) return -1;
    /* words 27..46 = model, two characters per word, first character in the high byte */
    for (w = 27; w <= 46; w++) {
        m[n++] = (char)b[2 * w + 1];
        m[n++] = (char)b[2 * w];
    }
    m[n] = 0;
    for (w = 0; w < n; w++)
        if ((unsigned char)m[w] < 0x20 || (unsigned char)m[w] > 0x7E) m[w] = '?';
    while (n > 0 && m[n - 1] == ' ') m[--n] = 0;
    if (cap) { strncpy(model, m, cap - 1); model[cap - 1] = 0; }
    return fw_model_is_netgd(m) ? 0 : 1;
}

/* ================================================================== update ==== */

void fw_options_default(fw_options *o)
{
    o->chunk_blocks = 64;
    o->max_retries = 2;
    o->t_cmd_ms = 5000;
    o->t_data_ms = 10000;
    o->t_verify_ms = 60000;
}

typedef struct {
    const fw_env *env;
    const fw_options *o;
    fw_report *rep;
    int nimages;
} run_t;

static void logf_(const fw_env *e, const char *fmt, ...)
{
    char line[400];
    va_list ap;
    if (!e->log) return;
    va_start(ap, fmt);
    vsnprintf(line, sizeof line, fmt, ap);
    va_end(ap);
    e->log(e->ctx, line);
}

enum { SEND_OK = 0, SEND_DEV = 1, SEND_IO = 2 };

static const char *step_name(int msgid)
{
    switch (msgid) {
    case FWM_STEP_ARM: return "arm";
    case FWM_STEP_ENTER: return "enter";
    case FWM_STEP_PROBE: return "probe";
    case FWM_STEP_HEADER: return "header";
    case FWM_STEP_DATA: return "data";
    case FWM_STEP_VERIFY: return "verify";
    case FWM_STEP_RESET: return "reset";
    default: return "?";
    }
}

/* One F9 command, logged. Records the failing step in the report. */
static int f9_send(run_t *r, int msgid, int image, int attempt, uint8_t feat, uint8_t count,
                int use_key, uint64_t key, const uint8_t *data, uint32_t nblocks,
                uint32_t tmo, fw_reply *rp)
{
    const fw_env *e = r->env;
    int rc;
    memset(rp, 0, sizeof *rp);
    rc = e->f9(e->ctx, feat, count, use_key, key, data, nblocks, tmo, rp);
    rp->rc = rc;
    if (rc != FWIO_OK) {
        logf_(e, "t=%lu step=%s image=%d attempt=%d feat=0x%02x blocks=%lu io=%s status=0x%02x error=0x%02x ja=%s",
              (unsigned long)(e->now_ms ? e->now_ms(e->ctx) : 0), step_name(msgid), image, attempt,
              feat, (unsigned long)rp->blocks,
              rc == FWIO_TIMEOUT ? "timeout" : rc == FWIO_BUSY ? "busy" : "proto", rp->status, rp->error,
              fw_msg_ja(fw_io_msgid(rc)));
        r->rep->fail_msgid = msgid;
        r->rep->io_rc = rc;
        r->rep->dev_status = rp->status;
        r->rep->dev_error = rp->error;
        r->rep->failed_image = image;
        return SEND_IO;
    }
    if (rp->status & FW_ST_ERR) {
        logf_(e, "t=%lu step=%s image=%d attempt=%d feat=0x%02x blocks=%lu status=0x%02x error=0x%02x name=%s ja=%s",
              (unsigned long)(e->now_ms ? e->now_ms(e->ctx) : 0), step_name(msgid), image, attempt, feat,
              (unsigned long)rp->blocks, rp->status, rp->error, fw_status_name(rp->error),
              fw_msg_ja(fw_status_msgid(rp->error)));
        r->rep->fail_msgid = msgid;
        r->rep->io_rc = FWIO_OK;
        r->rep->dev_status = rp->status;
        r->rep->dev_error = rp->error;
        r->rep->failed_image = image;
        return SEND_DEV;
    }
    logf_(e, "t=%lu step=%s image=%d attempt=%d feat=0x%02x blocks=%lu status=0x%02x error=0x%02x ok",
          (unsigned long)(e->now_ms ? e->now_ms(e->ctx) : 0), step_name(msgid), image, attempt, feat,
          (unsigned long)rp->blocks, rp->status, rp->error);
    return SEND_OK;
}

static int is_updater_code(uint8_t e)
{
    return (e & 0x0F) == 0x04 && (e >> 4) != 0;
}

static int retryable(uint8_t e)
{
    return e == FWS_OVERRUN || e == FWS_FLASHERROR || e == FWS_CRCMISMATCH ||
           e == FWS_NOHEADER || e == FWS_BADLENGTH;
}

static void progress(run_t *r, int image, uint32_t done, uint32_t total, int msgid, int attempt)
{
    fw_progress p;
    if (!r->env->progress) return;
    p.image = image;
    p.nimages = r->nimages;
    p.blocks_done = done;
    p.blocks_total = total;
    p.msgid = msgid;
    p.attempt = attempt;
    r->env->progress(r->env->ctx, &p);
}

enum { IMG_OK = 0, IMG_RETRY = 1, IMG_REJECTED = 2, IMG_FAILED = 3, IMG_IO = 4 };

/* Header, data, verify for one image: FORMAT.md "Sending" steps 1-3. */
static int send_image(run_t *r, int idx, const fw_image *im, int attempt)
{
    const fw_options *o = r->o;
    fw_report *rep = r->rep;
    uint32_t total = im->h.len / FW_BLOCK, done = 0;
    fw_reply rp;
    int rc;

    progress(r, idx, 0, total, FWM_STEP_HEADER, attempt);
    rc = f9_send(r, FWM_STEP_HEADER, idx, attempt, FW_FEAT_HEADER, 0, 0, 0, im->header_block, 1,
              o->t_cmd_ms, &rp);
    if (rc == SEND_IO) return IMG_IO;
    if (rc == SEND_DEV) {
        /* Everything the header step refuses is refused before a page is touched. Only
         * Overrun (we were too slow) is worth another try. */
        return rp.error == FWS_OVERRUN ? IMG_RETRY : IMG_REJECTED;
    }

    while (done < total) {
        uint32_t n = total - done, was_touched = (uint32_t)rep->flash_touched;
        if (n > o->chunk_blocks) n = o->chunk_blocks;
        progress(r, idx, done, total, FWM_STEP_DATA, attempt);
        /* From the first block on, pages 1.. of the image are being rewritten. */
        rep->flash_touched = 1;
        rc = f9_send(r, FWM_STEP_DATA, idx, attempt, FW_FEAT_DATA, (uint8_t)(n == 256 ? 0 : n), 0, 0,
                  im->payload + (size_t)done * FW_BLOCK, n, o->t_data_ms, &rp);
        if (rc == SEND_IO) return IMG_IO;
        if (rc == SEND_DEV) {
            if (rp.error == FWS_BADVECTOR) {          /* raised on block 0, before any write */
                rep->flash_touched = (int)was_touched;
                return IMG_REJECTED;
            }
            return retryable(rp.error) ? IMG_RETRY : IMG_FAILED;
        }
        done += n;
    }
    progress(r, idx, total, total, FWM_STEP_VERIFY, attempt);
    rc = f9_send(r, FWM_STEP_VERIFY, idx, attempt, FW_FEAT_VERIFY, 0, 0, 0, NULL, 0, o->t_verify_ms, &rp);
    if (rc == SEND_IO) return IMG_IO;
    if (rc == SEND_DEV) return retryable(rp.error) ? IMG_RETRY : IMG_FAILED;
    progress(r, idx, total, total, FWM_STEP_VERIFY, attempt);
    return IMG_OK;
}

int fw_update(const fw_env *env, const fw_file *f, const fw_options *o_in, fw_report *rep)
{
    fw_options o;
    run_t r;
    fw_reply rp;
    int rc, i;

    memset(rep, 0, sizeof *rep);
    rep->failed_image = -1;
    if (o_in) o = *o_in; else fw_options_default(&o);
    if (o.chunk_blocks < 1) o.chunk_blocks = 1;
    if (o.chunk_blocks > 256) o.chunk_blocks = 256;
    r.env = env; r.o = &o; r.rep = rep; r.nimages = f->n;

    /* PIO timing of the secondary (what KOS's g1_ata_scan sets). Non-fatal. */
    if (env->prepare && env->prepare(env->ctx) != 0)
        logf_(env, "prepare=failed (SET FEATURES refused; PIO timing left as the BIOS had it)");

    /* ---- handover: fwupdate.rs "Getting in" ---------------------------------------- */
    rc = f9_send(&r, FWM_STEP_ARM, -1, 0, FW_FEAT_ARM, 0, 1, FW_MAGIC_ARM, NULL, 0, o.t_cmd_ms, &rp);
    if (rc == SEND_IO) { rep->outcome = FWR_IO; return -1; }
    if (rc == SEND_DEV) {
        /* Refused. The normal firmware answers a plain ABRT when a write session is active
         * (ide.rs handle_fw_command) or when the key is wrong. But an updater that is
         * already running (an earlier run of this app failed half-way, and the board must
         * NOT be power-cycled then) answers an unknown step the same way. F9/57 tells them
         * apart: the normal firmware refuses it with plain 0x04, the updater answers with
         * one of its own codes (or success). */
        rc = f9_send(&r, FWM_STEP_PROBE, -1, 0, FW_FEAT_VERIFY, 0, 0, 0, NULL, 0, o.t_verify_ms, &rp);
        if (rc == SEND_IO) { rep->outcome = FWR_IO; return -1; }
        if (rc == SEND_OK || (rc == SEND_DEV && is_updater_code(rp.error))) {
            rep->resumed = 1;
            rep->handed_over = 1;
            rep->flash_touched = 1;      /* unknown what the earlier run did: be conservative */
            logf_(env, "resumed=1 the updater was already running");
        } else {
            rep->outcome = FWR_REFUSED_START;
            return -1;
        }
    } else {
        rc = f9_send(&r, FWM_STEP_ENTER, -1, 0, FW_FEAT_ENTER, 0, 1, FW_MAGIC_ENTER, NULL, 0, o.t_cmd_ms, &rp);
        if (rc == SEND_IO) {
            rep->handed_over = 1;        /* may well have started */
            rep->outcome = FWR_IO;
            return -1;
        }
        if (rc == SEND_DEV) { rep->outcome = FWR_ENTER_FAILED; return -1; }
        rep->handed_over = 1;
    }

    /* ---- every image, in file order -------------------------------------------------- */
    for (i = 0; i < f->n; i++) {
        int attempt = 0;
        for (;;) {
            int st = send_image(&r, i, &f->img[i], attempt);
            if (st == IMG_OK) break;
            if (st == IMG_RETRY && attempt < o.max_retries) {
                attempt++;
                rep->retries_used++;
                logf_(env, "retry image=%d attempt=%d after status=0x%02x error=0x%02x name=%s", i, attempt,
                      rep->dev_status, rep->dev_error, fw_status_name(rep->dev_error));
                if (env->sleep_ms) env->sleep_ms(env->ctx, 200);
                continue;
            }
            rep->outcome = st == IMG_IO ? FWR_IO : st == IMG_REJECTED ? FWR_REJECTED : FWR_FAILED;
            return -1;
        }
        rep->images_verified++;
        rep->failed_image = -1;
        rep->fail_msgid = 0;
    }

    /* ---- reset: only now, only the plain F9/58 (refused by the updater unless verified) ---- */
    rc = f9_send(&r, FWM_STEP_RESET, -1, 0, FW_FEAT_RESET, 0, 0, 0, NULL, 0, o.t_cmd_ms, &rp);
    if (rc == SEND_OK) { rep->outcome = FWR_OK; return 0; }
    if (rc == SEND_IO) {
        /* The board resets right after it answers; a bus that goes quiet then is expected. */
        rep->outcome = FWR_OK_UNCONFIRMED;
        return 0;
    }
    rep->outcome = FWR_NO_RESET;
    return -1;
}

/* ============================================================= result text ==== */

static void app(char *buf, size_t cap, size_t *len, const char *fmt, ...)
{
    va_list ap;
    int n;
    if (*len + 1 >= cap) return;
    va_start(ap, fmt);
    n = vsnprintf(buf + *len, cap - *len, fmt, ap);
    va_end(ap);
    if (n < 0) return;
    if ((size_t)n >= cap - *len) *len = cap - 1; else *len += (size_t)n;
}

size_t fw_describe_text(char *buf, size_t cap, const char *image_path, size_t file_size,
                        const fw_file *f, const char *model)
{
    size_t len = 0;
    int i;
    if (!cap) return 0;
    buf[0] = 0;
    app(buf, cap, &len, "# netgd-fwupd result\n");
    app(buf, cap, &len, "file=%s size=%lu images=%d target=%s\n", image_path, (unsigned long)file_size,
        f ? f->n : 0, f ? fw_part_name(f->part) : "unknown");
    if (model && model[0]) app(buf, cap, &len, "device_model=%s\n", model);
    for (i = 0; f && i < f->n; i++) {
        const fw_header *h = &f->img[i].h;
        app(buf, cap, &len, "image%d kind=%s dest=0x%08lx len=%lu crc=0x%08lx cidr=0x%08lx version=%lu format=%lu\n", i,
            h->kind == FW_KIND_CODE ? "code" : "bitstream", (unsigned long)h->dest, (unsigned long)h->len,
            (unsigned long)h->crc, (unsigned long)h->target_cidr, (unsigned long)h->version,
            (unsigned long)h->format);
    }
    return len;
}

size_t fw_summary_text(char *buf, size_t cap, const fw_report *rep)
{
    size_t len = 0;
    static const char *const names[] = { "ok", "ok_unconfirmed", "no_reset", "not_netgd", "ident_fail",
                                         "refused_start", "enter_failed", "rejected", "failed", "io", "cancelled" };
    if (!cap) return 0;
    buf[0] = 0;
    app(buf, cap, &len, "outcome=%s handed_over=%d resumed=%d flash_touched=%d images_verified=%d retries=%d",
        names[rep->outcome], rep->handed_over, rep->resumed, rep->flash_touched, rep->images_verified,
        rep->retries_used);
    if (rep->failed_image >= 0)
        app(buf, cap, &len, " failed_image=%d status=0x%02x error=0x%02x name=%s", rep->failed_image, rep->dev_status,
            rep->dev_error, fw_status_name(rep->dev_error));
    app(buf, cap, &len, "\n");
    app(buf, cap, &len, "message_en=%s\n", fw_msg_en(fw_outcome_msgid(rep)));
    app(buf, cap, &len, "message_ja=%s\n", fw_msg_ja(fw_outcome_msgid(rep)));
    return len;
}

/* =============================================================== config ==== */

int fw_cfg_image_path(const char *text, char *out, size_t cap)
{
    const char *p = text;
    while (*p) {
        const char *e = p;
        size_t n;
        while (*e && *e != '\n') e++;
        while (p < e && (*p == ' ' || *p == '\t')) p++;
        if (p < e && *p != '#' && (size_t)(e - p) > 6 && !strncmp(p, "image=", 6)) {
            const char *v = p + 6, *t = e;
            while (t > v && (t[-1] == '\r' || t[-1] == ' ' || t[-1] == '\t')) t--;
            n = (size_t)(t - v);
            if (n > 0 && n < cap) {
                memcpy(out, v, n);
                out[n] = 0;
                return 1;
            }
        }
        p = *e ? e + 1 : e;
    }
    return 0;
}
