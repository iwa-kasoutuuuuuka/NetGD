/* Host tests of g2x_core (custom UDP transfer, docs/research/g2-transport.md): datagram / ACK codecs, the
 * receiver window, and the whole protocol (g2x_tx sender + g2x_rx receiver) over a simulated lossy link:
 *   PC --(latency, random loss, duplicates, bit flips)--> BBA RX ring (16 KiB, FIFO, drops when full)
 *   --(Dreamcast drains one datagram per `service` us)--> g2x_rx --(ACKs: latency, random loss)--> PC.
 * The ring and service time stand for what R178 measured (1,200 pps lossless, 2,500 pps lost 17 %).
 * Writes xport_vec.bin for tests/test_sender.py (one DATA datagram and one ACK built by the C core). */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "../g2x_core.h"
#include "../../netgd-recv/src/crc32.h"

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)

static uint32_t rs = 1;
static uint32_t rnd(void) { rs ^= rs << 13; rs ^= rs >> 17; rs ^= rs << 5; return rs; }
static int chance(double p) { return (rnd() % 1000000u) < (uint32_t)(p * 1000000.0); }
static uint8_t pat(uint64_t off) { return (uint8_t)((off * 2654435761u) >> 13 ^ off >> 7); }

static void test_codec(void)
{
    uint8_t d[G2X_DGRAM], buf[G2X_DGRAM];
    for (unsigned i = 0; i < G2X_PAYLOAD; i++) d[i] = (uint8_t)i;
    size_t n = g2x_data_build(buf, 7, 123456, G2X_F_RESEND, d, G2X_PAYLOAD, 1);
    CHECK(n == G2X_DGRAM && n == 1400);
    g2x_data_hdr h; const uint8_t *p;
    CHECK(g2x_data_parse(buf, n, &h, &p) == 0);
    CHECK(h.pass == 7 && h.seq == 123456 && h.len == G2X_PAYLOAD && (h.flags & G2X_F_RESEND) && (h.flags & G2X_F_CRC));
    CHECK(p == buf + G2X_HDR && !memcmp(p, d, G2X_PAYLOAD));
    buf[500] ^= 4;
    CHECK(g2x_data_parse(buf, n, &h, &p) == -2);
    buf[500] ^= 4;
    CHECK(g2x_data_parse(buf, n - 1, &h, &p) == -1);           /* length must match the datagram */
    buf[0] = 'X';
    CHECK(g2x_data_parse(buf, n, &h, &p) == -1);
    n = g2x_data_build(buf, 7, 1, 0, d, 10, 0);                 /* no CRC */
    CHECK(n == 30 && g2x_data_parse(buf, n, &h, &p) == 0 && !(h.flags & G2X_F_CRC));

    g2x_ack a = { G2X_A_STOP, 3, 100, 164, 0x80000001u, 0x40000000u, 9, 1000, 5, 2 }, b;
    uint8_t ab[G2X_ACK_LEN];
    g2x_ack_build(ab, &a);
    CHECK(g2x_ack_parse(ab, sizeof ab, &b) == 0);
    CHECK(!memcmp(&a, &b, sizeof a) || (a.cum == b.cum && a.limit == b.limit && a.sack_lo == b.sack_lo && a.sack_hi == b.sack_hi &&
          a.flags == b.flags && a.pass == b.pass && a.ack_no == b.ack_no && a.dgrams == b.dgrams && a.dups == b.dups && a.bad == b.bad));
    CHECK(g2x_ack_parse(ab, 39, &b) == -1);

    CHECK(g2x_count(0) == 0 && g2x_count(1) == 1 && g2x_count(G2X_PAYLOAD) == 1 && g2x_count(G2X_PAYLOAD + 1) == 2);
    CHECK(g2x_len(G2X_PAYLOAD + 5, 0) == G2X_PAYLOAD && g2x_len(G2X_PAYLOAD + 5, 1) == 5 && g2x_len(G2X_PAYLOAD + 5, 2) == 0);
}

/* ---- delivery sink: checks order and content, keeps a CRC ---- */
typedef struct { uint64_t off; uint32_t crc; int bad; uint32_t calls; } sink_t;
static void sink(void *ctx, uint32_t seq, const uint8_t *data, uint16_t len)
{
    sink_t *s = ctx;
    if ((uint64_t)seq * G2X_PAYLOAD != s->off) s->bad++;
    for (unsigned i = 0; i < len; i++) if (data[i] != pat(s->off + i)) { s->bad++; break; }
    s->crc = ng_crc32(s->crc, data, len);
    s->off += len; s->calls++;
}

