/* g2netbench: phase D of docs/research/g2-perf-plan.md. Measures how fast the Dreamcast can RECEIVE the data
 * stream over KallistiOS's own TCP/IP stack on the BBA, with the NGTP frames and the checksums of netgd-recv's
 * plan (docs/design/menu-app.md 8: the Dreamcast sends WANT, the PC answers DATA, up to 64 KiB per frame), and
 * the data goes into a sink (a 64 KiB buffer) instead of the SD card.
 *
 * Needs a Dreamcast + BBA started through dcload-ip (dc_auto.ps1 + the g2netbench preset). KOS is started with
 * INIT_NET: it takes over the BBA from dcload-ip (same IP), the dc-tool console and /pc/ keep working through
 * KOS's fs_dclsocket. The PC side is g2n_send.py (started by the preset). Never touches the GD-ROM drive:
 * INIT_CDROM is off, no G1 / ATA / cdrom call.
 *
 * Passes (TCP): want size / WANTs in flight / receive buffer / checksums on or off. Then UDP bursts (what a
 * custom UDP transfer could count on). Results are sent to the PC as LINE frames (key=value); g2n_send.py
 * writes them to host\g2net_NNN.txt. See README_ja.md. */
#include <kos.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include "g2b_core.h"
#include "g2n_core.h"
#include "../netgd-recv/src/crc32.h"

KOS_INIT_FLAGS((INIT_DEFAULT & ~INIT_CDROM) | INIT_NET);

#define WAIT_PC_MS     600000     /* wait this long for the PC sender to connect */
#define FRAME_TO_MS    15000      /* inside a frame (menu-app.md 8: 15 s) */
#define DATA_TO_MS     30000      /* WANT -> DATA (menu-app.md 8: 30 s) */
#define PASS_LIMIT_US  (20ull * 1000000ull)
#define PASS_BYTES     (32ull << 20)

static uint64_t now_us(void) { return timer_us_gettime64(); }

static int g_fd = -1;
static uint32_t g_txseq;                       /* sequence of the request being served */
static uint8_t g_tx[G2N_HDR + 256 + 4];
static uint8_t g_rxbuf[G2N_MAX_PAYLOAD + 4] __attribute__((aligned(32)));
static uint8_t g_sink[G2N_MAX_DATA] __attribute__((aligned(32)));

typedef struct {
    uint64_t recv_us, send_us, copy_us, crc_frame_us, crc_file_us, fnv_us, max_frame_us;
    uint64_t frames, recv_calls;
} g2n_stat;

