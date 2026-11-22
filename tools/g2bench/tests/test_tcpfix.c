/* Host test of the receive decisions of tools/kos-patches/net_tcp-rx-fix.patch (KOS 2.3.0 net_tcp.c process_pkt).
 * This is a MODEL, not KOS's code: process() below copies the patched process_pkt's sequence check, RST / SYN
 * handling, out-of-order drop / overlap trim, window truncation, data copy and FIN step (same expressions and
 * order), with KOS's socket reduced to rcv.nxt, rcv.wnd, the state and a byte FIFO; the ACK processing is left
 * out (it does not change what is accepted). run_tests.sh greps the patch for the copied condition lines, so the
 * model cannot drift from the patch silently. MODE_R1 is the earlier version of the fix (43e37535: the checks
 * gated on sz), kept to show the bare out-of-order FIN case it got wrong. */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)

#define SEQ_LT(x, y)    (((int32_t)((x) - (y))) < 0)
#define SEQ_GT(x, y)    (((int32_t)((x) - (y))) > 0)
#define SEQ_GE(x, y)    (((int32_t)((x) - (y))) >= 0)
#define TCP_FLAG_FIN 0x01
#define TCP_FLAG_SYN 0x02
#define TCP_FLAG_RST 0x04
#define TCP_FLAG_ACK 0x10

enum { ST_ESTABLISHED, ST_CLOSE_WAIT, ST_RESET };
enum { MODE_FIX, MODE_R1 };

typedef struct {
    uint32_t nxt, wnd;
    int state;
    uint8_t fifo[4096]; uint32_t n;    /* what recv() would return, in order */
    unsigned acks, resets_sent, ooo, trimmed;
} sock_t;

static void sock_init(sock_t *s, uint32_t nxt) { memset(s, 0, sizeof *s); s->nxt = nxt; s->wnd = 2048; s->state = ST_ESTABLISHED; }

static void process(sock_t *s, int mode, uint32_t seq, uint16_t flags, const uint8_t *buf, size_t sz)
{
    int bad_pkt = 0;
    uint32_t seglen = (uint32_t)sz + ((flags & TCP_FLAG_FIN) ? 1 : 0);
    if (mode == MODE_R1) seglen = (uint32_t)sz;

    if (s->wnd == 0) {
        if (sz || seq != s->nxt) bad_pkt = 1;
    } else if (!sz) {
        if (!(SEQ_GE(seq, s->nxt) && SEQ_LT(seq, s->nxt + s->wnd))) bad_pkt = 1;
    } else if (mode == MODE_FIX) {
        if(!(SEQ_GE(seq, s->nxt) && SEQ_LT(seq, s->nxt + s->wnd)) &&
           !(!(flags & (TCP_FLAG_RST | TCP_FLAG_SYN)) &&
                SEQ_LT(seq, s->nxt) &&
                SEQ_GT(seq + seglen, s->nxt)))
            bad_pkt = 1;
    } else {
        if (!(SEQ_GE(seq, s->nxt) && SEQ_LT(seq, s->nxt + s->wnd)) && !(SEQ_LT(seq, s->nxt) && SEQ_GT(seq + sz, s->nxt)))
            bad_pkt = 1;
    }
    if (bad_pkt) {
        if (flags & TCP_FLAG_RST) return;
        s->acks++;
        return;
    }
    if (flags & TCP_FLAG_RST) { s->state = ST_RESET; return; }
    if (flags & TCP_FLAG_SYN) { s->resets_sent++; return; }
    if (!(flags & TCP_FLAG_ACK)) return;

    if (s->state == ST_ESTABLISHED) {
        if(sz && SEQ_LT(seq, s->nxt)) {
            buf += s->nxt - seq;
            sz -= s->nxt - seq;
            ++s->trimmed;
        }
        else if(seglen && seq != s->nxt) {
            ++s->ooo;
            sz = 0;
            bad_pkt = 1;
            s->acks++;
        }
        if (sz > s->wnd) { sz = s->wnd; bad_pkt = 1; }
        if (sz) {
            memcpy(s->fifo + s->n, buf, sz);
            s->n += (uint32_t)sz; s->nxt += (uint32_t)sz; s->wnd -= (uint32_t)sz;
            s->acks++;
        }
    } else if (sz) {
        bad_pkt = 1;
    }
    if (!bad_pkt && (flags & TCP_FLAG_FIN)) {
        ++s->nxt;
        s->acks++;
        if (s->state == ST_ESTABLISHED) s->state = ST_CLOSE_WAIT;
    }
}

static uint8_t data[4096];
static int eof_ok(const sock_t *s, uint32_t want) { return s->state == ST_CLOSE_WAIT && s->n == want && !memcmp(s->fifo, data, want); }

