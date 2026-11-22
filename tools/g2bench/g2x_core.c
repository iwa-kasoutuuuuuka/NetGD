#include "g2x_core.h"
#include <string.h>
#include "../netgd-recv/src/crc32.h"

static void put16(uint8_t *p, uint32_t v) { p[0] = (uint8_t)v; p[1] = (uint8_t)(v >> 8); }
static void put32(uint8_t *p, uint32_t v) { put16(p, v); put16(p + 2, v >> 16); }
static uint32_t get16(const uint8_t *p) { return (uint32_t)p[0] | ((uint32_t)p[1] << 8); }
static uint32_t get32(const uint8_t *p) { return get16(p) | (get16(p + 2) << 16); }

size_t g2x_data_build(uint8_t *out, uint16_t pass, uint32_t seq, uint8_t flags, const uint8_t *data, uint16_t len, int with_crc)
{
    memcpy(out, "G2X1", 4);
    out[4] = G2X_T_DATA;
    out[5] = (uint8_t)((flags & ~G2X_F_CRC) | (with_crc ? G2X_F_CRC : 0));
    put16(out + 6, pass); put32(out + 8, seq); put16(out + 12, len); put16(out + 14, 0); put32(out + 16, 0);
    if (len) memcpy(out + G2X_HDR, data, len);
    if (with_crc) put32(out + 16, ng_crc32(ng_crc32(0, out, 16), out + G2X_HDR, len));
    return G2X_HDR + (size_t)len;
}

int g2x_data_parse(const uint8_t *p, size_t n, g2x_data_hdr *h, const uint8_t **data)
{
    if (n < G2X_HDR || memcmp(p, "G2X1", 4) != 0 || p[4] != G2X_T_DATA) return -1;
    h->type = p[4]; h->flags = p[5]; h->pass = (uint16_t)get16(p + 6); h->seq = get32(p + 8);
    h->len = (uint16_t)get16(p + 12); h->crc = get32(p + 16);
    if (get16(p + 14) != 0 || h->len > G2X_PAYLOAD || (size_t)h->len + G2X_HDR != n) return -1;
    if ((h->flags & G2X_F_CRC) && ng_crc32(ng_crc32(0, p, 16), p + G2X_HDR, h->len) != h->crc) return -2;
    *data = p + G2X_HDR;
    return 0;
}

void g2x_ack_build(uint8_t out[G2X_ACK_LEN], const g2x_ack *a)
{
    memcpy(out, "G2XA", 4);
    out[4] = G2X_T_ACK; out[5] = a->flags; put16(out + 6, a->pass);
    put32(out + 8, a->cum); put32(out + 12, a->limit); put32(out + 16, a->sack_lo); put32(out + 20, a->sack_hi);
    put32(out + 24, a->ack_no); put32(out + 28, a->dgrams); put32(out + 32, a->dups); put32(out + 36, a->bad);
}

int g2x_ack_parse(const uint8_t *p, size_t n, g2x_ack *a)
{
    if (n < G2X_ACK_LEN || memcmp(p, "G2XA", 4) != 0 || p[4] != G2X_T_ACK) return -1;
    a->flags = p[5]; a->pass = (uint16_t)get16(p + 6);
    a->cum = get32(p + 8); a->limit = get32(p + 12); a->sack_lo = get32(p + 16); a->sack_hi = get32(p + 20);
    a->ack_no = get32(p + 24); a->dgrams = get32(p + 28); a->dups = get32(p + 32); a->bad = get32(p + 36);
    return 0;
}

uint32_t g2x_count(uint64_t total) { return (uint32_t)((total + G2X_PAYLOAD - 1) / G2X_PAYLOAD); }

uint16_t g2x_len(uint64_t total, uint32_t seq)
{
    uint64_t off = (uint64_t)seq * G2X_PAYLOAD;
    if (off >= total) return 0;
    return (uint16_t)(total - off < G2X_PAYLOAD ? total - off : G2X_PAYLOAD);
}

/* ---------------- receiver ---------------- */
void g2x_rx_init(g2x_rx *r, uint16_t pass, uint64_t total, uint8_t (*slots)[G2X_PAYLOAD], g2x_deliver_fn fn, void *ctx)
{
    memset(r, 0, sizeof *r);
    r->pass = pass; r->total = total; r->count = g2x_count(total);
    r->slot = slots; r->deliver = fn; r->ctx = ctx;
}

