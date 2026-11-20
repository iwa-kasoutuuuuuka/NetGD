#include "record.h"
#include <stdio.h>
#include <string.h>

const char *rec_rc_name(int rc)
{
    switch (rc) {
    case GD_OK: return "ok";
    case GD_CHECK: return "check";
    case GD_TIMEOUT: return "timeout";
    case GD_BUSY: return "busy";
    case GD_PROTO: return "proto";
    case GD_REFUSED: return "refused";
    default: return "unknown";
    }
}

size_t rec_hex(char *out, const uint8_t *in, size_t n)
{
    static const char d[] = "0123456789abcdef";
    size_t i;
    for (i = 0; i < n; i++) {
        out[2 * i] = d[in[i] >> 4];
        out[2 * i + 1] = d[in[i] & 15];
    }
    out[2 * n] = 0;
    return 2 * n;
}

/* snprintf that never reports more than fits. */
static size_t put(char *out, size_t cap, size_t pos, const char *fmt, ...) __attribute__((format(printf, 4, 5)));
#include <stdarg.h>
static size_t put(char *out, size_t cap, size_t pos, const char *fmt, ...)
{
    va_list ap;
    int n;
    if (pos >= cap)
        return pos;
    va_start(ap, fmt);
    n = vsnprintf(out + pos, cap - pos, fmt, ap);
    va_end(ap);
    if (n < 0)
        return pos;
    pos += (size_t)n;
    if (pos >= cap)
        pos = cap - 1;
    return pos;
}

static size_t finish(char *out, size_t cap, size_t pos)
{
    if (cap == 0)
        return 0;
    if (pos + 1 >= cap)
        pos = cap - 2;
    out[pos++] = '\n';
    out[pos] = 0;
    return pos;
}

size_t rec_probe_line(char *out, size_t cap, const rec_probe *p)
{
    size_t pos = 0, n;
    char hex[2 * REC_DATA_MAX + 1];
    if (cap < 8)
        return 0;
    pos = put(out, cap, pos, "img=%d probe=%s", p->img, p->name);
    if (p->extra && p->extra[0])
        pos = put(out, cap, pos, " %s", p->extra);
    if (p->cdb) {
        char c[25];
        rec_hex(c, p->cdb, 12);
        pos = put(out, cap, pos, " cdb=%s", c);
    }
    pos = put(out, cap, pos, " rc=%s status=0x%02x err=0x%02x irr=0x%02x bytes=%lu",
              rec_rc_name(p->res->rc), p->res->status, p->res->error, p->res->irr,
              (unsigned long)p->res->total);
    n = p->res->stored;
    if (!p->data)
        n = 0;
    if (n > REC_DATA_MAX)
        n = REC_DATA_MAX;
    if (n) {
        rec_hex(hex, p->data, n);
        pos = put(out, cap, pos, " data=%s", hex);
    }
    pos = put(out, cap, pos, " us=%lu", (unsigned long)p->res->us);
    return finish(out, cap, pos);
}

size_t rec_hash_line(char *out, size_t cap, int img, uint32_t fad, uint32_t crc, uint32_t ssz)
{
    size_t pos = put(out, cap, 0, "img=%d probe=HASH fad=%lu ssz=%lu crc=%08lx", img,
                     (unsigned long)fad, (unsigned long)ssz, (unsigned long)crc);
    return finish(out, cap, pos);
}

size_t rec_comment_line(char *out, size_t cap, const char *text)
{
    size_t pos = put(out, cap, 0, "# %s", text);
    return finish(out, cap, pos);
}

size_t rec_event_line(char *out, size_t cap, int img, const char *name, const char *fields)
{
    size_t pos = put(out, cap, 0, "img=%d probe=%s", img, name);
    if (fields && fields[0])
        pos = put(out, cap, pos, " %s", fields);
    return finish(out, cap, pos);
}