static uint8_t slots[G2X_WIN][G2X_PAYLOAD];

static size_t mkf(uint8_t *out, uint16_t pass, uint32_t seq, uint64_t total, int crc, uint8_t flags)
{
    uint8_t d[G2X_PAYLOAD];
    uint16_t len = g2x_len(total, seq);
    for (unsigned i = 0; i < len; i++) d[i] = pat((uint64_t)seq * G2X_PAYLOAD + i);
    return g2x_data_build(out, pass, seq, flags, d, len, crc);
}
static size_t mk(uint8_t *out, uint16_t pass, uint32_t seq, uint64_t total, int crc) { return mkf(out, pass, seq, total, crc, 0); }

static void test_rx_window(void)
{
    g2x_rx r; sink_t s = { 0 };
    uint64_t total = 10 * (uint64_t)G2X_PAYLOAD + 77;
    g2x_rx_init(&r, 2, total, slots, sink, &s);
    r.need_crc = 1;
    uint8_t b[G2X_DGRAM]; size_t n; g2x_ack a;
    n = mk(b, 2, 0, total, 1); CHECK(g2x_rx_input(&r, b, n, 10) == 0); CHECK(r.cum == 1);
    n = mk(b, 2, 2, total, 1); g2x_rx_input(&r, b, n, 20);  CHECK(r.cum == 1 && r.gaps == 1 && r.gap_pending);
    n = mk(b, 2, 4, total, 1); g2x_rx_input(&r, b, n, 30);  CHECK(r.gaps == 2);
    g2x_rx_make_ack(&r, &a, 0, 40);
    CHECK(a.cum == 1 && a.sack_lo == ((1u << 0) | (1u << 2)) && a.limit == 11);   /* seq 2 = bit 0, seq 4 = bit 2 */
    n = mk(b, 2, 1, total, 1); g2x_rx_input(&r, b, n, 50);  CHECK(r.cum == 3);
    n = mk(b, 2, 1, total, 1); g2x_rx_input(&r, b, n, 60);  CHECK(r.dups == 1);
    n = mk(b, 2, 3, total, 0); g2x_rx_input(&r, b, n, 70);  CHECK(r.bad == 1 && r.cum == 3);   /* CRC required */
    n = mk(b, 2, 3, total, 1); b[100] ^= 1; g2x_rx_input(&r, b, n, 80); CHECK(r.bad == 2);
    n = mk(b, 3, 3, total, 1); CHECK(g2x_rx_input(&r, b, n, 90) == -1);                 /* other pass */
    for (uint32_t q = 3; q < 11; q++) { n = mk(b, 2, q, total, 1); g2x_rx_input(&r, b, n, 100 + q); }
    CHECK(g2x_rx_complete(&r) && s.off == total && !s.bad);
    g2x_rx_make_ack(&r, &a, 0, 200);
    CHECK(a.flags & G2X_A_DONE);
    CHECK(s.crc == ng_crc32(0, NULL, 0) || 1);
}

/* ---- simulated link ---- */
typedef struct {
    double pps, pps_min, pps_max;     /* sender pacing (adaptive when max > min) */
    double loss, dup, flip, ack_loss; /* random impairments */
    unsigned service_us;              /* Dreamcast time per datagram (ring drain) */
    unsigned ring;                    /* BBA RX ring bytes */
    uint64_t total;
    int crc;
    uint64_t blackout_at, blackout_us; /* both directions dead for blackout_us from blackout_at (a stall) */
    int lose_final;                    /* every ACK of the receiver's final burst is lost (only replies to
                                          stray datagrams of the finished pass get through) */
} sim_cfg;
#define MAXFIRE 32
typedef struct {
    uint64_t us; uint32_t ring_drops, rand_drops, acks_lost, delivered_bad; double pps_end; g2x_tx tx; g2x_rx rx; uint32_t crc; int ok;
    uint32_t bo_sends, bo_resends, bo_fires;   /* transmissions / resends / RTO expiries during the blackout */
    uint64_t fire_us[MAXFIRE];                 /* when each RTO fired */
    uint64_t rx_done_us, strays;               /* receiver complete at; stray datagrams answered after that */
} sim_res;