/* the review case: [nxt, nxt+L) lost, then the bare FIN at nxt+L */
static void test_lost_last_then_fin(int mode, uint32_t isn, int *eof_with_hole)
{
    sock_t s; sock_init(&s, isn);
    const uint32_t A = 500, L = 300;
    process(&s, mode, isn, TCP_FLAG_ACK, data, A);                                     /* [0, A) */
    /* [A, A+L) lost */
    unsigned acks = s.acks;
    process(&s, mode, isn + A + L, TCP_FLAG_ACK | TCP_FLAG_FIN, NULL, 0);              /* bare FIN past the hole */
    *eof_with_hole = s.state == ST_CLOSE_WAIT;
    if (mode != MODE_FIX) return;
    CHECK(s.state == ST_ESTABLISHED);                  /* no EOF */
    CHECK(s.nxt == isn + A);                           /* FIN not taken */
    CHECK(s.acks == acks + 1 && s.ooo == 1);           /* duplicate ACK for nxt */
    process(&s, mode, isn + A, TCP_FLAG_ACK, data + A, L);                             /* the retransmission */
    CHECK(s.nxt == isn + A + L && s.n == A + L && s.state == ST_ESTABLISHED);
    process(&s, mode, isn + A + L, TCP_FLAG_ACK | TCP_FLAG_FIN, NULL, 0);              /* the FIN again */
    CHECK(eof_ok(&s, A + L));
    CHECK(s.nxt == isn + A + L + 1);
}

/* [A, A+L) lost; the sender retransmits from A to the end with its FIN in one segment, which overlaps nothing.
   Then a segment that starts before nxt (old data + new data + FIN). */
static void test_merged(uint32_t isn)
{
    sock_t s; sock_init(&s, isn);
    process(&s, MODE_FIX, isn, TCP_FLAG_ACK, data, 400);
    process(&s, MODE_FIX, isn + 600, TCP_FLAG_ACK, data + 600, 200);                  /* past the hole: dropped */
    CHECK(s.ooo == 1 && s.n == 400);
    process(&s, MODE_FIX, isn + 300, TCP_FLAG_ACK | TCP_FLAG_FIN, data + 300, 500);   /* old 100 + new 400 + FIN */
    CHECK(s.trimmed == 1);
    CHECK(eof_ok(&s, 800));
    CHECK(s.nxt == isn + 801);

    /* old data + FIN only: the FIN sits right at nxt */
    sock_init(&s, isn);
    process(&s, MODE_FIX, isn, TCP_FLAG_ACK, data, 400);
    process(&s, MODE_FIX, isn + 100, TCP_FLAG_ACK | TCP_FLAG_FIN, data + 100, 300);
    CHECK(eof_ok(&s, 400) && s.nxt == isn + 401);
}

static void test_duplicates(uint32_t isn)
{
    sock_t s; sock_init(&s, isn);
    process(&s, MODE_FIX, isn, TCP_FLAG_ACK, data, 400);
    unsigned acks = s.acks;
    process(&s, MODE_FIX, isn + 100, TCP_FLAG_ACK, data + 100, 300);                  /* ends at nxt: all old */
    CHECK(s.n == 400 && s.nxt == isn + 400 && s.acks == acks + 1 && s.trimmed == 0);
    process(&s, MODE_FIX, isn, TCP_FLAG_ACK | TCP_FLAG_FIN, data, 399);               /* old, FIN would be at 399 */
    CHECK(s.state == ST_ESTABLISHED && s.nxt == isn + 400 && s.n == 400);
    process(&s, MODE_FIX, isn, TCP_FLAG_ACK, data, 400);                              /* the same segment again */
    CHECK(s.n == 400 && !memcmp(s.fifo, data, 400));
    process(&s, MODE_FIX, isn + 400, TCP_FLAG_ACK | TCP_FLAG_FIN, NULL, 0);
    CHECK(eof_ok(&s, 400));
    acks = s.acks;
    process(&s, MODE_FIX, isn + 400, TCP_FLAG_ACK | TCP_FLAG_FIN, NULL, 0);           /* the FIN retransmitted */
    CHECK(s.nxt == isn + 401 && s.acks == acks + 1);                                    /* re-ACKed, not taken twice */
}

static void test_rst_syn(uint32_t isn)
{
    sock_t s; sock_init(&s, isn);
    process(&s, MODE_FIX, isn, TCP_FLAG_ACK, data, 400);
    process(&s, MODE_FIX, isn + 100, TCP_FLAG_ACK | TCP_FLAG_RST, data, 400);         /* overlapping RST: not taken */
    CHECK(s.state == ST_ESTABLISHED);
    process(&s, MODE_FIX, isn + 100, TCP_FLAG_ACK | TCP_FLAG_SYN, data, 400);         /* overlapping SYN: just ACKed */
    CHECK(s.resets_sent == 0 && s.state == ST_ESTABLISHED && s.n == 400);
    process(&s, MODE_FIX, isn + 400, TCP_FLAG_ACK | TCP_FLAG_RST, NULL, 0);           /* in-window RST: reset */
    CHECK(s.state == ST_RESET);
}

int main(void)
{
    for (unsigned i = 0; i < sizeof data; i++) data[i] = (uint8_t)(i * 7 + (i >> 8));
    const uint32_t isns[] = { 1000u, 0xFFFFFF00u /* the stream crosses 2^32 */, 0u - 800u /* the FIN is at seq 0 */ };
    for (unsigned k = 0; k < 3; k++) {
        int hole;
        test_lost_last_then_fin(MODE_FIX, isns[k], &hole);
        CHECK(!hole);
        test_lost_last_then_fin(MODE_R1, isns[k], &hole);
        CHECK(hole);                                   /* the bug the fix is for: EOF with 300 bytes missing */
        test_merged(isns[k]);
        test_duplicates(isns[k]);
        test_rst_syn(isns[k]);
    }
    if (fails) { printf("test_tcpfix: %d failures\n", fails); return 1; }
    printf("test_tcpfix: all ok (model of the patched process_pkt; R1 version reproduces EOF-with-hole)\n");
    return 0;
}