int g2x_rx_complete(const g2x_rx *r) { return r->cum >= r->count; }

static int ack_due(const g2x_rx *r, uint64_t now)
{
    if (g2x_rx_complete(r)) return 1;
    if (r->since_ack >= G2X_ACK_EVERY) return 1;
    return r->gap_pending && now - r->last_ack_us >= G2X_ACK_GAP_US;
}

int g2x_rx_input(g2x_rx *r, const uint8_t *p, size_t n, uint64_t now)
{
    g2x_data_hdr h;
    const uint8_t *d;
    int pr = g2x_data_parse(p, n, &h, &d);
    if (pr == -1) {
        if (n >= 8 && memcmp(p, "G2X1", 4) == 0 && get16(p + 6) == r->pass) { r->bad++; return 0; }
        return -1;
    }
    if (h.pass != r->pass) return -1;
    r->dgrams++;
    r->last_rx_us = now;
    if (pr == -2 || (r->need_crc && !(h.flags & G2X_F_CRC)) || h.seq >= r->count || h.len != g2x_len(r->total, h.seq)) {
        r->bad++;                                   /* treated as lost: the sender resends it */
        return ack_due(r, now);
    }
    if (h.seq < r->cum) {                           /* we have it: the sender missed an ACK */
        r->dups++; r->gap_pending = 1;
        return ack_due(r, now);
    }
    if (h.seq >= r->cum + G2X_WIN) { r->beyond++; return ack_due(r, now); }
    unsigned s = h.seq % G2X_WIN;
    if (h.seq == r->cum) {                          /* in order: straight from the datagram, no copy */
        r->deliver(r->ctx, h.seq, d, h.len);
        r->cum++; r->since_ack++;
        while (r->cum < r->count && r->cum < r->high && r->have[r->cum % G2X_WIN]) {
            unsigned c = r->cum % G2X_WIN;
            r->deliver(r->ctx, r->cum, r->slot[c], r->len[c]);
            r->have[c] = 0; r->cum++; r->since_ack++;
        }
        if (r->high < r->cum) r->high = r->cum;
    } else if (r->have[s]) {
        r->dups++; r->gap_pending = 1;
    } else {
        memcpy(r->slot[s], d, h.len);
        r->have[s] = 1; r->len[s] = h.len;
        r->gap_pending = 1;                         /* out of order: something below is missing */
        if (h.seq > r->high) r->gaps++;             /* a new hole */
        if (h.seq >= r->high) r->high = h.seq + 1;
    }
    return ack_due(r, now);
}

int g2x_rx_poll(g2x_rx *r, uint64_t now)
{
    if (!(r->since_ack || r->gap_pending || r->high > r->cum)) return 0;
    if (r->gap_pending && now - r->last_ack_us >= G2X_ACK_GAP_US) return 1;
    return now - r->last_ack_us >= G2X_ACK_IDLE_US;
}

void g2x_rx_make_ack(g2x_rx *r, g2x_ack *a, uint8_t flags, uint64_t now)
{
    memset(a, 0, sizeof *a);
    a->flags = (uint8_t)(flags | (g2x_rx_complete(r) ? G2X_A_DONE : 0));
    a->pass = r->pass;
    a->cum = r->cum;
    a->limit = r->cum + G2X_WIN;
    if (a->limit > r->count) a->limit = r->count;
    for (unsigned i = 0; i + 1 < G2X_WIN; i++) {
        uint32_t q = r->cum + 1 + i;
        if (q >= r->high) break;
        if (r->have[q % G2X_WIN]) {
            if (i < 32) a->sack_lo |= 1u << i;
            else a->sack_hi |= 1u << (i - 32);
        }
    }
    a->ack_no = ++r->ack_no;
    a->dgrams = r->dgrams; a->dups = r->dups; a->bad = r->bad;
    r->since_ack = 0; r->gap_pending = 0; r->last_ack_us = now; r->acks++;
}