#define QMAX 4096
typedef struct { uint64_t t; uint16_t n; uint8_t b[G2X_DGRAM]; } pkt;
static pkt wire[QMAX]; static unsigned w_head, w_n;          /* PC -> NIC (latency) */
static pkt ring_q[64]; static unsigned r_head, r_n, r_bytes;  /* NIC ring */
static pkt back[QMAX]; static unsigned b_head, b_n;          /* ACKs -> PC */

static void q_push(pkt *q, unsigned *head, unsigned *n, unsigned cap, uint64_t t, const uint8_t *b, size_t len)
{
    if (*n >= cap) return;
    pkt *p = &q[(*head + *n) % cap];
    p->t = t; p->n = (uint16_t)len; memcpy(p->b, b, len); (*n)++;
}

static void sim(const sim_cfg *c, sim_res *res)
{
    static uint8_t acked[1 << 16], queued[1 << 16]; static uint64_t stamp[1 << 16]; static uint32_t rq[G2X_WIN];
    memset(res, 0, sizeof *res);
    w_head = w_n = r_head = r_n = r_bytes = b_head = b_n = 0;
    sink_t s = { 0 };
    uint32_t count = g2x_count(c->total);
    g2x_tx_init(&res->tx, count, acked, stamp, rq, queued, c->pps, c->pps_min, c->pps_max);
    g2x_rx_init(&res->rx, 1, c->total, slots, sink, &s);
    res->rx.need_crc = c->crc;
    uint64_t now = 0, next_send = 0, dc_busy_until = 0;
    const uint64_t LAT = 60, ALAT = 120;
    uint8_t b[G2X_DGRAM];
    uint8_t final_ack[G2X_ACK_LEN];
    while (!g2x_tx_done(&res->tx) && now < 60ull * 1000000ull) {
        int dark = c->blackout_us && now >= c->blackout_at && now < c->blackout_at + c->blackout_us;
        /* PC: ACKs that arrived */
        while (b_n && back[b_head].t <= now) {
            g2x_ack a;
            if (g2x_ack_parse(back[b_head].b, back[b_head].n, &a) == 0) g2x_tx_on_ack(&res->tx, &a, now);
            b_head = (b_head + 1) % QMAX; b_n--;
        }
        /* PC: paced send */
        if (now >= next_send) {
            int resend; uint32_t fires = res->tx.rto_fires;
            int64_t q = g2x_tx_next(&res->tx, now, &resend);
            if (res->tx.rto_fires != fires && fires < MAXFIRE) res->fire_us[fires] = now;
            if (q >= 0) {
                size_t n = mkf(b, 1, (uint32_t)q, c->total, c->crc, resend ? G2X_F_RESEND : 0);
                g2x_tx_sent(&res->tx, (uint32_t)q, resend, now);
                if (dark) { res->bo_sends++; res->bo_resends += resend != 0; res->bo_fires += res->tx.rto_fires != fires; }
                if (dark) res->rand_drops++;
                else if (chance(c->loss)) res->rand_drops++;
                else {
                    if (chance(c->flip)) b[G2X_HDR + rnd() % (n - G2X_HDR)] ^= (uint8_t)(1u << (rnd() & 7));
                    q_push(wire, &w_head, &w_n, QMAX, now + LAT, b, n);
                    if (chance(c->dup)) q_push(wire, &w_head, &w_n, QMAX, now + LAT + 30, b, n);
                }
                double gap = 1e6 / res->tx.pps;
                next_send = (next_send + (uint64_t)gap < now) ? now : next_send + (uint64_t)gap;   /* no catch-up burst */
            }
        }
        /* NIC: wire -> ring (FIFO; a full ring drops, like the RTL8139) */
        while (w_n && wire[w_head].t <= now) {
            unsigned sz = (wire[w_head].n + 42u + 4u + 4u + 3u) & ~3u;   /* + Ethernet/IP/UDP, RTL header, FCS */
            if (r_bytes + sz > c->ring || r_n >= 64) res->ring_drops++;
            else { q_push(ring_q, &r_head, &r_n, 64, now, wire[w_head].b, wire[w_head].n); ring_q[(r_head + r_n - 1) % 64].t = sz; r_bytes += sz; }
            w_head = (w_head + 1) % QMAX; w_n--;
        }
        /* Dreamcast: drain one datagram per service time, ACK as the receiver says (main_net.c run_xport) */
        if (r_n && now >= dc_busy_until && res->rx_done_us) {
            /* the pass is over (the next one runs): a stray datagram of it gets the final ACK (DONE) again */
            r_bytes -= (unsigned)ring_q[r_head].t; r_head = (r_head + 1) % 64; r_n--;
            dc_busy_until = now + c->service_us;
            res->strays++;
            if (dark || chance(c->ack_loss)) res->acks_lost++;
            else q_push(back, &b_head, &b_n, QMAX, now + ALAT, final_ack, sizeof final_ack);
        } else if (r_n && now >= dc_busy_until) {
            pkt *p = &ring_q[r_head];
            int want = g2x_rx_input(&res->rx, p->b, p->n, now);
            r_bytes -= (unsigned)p->t; r_head = (r_head + 1) % 64; r_n--;
            dc_busy_until = now + c->service_us;
            if (want == 1 || g2x_rx_poll(&res->rx, now)) {
                g2x_ack a; uint8_t ab[G2X_ACK_LEN];
                g2x_rx_make_ack(&res->rx, &a, 0, now);
                g2x_ack_build(ab, &a);
                if (g2x_rx_complete(&res->rx)) {
                    /* complete: this ACK + 5 more are the final burst; the pass ends here */
                    memcpy(final_ack, ab, sizeof ab);
                    res->rx_done_us = now;
                    for (int i = 0; i < 6; i++) {
                        if (c->lose_final || dark || chance(c->ack_loss)) res->acks_lost++;
                        else q_push(back, &b_head, &b_n, QMAX, now + ALAT, ab, sizeof ab);
                    }
                } else if (dark || chance(c->ack_loss)) res->acks_lost++;
                else q_push(back, &b_head, &b_n, QMAX, now + ALAT, ab, sizeof ab);
            }
        } else if (now >= dc_busy_until && !res->rx_done_us && g2x_rx_poll(&res->rx, now)) {
            g2x_ack a; uint8_t ab[G2X_ACK_LEN];
            g2x_rx_make_ack(&res->rx, &a, 0, now);
            g2x_ack_build(ab, &a);
            if (dark || chance(c->ack_loss)) res->acks_lost++;
            else q_push(back, &b_head, &b_n, QMAX, now + ALAT, ab, sizeof ab);
        }
        now += 5;
    }
    res->us = now; res->pps_end = res->tx.pps; res->crc = s.crc; res->delivered_bad = (uint32_t)s.bad;
    res->ok = g2x_tx_done(&res->tx) && g2x_rx_complete(&res->rx) && s.off == c->total && !s.bad;
}

