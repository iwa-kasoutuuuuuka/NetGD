#include "g2b_core.h"
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include "../netgd-recv/src/crc32.h"

static uint32_t xs_next(uint32_t *s)
{
    uint32_t x = *s;
    x ^= x << 13; x ^= x >> 17; x ^= x << 5;
    return *s = x;
}

void g2b_gen_block(uint32_t k, uint8_t *out)
{
    uint32_t s = 0x2545F491u, m = k * 0x9E3779B1u;
    for (uint32_t j = 0; j < G2B_WORDS; j++) {
        uint32_t w = xs_next(&s) ^ m;
        out[4 * j] = (uint8_t)w; out[4 * j + 1] = (uint8_t)(w >> 8);
        out[4 * j + 2] = (uint8_t)(w >> 16); out[4 * j + 3] = (uint8_t)(w >> 24);
    }
}

uint32_t g2b_expected_crc(uint32_t blocks)
{
    static uint8_t b[G2B_BLOCK];
    uint32_t crc = 0;
    for (uint32_t k = 0; k < blocks; k++) { g2b_gen_block(k, b); crc = ng_crc32(crc, b, sizeof b); }
    return crc;
}

uint32_t g2b_expected_crc_bytes(uint64_t n)
{
    static uint8_t b[G2B_BLOCK];
    uint32_t crc = 0;
    for (uint32_t k = 0; n; k++) {
        size_t take = n < G2B_BLOCK ? (size_t)n : G2B_BLOCK;
        g2b_gen_block(k, b); crc = ng_crc32(crc, b, take); n -= take;
    }
    return crc;
}

uint64_t g2b_kbps(uint64_t bytes, uint64_t us)
{
    if (!us) return 0;
    return bytes * 1000000ull / 1024ull / us;
}

void g2b_kv_init(g2b_kv *kv, char *buf, size_t cap)
{
    kv->buf = buf; kv->cap = cap; kv->len = 0; kv->err = 0;
    if (cap) buf[0] = 0;
}

static void kv_line(g2b_kv *kv, const char *key, const char *val)
{
    int n = snprintf(kv->buf + kv->len, kv->cap - kv->len, "%s=%s\n", key, val);
    if (n < 0 || (size_t)n >= kv->cap - kv->len) { kv->err = 1; kv->buf[kv->len] = 0; return; }
    kv->len += (size_t)n;
}

void g2b_kv_str(g2b_kv *kv, const char *key, const char *val) { kv_line(kv, key, val); }

void g2b_kv_u64(g2b_kv *kv, const char *key, uint64_t v)
{
    char t[24]; snprintf(t, sizeof t, "%lu", (unsigned long)v); kv_line(kv, key, t);
}

void g2b_kv_hex32(g2b_kv *kv, const char *key, uint32_t v)
{
    char t[16]; snprintf(t, sizeof t, "0x%08lx", (unsigned long)v); kv_line(kv, key, t);
}

void g2b_parse(g2b_result *r, const char *text, size_t len)
{
    r->n = 0; r->bad_lines = 0;
    size_t i = 0;
    while (i < len) {
        size_t s = i;
        while (i < len && text[i] != '\n') i++;
        size_t e = i; if (i < len) i++;
        if (e > s && text[e - 1] == '\r') e--;
        if (e == s || text[s] == '#') continue;
        const char *eq = memchr(text + s, '=', e - s);
        size_t kl = eq ? (size_t)(eq - (text + s)) : 0;
        size_t vl = eq ? e - s - kl - 1 : 0;
        if (!eq || kl == 0 || kl >= sizeof r->e[0].key || vl >= sizeof r->e[0].val || r->n >= G2B_MAX_ENTRIES) { r->bad_lines++; continue; }
        memcpy(r->e[r->n].key, text + s, kl); r->e[r->n].key[kl] = 0;
        memcpy(r->e[r->n].val, eq + 1, vl);   r->e[r->n].val[vl] = 0;
        r->n++;
    }
}

const char *g2b_get(const g2b_result *r, const char *key)
{
    for (int i = r->n - 1; i >= 0; i--) if (!strcmp(r->e[i].key, key)) return r->e[i].val;
    return NULL;
}

int g2b_get_u64(const g2b_result *r, const char *key, uint64_t *out)
{
    const char *v = g2b_get(r, key);
    if (!v || !*v) return -1;
    char *end; unsigned long long x = strtoull(v, &end, 0);
    if (*end) return -1;
    *out = x; return 0;
}
