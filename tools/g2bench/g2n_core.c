#include "g2n_core.h"
#include <string.h>
#include "../netgd-recv/src/crc32.h"

static void put16(uint8_t *p, uint32_t v) { p[0] = (uint8_t)v; p[1] = (uint8_t)(v >> 8); }
static void put32(uint8_t *p, uint32_t v) { put16(p, v); put16(p + 2, v >> 16); }
static void put64(uint8_t *p, uint64_t v) { put32(p, (uint32_t)v); put32(p + 4, (uint32_t)(v >> 32)); }
static uint32_t get16(const uint8_t *p) { return (uint32_t)p[0] | ((uint32_t)p[1] << 8); }
static uint32_t get32(const uint8_t *p) { return get16(p) | (get16(p + 2) << 16); }
static uint64_t get64(const uint8_t *p) { return get32(p) | ((uint64_t)get32(p + 4) << 32); }

size_t g2n_frame_build(uint8_t *out, uint8_t type, uint8_t flags, uint32_t seq, const void *payload, uint32_t n)
{
    memcpy(out, "NGT1", 4);
    out[4] = type; out[5] = flags; out[6] = 0; out[7] = 0;
    put32(out + 8, seq); put32(out + 12, n);
    if (n) memcpy(out + G2N_HDR, payload, n);
    put32(out + G2N_HDR + n, ng_crc32(0, out, G2N_HDR + n));
    return (size_t)n + G2N_HDR + 4;
}

int g2n_hdr_parse(const uint8_t hdr[G2N_HDR], g2n_hdr *h)
{
    if (memcmp(hdr, "NGT1", 4) != 0 || hdr[6] || hdr[7]) return -1;
    h->type = hdr[4]; h->flags = hdr[5]; h->seq = get32(hdr + 8); h->len = get32(hdr + 12);
    return h->len > G2N_MAX_PAYLOAD ? -1 : 0;
}

uint32_t g2n_frame_crc(const uint8_t hdr[G2N_HDR], const void *payload, uint32_t n)
{
    return ng_crc32(ng_crc32(0, hdr, G2N_HDR), payload, n);
}

void g2n_want_build(uint8_t out[16], uint16_t idx, uint64_t off, uint32_t len)
{
    put16(out, idx); put16(out + 2, 0); put64(out + 4, off); put32(out + 12, len);
}

int g2n_data_parse(const uint8_t *p, uint32_t n, uint16_t *idx, uint64_t *off, uint32_t *dlen, const uint8_t **data)
{
    if (n < 12) return -1;
    *idx = (uint16_t)get16(p); *off = get64(p + 4); *dlen = n - 12; *data = p + 12;
    return 0;
}

void g2n_pass_init(g2n_pass *p, uint16_t idx, uint64_t total, uint32_t want, uint32_t depth)
{
    memset(p, 0, sizeof *p);
    p->idx = idx; p->total = total; p->want = want ? want : 1; p->depth = depth ? depth : 1;
}

static uint32_t piece(const g2n_pass *p, uint64_t off)
{
    uint64_t left = p->total - off;
    return left < p->want ? (uint32_t)left : p->want;
}

int g2n_pass_next_want(g2n_pass *p, uint64_t *off, uint32_t *len)
{
    if (p->stopped || p->outstanding >= p->depth || p->issue_off >= p->total) return 0;
    *off = p->issue_off; *len = piece(p, p->issue_off);
    p->issue_off += *len; p->outstanding++;
    return 1;
}

void g2n_pass_stop(g2n_pass *p) { p->stopped = 1; }

int g2n_pass_on_data(g2n_pass *p, uint16_t idx, uint64_t off, uint32_t len)
{
    if (!p->outstanding) return G2N_E_STATE;
    if (idx != p->idx) return G2N_E_FILE;
    if (off != p->recv_off) return G2N_E_OFFSET;
    if (len != piece(p, off)) return G2N_E_LENGTH;
    p->recv_off += len; p->outstanding--;
    return 0;
}

int g2n_pass_done(const g2n_pass *p)
{
    return p->outstanding == 0 && (p->stopped || p->issue_off >= p->total);
}

void g2n_udp_build(uint8_t *out, uint32_t size, uint8_t type, uint16_t pass, uint32_t seq)
{
    memcpy(out, "G2U1", 4); out[4] = type; out[5] = 0; put16(out + 6, pass); put32(out + 8, seq);
    for (uint32_t i = G2N_U_HDR; i < size; i++) out[i] = (uint8_t)(0xA5 ^ i);
}

int g2n_udp_parse(const uint8_t *p, uint32_t n, uint8_t *type, uint16_t *pass, uint32_t *seq)
{
    if (n < G2N_U_HDR || memcmp(p, "G2U1", 4) != 0) return -1;
    *type = p[4]; *pass = (uint16_t)get16(p + 6); *seq = get32(p + 8);
    return 0;
}

void g2n_udp_stat_init(g2n_udp_stat *s) { memset(s, 0, sizeof *s); }

void g2n_udp_stat_pkt(g2n_udp_stat *s, uint32_t seq, uint32_t bytes)
{
    s->got++; s->bytes += bytes;
    if (seq >= s->next) { s->lost += seq - s->next; s->next = seq + 1; }
    else { s->late++; if (s->lost) s->lost--; }
}

uint32_t g2n_udp_stat_missing(const g2n_udp_stat *s) { return s->next - s->got; }