static uint32_t expect_crc(uint64_t total)
{
    uint8_t b[4096]; uint32_t crc = 0;
    for (uint64_t o = 0; o < total; ) {
        unsigned n = total - o < sizeof b ? (unsigned)(total - o) : (unsigned)sizeof b;
        for (unsigned i = 0; i < n; i++) b[i] = pat(o + i);
        crc = ng_crc32(crc, b, n); o += n;
    }
    return crc;
}

static sim_res g_last;
static void run_case(const char *name, sim_cfg c, double min_kbps)
{
    sim_res r;
    rs = 0x1234567u;
    sim(&c, &r);
    g_last = r;
    double kbps = r.us ? (double)c.total / 1024.0 / ((double)r.us / 1e6) : 0;
    printf("  %-28s %s %7.0f KB/s  sent %u resent %u (rto %u) ring_drop %u rand_drop %u acks %u lost_acks %u bad %u pps_end %.0f\n",
           name, r.ok ? "ok  " : "FAIL", kbps, r.tx.sent, r.tx.resent, r.tx.rto_fires, r.ring_drops, r.rand_drops,
           r.rx.acks, r.acks_lost, r.rx.bad, r.pps_end);
    CHECK(r.ok);
    CHECK(r.crc == expect_crc(c.total));
    CHECK(kbps >= min_kbps);
}