/* ---------------- sender ---------------- */
void g2x_tx_init(g2x_tx *t, uint32_t count, uint8_t *acked, uint64_t *stamp, uint32_t *rq, uint8_t *queued,
                 double pps, double pps_min, double pps_max)
{
    memset(t, 0, sizeof *t);
    t->count = count; t->acked = acked; t->stamp = stamp; t->rq = rq; t->queued = queued;
    t->rq_cap = G2X_WIN;
    memset(acked, 0, count); memset(queued, 0, count); memset(stamp, 0, sizeof *stamp * count);
    t->limit = count < G2X_WIN ? count : G2X_WIN;
    t->pps = pps; t->pps_min = pps_min; t->pps_max = pps_max;
}

static void enqueue(g2x_tx *t, uint32_t s)
{
    if (t->acked[s] || t->queued[s] || t->rq_n >= t->rq_cap) return;
    t->rq[t->rq_n++] = s; t->queued[s] = 1;
}

static void cut(g2x_tx *t, uint64_t now)
{
    if (t->last_cut_us && now - t->last_cut_us < G2X_CUT_GAP_US) return;
    t->last_cut_us = now; t->loss_events++;
    if (t->pps_max > t->pps_min) { t->pps *= 0.85; if (t->pps < t->pps_min) t->pps = t->pps_min; }
}

void g2x_tx_on_ack(g2x_tx *t, const g2x_ack *a, uint64_t now)
{
    int progress = 0;
    uint32_t cum = a->cum > t->count ? t->count : a->cum;
    while (t->cum < cum) { if (!t->acked[t->cum]) { t->acked[t->cum] = 1; progress = 1; } t->cum++; }
    int64_t high = -1;
    for (unsigned i = 0; i + 1 < G2X_WIN; i++) {
        uint32_t bit = i < 32 ? (a->sack_lo >> i) & 1u : (a->sack_hi >> (i - 32)) & 1u;
        uint32_t q = cum + 1 + i;
        if (!bit || q >= t->count) continue;
        if (!t->acked[q]) { t->acked[q] = 1; progress = 1; }
        high = q;
    }
    while (t->cum < t->count && t->acked[t->cum]) t->cum++;
    if (a->limit > t->limit) t->limit = a->limit > t->count ? t->count : a->limit;
    if (progress) t->last_progress_us = now;
    /* a hole below a datagram sent after it = lost (the BBA ring is FIFO: nothing is reordered) */
    int lost = 0;
    if (high >= 0) {
        uint64_t hs = t->stamp[high];
        for (uint32_t q = t->cum; q < (uint32_t)high; q++)
            if (!t->acked[q] && !t->queued[q] && t->stamp[q] && t->stamp[q] < hs) { enqueue(t, q); lost = 1; }
    }
    if (lost) cut(t, now);
    else if (progress && t->pps_max > t->pps_min) { t->pps += 3.0; if (t->pps > t->pps_max) t->pps = t->pps_max; }
}

int64_t g2x_tx_next(g2x_tx *t, uint64_t now, int *resend)
{
    while (t->rq_n) {
        uint32_t s = t->rq[0];
        memmove(t->rq, t->rq + 1, sizeof *t->rq * --t->rq_n);
        t->queued[s] = 0;
        if (!t->acked[s]) { *resend = 1; return s; }
    }
    if (t->cum < t->next_new && t->last_progress_us && now - t->last_progress_us > G2X_RTO_US) {
        t->last_progress_us = now;                  /* back off until the next RTO */
        t->rto_fires++;
        cut(t, now);
        for (uint32_t q = t->cum; q < t->next_new; q++) enqueue(t, q);   /* all of them are >= RTO old */
        if (t->rq_n) return g2x_tx_next(t, now, resend);
    }
    if (t->next_new < t->count && t->next_new < t->limit) { *resend = 0; return t->next_new; }
    return -1;
}

void g2x_tx_sent(g2x_tx *t, uint32_t seq, int resend, uint64_t now)
{
    t->stamp[seq] = ++t->tx_no;
    if (resend) t->resent++;
    else { t->next_new++; t->sent++; }
    if (!t->last_progress_us) t->last_progress_us = now ? now : 1;   /* the RTO clock starts with the first send */
}

int g2x_tx_done(const g2x_tx *t) { return t->cum >= t->count; }