static uint32_t le32(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

/* ---- socket helpers ---- */
static int wait_readable(int fd, int ms)
{
    struct pollfd p = { fd, POLLIN, 0 };
    int r = poll(&p, 1, ms);
    return r > 0 ? 1 : (r == 0 ? 0 : -1);
}

/* all n bytes, or -1 on error / close (errno 0) / timeout (ETIMEDOUT) */
static int recv_all(int fd, uint8_t *buf, size_t n, int ms, g2n_stat *st)
{
    size_t got = 0;
    uint64_t t0 = now_us();
    while (got < n) {
        int w = wait_readable(fd, ms);
        if (w <= 0) { if (st) st->recv_us += now_us() - t0; errno = w == 0 ? ETIMEDOUT : errno; return -1; }
        ssize_t r = recv(fd, buf + got, n - got, 0);
        if (st) st->recv_calls++;
        if (r <= 0) { if (st) st->recv_us += now_us() - t0; if (r == 0) errno = 0; return -1; }
        got += (size_t)r;
    }
    if (st) st->recv_us += now_us() - t0;
    return 0;
}

static int send_all(int fd, const uint8_t *buf, size_t n, g2n_stat *st)
{
    size_t sent = 0;
    uint64_t t0 = now_us();
    while (sent < n) {
        ssize_t r = send(fd, buf + sent, n - sent, 0);
        if (r <= 0) { if (st) st->send_us += now_us() - t0; return -1; }
        sent += (size_t)r;
    }
    if (st) st->send_us += now_us() - t0;
    return 0;
}

static int send_frame(uint8_t type, const void *payload, uint32_t n, g2n_stat *st)
{
    size_t len = g2n_frame_build(g_tx, type, 0, g_txseq, payload, n);
    return send_all(g_fd, g_tx, len, st);
}

static void send_line(const char *text)
{
    send_frame(G2N_T_LINE, text, (uint32_t)strlen(text), NULL);
}

/* a frame whose payload is at most 256 bytes must fit g_tx: LINE texts are short (<= 120) */

/* the buffer of g2b_kv, one LINE per line */
static void send_kv(const char *text)
{
    char line[120];
    while (*text) {
        size_t n = strcspn(text, "\n");
        if (n >= sizeof line) n = sizeof line - 1;
        memcpy(line, text, n); line[n] = 0;
        if (n) { send_line(line); printf("[g2net] %s\n", line); }
        text += n; if (*text == '\n') text++;
    }
}

/* ---- one TCP pass ---- */
typedef struct { uint32_t want, depth, rcvbuf; int ck; } pass_cfg;
static const pass_cfg PASSES[] = {
    { 16384, 1,  8192, 1 },   /* KOS default receive buffer (8 KiB window) */
    { 16384, 1, 65535, 1 },
    { 65536, 1, 65535, 1 },   /* netgd-recv's plan (menu-app.md D6) */
    { 65536, 2, 65535, 1 },   /* one WANT read ahead */
    { 65536, 1, 65535, 0 },   /* no checksums: the network alone */
    { 65536, 2, 65535, 0 },
};

static char g_text[2048];
static g2b_kv g_kv;

static void kvp(const char *pfx, const char *key, uint64_t v)
{
    char k[64];
    snprintf(k, sizeof k, "%s_%s", pfx, key);
    g2b_kv_u64(&g_kv, k, v);
}

static void kvs(const char *pfx, const char *key, const char *v)
{
    char k[64];
    snprintf(k, sizeof k, "%s_%s", pfx, key);
    g2b_kv_str(&g_kv, k, v);
}

static void kvh(const char *pfx, const char *key, uint32_t v)
{
    char k[64];
    snprintf(k, sizeof k, "%s_%s", pfx, key);
    g2b_kv_hex32(&g_kv, k, v);
}

static int run_pass(unsigned n, const pass_cfg *c)
{
    char pfx[16];
    snprintf(pfx, sizeof pfx, "p%u", n);
    g2b_kv_init(&g_kv, g_text, sizeof g_text);
    kvp(pfx, "want", c->want); kvp(pfx, "depth", c->depth); kvp(pfx, "checksums", (uint64_t)c->ck);

    uint32_t rb = c->rcvbuf;
    int sorc = setsockopt(g_fd, SOL_SOCKET, SO_RCVBUF, &rb, sizeof rb);
    uint32_t got_rb = 0; socklen_t gl = sizeof got_rb;
    if (getsockopt(g_fd, SOL_SOCKET, SO_RCVBUF, &got_rb, &gl) != 0) got_rb = 0;
    kvp(pfx, "rcvbuf_set_ok", (uint64_t)(sorc == 0)); kvp(pfx, "rcvbuf", got_rb);

    g2n_pass ps;
    g2n_pass_init(&ps, (uint16_t)n, PASS_BYTES, c->want, c->depth);
    g2n_stat st; memset(&st, 0, sizeof st);
    uint32_t crc = 0, fnv = NG_FNV_INIT;
    const char *status = "ok";
    uint64_t t0 = now_us();
    uint8_t hdr[G2N_HDR];

    while (!g2n_pass_done(&ps)) {
        uint64_t off; uint32_t len; uint8_t w[16];
        while (g2n_pass_next_want(&ps, &off, &len)) {
            g2n_want_build(w, ps.idx, off, len);
            if (send_frame(G2N_T_WANT, w, sizeof w, &st) != 0) { status = "send_failed"; goto out; }
        }
        if (g2n_pass_done(&ps)) break;
        uint64_t f0 = now_us();
        if (recv_all(g_fd, hdr, G2N_HDR, ps.outstanding ? DATA_TO_MS : FRAME_TO_MS, &st) != 0) {
            status = errno == ETIMEDOUT ? "timeout" : "recv_failed"; goto out;
        }
        g2n_hdr h;
        if (g2n_hdr_parse(hdr, &h) != 0 || h.type != G2N_T_DATA || h.seq != g_txseq || h.len < 12) { status = "bad_header"; goto out; }
        if (recv_all(g_fd, g_rxbuf, h.len + 4, FRAME_TO_MS, &st) != 0) { status = "recv_failed"; goto out; }
        if (c->ck) {
            uint64_t t = now_us();
            uint32_t fc = g2n_frame_crc(hdr, g_rxbuf, h.len);
            st.crc_frame_us += now_us() - t;
            if (fc != le32(g_rxbuf + h.len)) { status = "bad_frame_crc"; goto out; }
        }
        uint16_t idx; uint64_t foff; uint32_t dlen; const uint8_t *data;
        if (g2n_data_parse(g_rxbuf, h.len, &idx, &foff, &dlen, &data) != 0) { status = "bad_data"; goto out; }
        if (g2n_pass_on_data(&ps, idx, foff, dlen) != 0) { status = "bad_sequence"; goto out; }
        uint64_t t = now_us();
        memcpy(g_sink, data, dlen);                    /* netgd-recv's ng_src_t.read copies into its buffer */
        st.copy_us += now_us() - t;
        if (c->ck) {
            t = now_us(); crc = ng_crc32(crc, g_sink, dlen); st.crc_file_us += now_us() - t;
            t = now_us(); fnv = ng_fnv1a(fnv, g_sink, dlen); st.fnv_us += now_us() - t;
        }
        st.frames++;
        uint64_t ft = now_us() - f0;
        if (ft > st.max_frame_us) st.max_frame_us = ft;
        if (!ps.stopped && now_us() - t0 > PASS_LIMIT_US) { g2n_pass_stop(&ps); status = "time_limit"; }
    }
out:;
    uint64_t us = now_us() - t0;
    uint64_t accounted = st.recv_us + st.send_us + st.copy_us + st.crc_frame_us + st.crc_file_us + st.fnv_us;
    int good = strcmp(status, "ok") == 0 || strcmp(status, "time_limit") == 0;
    kvp(pfx, "status_ok", (uint64_t)good);
    kvs(pfx, "status", status);
    kvp(pfx, "bytes", ps.recv_off); kvp(pfx, "us", us); kvp(pfx, "kbps", g2b_kbps(ps.recv_off, us));
    kvp(pfx, "frames", st.frames); kvp(pfx, "recv_calls", st.recv_calls);
    kvp(pfx, "recv_wait_us", st.recv_us); kvp(pfx, "send_us", st.send_us); kvp(pfx, "copy_us", st.copy_us);
    kvp(pfx, "crc_frame_us", st.crc_frame_us); kvp(pfx, "crc_file_us", st.crc_file_us); kvp(pfx, "fnv_us", st.fnv_us);
    kvp(pfx, "other_us", us > accounted ? us - accounted : 0);
    kvp(pfx, "max_frame_us", st.max_frame_us);
    kvh(pfx, "crc", crc); kvh(pfx, "fnv", fnv);
    send_kv(g_text);
    return good ? 0 : -1;
}

/* ---- UDP burst: the PC sends `count` datagrams of `size` bytes at `pps` per second (0 = as fast as it can) ---- */
static int run_udp(int ufd, unsigned n, unsigned count, unsigned size, unsigned pps)
{
    char pfx[16], line[96];
    snprintf(pfx, sizeof pfx, "u%u", n);
    g2n_udp_stat us; g2n_udp_stat_init(&us);
    snprintf(line, sizeof line, "G2N-UDP go pass=%u count=%u size=%u pps=%u", n, count, size, pps);
    send_line(line);
    static uint8_t pkt[2048];
    uint64_t t0 = now_us(), tfirst = 0, tlast = 0, tidle = t0;
    int ended = 0;
    uint64_t pkts_seen = 0;
    while (!ended) {
        struct sockaddr_in from; socklen_t fl = sizeof from;
        ssize_t r = recvfrom(ufd, pkt, sizeof pkt, 0, (struct sockaddr *)&from, &fl);
        uint64_t t = now_us();
        if (r < 0) {
            if (t - tidle > (pkts_seen ? 3000000ull : 15000000ull)) break;   /* idle: the end packets were lost */
            thd_pass();
            continue;
        }
        tidle = t;
        uint8_t type; uint16_t pass; uint32_t seq;
        if (g2n_udp_parse(pkt, (uint32_t)r, &type, &pass, &seq) != 0 || pass != n) continue;
        pkts_seen++;
        if (type == G2N_U_END) { ended = 1; break; }
        if (!tfirst) tfirst = t;
        tlast = t;
        g2n_udp_stat_pkt(&us, seq, (uint32_t)r);
    }
    g2b_kv_init(&g_kv, g_text, sizeof g_text);
    uint64_t span = tlast > tfirst ? tlast - tfirst : 0;
    kvp(pfx, "count", count); kvp(pfx, "size", size); kvp(pfx, "pps", pps);
    kvp(pfx, "got", us.got); kvp(pfx, "missing", g2n_udp_stat_missing(&us)); kvp(pfx, "late", us.late);
    kvp(pfx, "bytes", us.bytes); kvp(pfx, "span_us", span); kvp(pfx, "kbps", g2b_kbps(us.bytes, span));
    kvp(pfx, "end_seen", (uint64_t)ended);
    send_kv(g_text);
    (void)t0;
    return 0;
}

/* ---- main ---- */
static void ip_str(char *out, size_t n)
{
    snprintf(out, n, "?.?.?.?");
    if (net_default_dev)
        snprintf(out, n, "%u.%u.%u.%u", net_default_dev->ip_addr[0], net_default_dev->ip_addr[1],
                 net_default_dev->ip_addr[2], net_default_dev->ip_addr[3]);
}

static int read_request(uint8_t want_type, uint32_t max_len, g2n_hdr *h)
{
    uint8_t hdr[G2N_HDR];
    if (recv_all(g_fd, hdr, G2N_HDR, FRAME_TO_MS, NULL) != 0 || g2n_hdr_parse(hdr, h) != 0 || h->type != want_type ||
        h->len > max_len || recv_all(g_fd, g_rxbuf, h->len + 4, FRAME_TO_MS, NULL) != 0 ||
        g2n_frame_crc(hdr, g_rxbuf, h->len) != le32(g_rxbuf + h->len))
        return -1;
    g_txseq = h->seq;
    return 0;
}

int main(int argc, char **argv)
{
    (void)argc; (void)argv;
    char ip[24];
    ip_str(ip, sizeof ip);
    printf("[g2net] start (no GD-ROM access; INIT_CDROM off; KOS net on, dcload type %d). IP %s TCP %u UDP %u\n",
           dcload_type, ip, G2N_TCP_PORT, G2N_UDP_PORT);
    if (!net_default_dev) { printf("[g2net] KOS has no network device\n"); return 1; }

    int lfd = socket(AF_INET, SOCK_STREAM, 0);
    int ufd = socket(AF_INET, SOCK_DGRAM, 0);
    if (lfd < 0 || ufd < 0) { printf("[g2net] socket() failed errno=%d\n", errno); return 1; }
    struct sockaddr_in a; memset(&a, 0, sizeof a);
    a.sin_family = AF_INET; a.sin_addr.s_addr = htonl(INADDR_ANY);
    a.sin_port = htons(G2N_TCP_PORT);
    if (bind(lfd, (struct sockaddr *)&a, sizeof a) != 0 || listen(lfd, 1) != 0) { printf("[g2net] TCP bind/listen failed errno=%d\n", errno); return 1; }
    a.sin_port = htons(G2N_UDP_PORT);
    if (bind(ufd, (struct sockaddr *)&a, sizeof a) != 0) { printf("[g2net] UDP bind failed errno=%d\n", errno); return 1; }
    fcntl(ufd, F_SETFL, fcntl(ufd, F_GETFL, 0) | O_NONBLOCK);
    printf("[g2net] listening; waiting for the PC sender (up to %d s)\n", WAIT_PC_MS / 1000);

    if (wait_readable(lfd, WAIT_PC_MS) <= 0) { printf("[g2net] no PC connected\n"); return 1; }
    struct sockaddr_in peer; socklen_t pl = sizeof peer;
    g_fd = accept(lfd, (struct sockaddr *)&peer, &pl);
    if (g_fd < 0) { printf("[g2net] accept failed errno=%d\n", errno); return 1; }
    printf("[g2net] PC connected\n");

    g2n_hdr h;
    if (read_request(G2N_T_HELLO, 256, &h) != 0) { printf("[g2net] bad HELLO\n"); return 1; }
    uint8_t ok[44]; memset(ok, 0, sizeof ok);
    ok[0] = 1;                                         /* version */
    ok[4] = (uint8_t)G2N_MAX_DATA; ok[5] = (uint8_t)(G2N_MAX_DATA >> 8); ok[6] = (uint8_t)(G2N_MAX_DATA >> 16);
    memcpy(ok + 34, "g2netbench", 10);
    send_frame(G2N_T_HELLO_OK, ok, sizeof ok, NULL);

    /* PUT starts the measurement (its manifest is ignored) */
    if (read_request(G2N_T_PUT, 4096, &h) != 0) { printf("[g2net] no PUT\n"); return 1; }
    printf("[g2net] PUT received: measuring\n");
    g2b_kv_init(&g_kv, g_text, sizeof g_text);
    g2b_kv_str(&g_kv, "tool", "g2netbench");
    g2b_kv_str(&g_kv, "dc_ip", ip);
    g2b_kv_u64(&g_kv, "pass_bytes", PASS_BYTES);
    g2b_kv_u64(&g_kv, "pass_limit_s", PASS_LIMIT_US / 1000000ull);
    g2b_kv_u64(&g_kv, "passes", sizeof PASSES / sizeof PASSES[0]);
    send_kv(g_text);

    int failed = 0;
    for (unsigned i = 0; i < sizeof PASSES / sizeof PASSES[0] && !failed; i++) {
        snprintf(g_text, sizeof g_text, "next_pass=p%u", i + 1);
        send_line(g_text);
        failed = run_pass(i + 1, &PASSES[i]) != 0;
    }
    if (!failed) {
        static const unsigned pps[] = { 0, 2500, 1200 };
        for (unsigned i = 0; i < sizeof pps / sizeof pps[0]; i++)
            run_udp(ufd, i + 1, 4000, 1400, pps[i]);
    }
    send_line(failed ? "status=failed" : "status=done");
    uint8_t done[1] = { (uint8_t)!failed };
    send_frame(G2N_T_DONE, done, 1, NULL);
    wait_readable(g_fd, 5000);                         /* let the PC close first: our last frames are not cut off */
    printf("[g2net] done%s\n", failed ? " (a pass failed)" : "");
    close(g_fd); close(lfd); close(ufd);
    return failed;
}