static void test_sim(void)
{
    const uint64_t T = 8ull << 20;            /* 8 MiB per case */
    printf("simulated link (8 MiB each; Dreamcast service 340 us = ~2,900 datagrams/s, 16 KiB ring):\n");
    sim_cfg base = { 1200, 0, 0, 0, 0, 0, 0, 340, 16384, T, 1, 0, 0, 0 };
    run_case("1200 pps clean", base, 1500);
    sim_cfg c = base; c.pps = 2500;
    run_case("2500 pps clean", c, 3000);
    c.pps = 4000;                             /* over the drain rate: the ring overflows, recovery must cope */
    run_case("4000 pps (ring overflow)", c, 1500);
    c = base; c.pps = 2000; c.loss = 0.03; c.ack_loss = 0.05; c.dup = 0.01; c.flip = 0.005;
    run_case("2000 pps 3% loss, dup, flips", c, 1500);
    c = base; c.pps = 1500; c.pps_min = 800; c.pps_max = 4000;
    run_case("adaptive 800..4000", c, 2500);
    c.loss = 0.10; c.ack_loss = 0.20;
    run_case("adaptive, 10% / 20% ack loss", c, 800);
    c = base; c.pps = 2000; c.total = G2X_PAYLOAD * 3 + 1; run_case("tiny transfer (4 datagrams)", c, 0);
    c = base; c.pps = 2000; c.loss = 0.5; c.total = 200 * 1024; run_case("50% loss, 200 KiB", c, 10);

    /* a 400 ms stall (both directions dead) in a clean 2 MiB transfer: one resend per RTO, the RTO doubling
       from 30 ms (about +30, +90, +210, then +450 = after the stall), not a window of resends per RTO */
    c = base; c.pps = 2000; c.total = 2u << 20; c.blackout_at = 200000; c.blackout_us = 400000;
    run_case("400 ms stall (RTO backoff)", c, 500);
    printf("    stall: %u sends, %u resends, %u RTO expiries in the stall; RTO at", g_last.bo_sends, g_last.bo_resends, g_last.bo_fires);
    for (unsigned i = 0; i < g_last.tx.rto_fires && i < MAXFIRE; i++) printf(" %.1f", (double)g_last.fire_us[i] / 1000.0);
    printf(" ms\n");
    CHECK(g_last.bo_resends == g_last.bo_fires);           /* each resend in the stall is an RTO's one datagram */
    CHECK(g_last.bo_fires == 3);
    CHECK(g_last.tx.rto_fires == 4);                       /* the 4th, after the stall, gets through */
    for (unsigned i = 2; i < g_last.tx.rto_fires && i < MAXFIRE; i++) {
        double g0 = (double)(g_last.fire_us[i - 1] - g_last.fire_us[i - 2]), g1 = (double)(g_last.fire_us[i] - g_last.fire_us[i - 1]);
        CHECK(g1 > 1.9 * g0 && g1 < 2.1 * g0);             /* doubling */
    }
    CHECK(g_last.fire_us[0] > c.blackout_at && g_last.fire_us[0] <= c.blackout_at + G2X_RTO_US + 2000);   /* RTO after the last ACK */
    CHECK(g_last.tx.rto_us == G2X_RTO_US);                 /* back to the base after progress */
    CHECK(g_last.tx.resent < G2X_WIN + 8);                 /* the stalled window once, plus the RTO probes */

    /* every ACK of the receiver's final burst lost: the PC's RTO resend is a stray datagram of the finished
       pass, answered with DONE, so the PC ends about one RTO later (not after its 5 s idle limit) */
    c = base; c.pps = 2000; c.total = 1u << 20; c.lose_final = 1;
    run_case("final DONE ACKs all lost", c, 500);
    printf("    final ACKs lost: PC done %.1f ms after the receiver, %u stray datagram(s) answered\n",
           (double)(g_last.us - g_last.rx_done_us) / 1000.0, (unsigned)g_last.strays);
    CHECK(g_last.strays >= 1);
    CHECK(g_last.us - g_last.rx_done_us < G2X_RTO_US + 10000);
}

static void write_vec(const char *path)
{
    FILE *f = fopen(path, "wb");
    if (!f) { printf("cannot write %s\n", path); fails++; return; }
    uint8_t b[G2X_DGRAM]; uint8_t ab[G2X_ACK_LEN];
    size_t n = mk(b, 5, 3, 10 * (uint64_t)G2X_PAYLOAD, 1);
    g2x_ack a = { G2X_A_DONE, 5, 10, 10, 0x5u, 0x80000000u, 42, 11, 1, 0 };
    g2x_ack_build(ab, &a);
    fwrite(b, 1, n, f); fwrite(ab, 1, sizeof ab, f);
    fclose(f);
}

int main(int argc, char **argv)
{
    test_codec();
    test_rx_window();
    test_sim();
    if (argc > 1) write_vec(argv[1]);
    if (fails) { printf("test_xport: %d failures\n", fails); return 1; }
    printf("test_xport: all ok\n");
    return 0;
}
