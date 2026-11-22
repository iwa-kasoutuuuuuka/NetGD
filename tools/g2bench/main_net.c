/* g2netbench: phase D of docs/research/g2-perf-plan.md. Measures how fast the Dreamcast can RECEIVE the data
 * stream over KallistiOS's own TCP/IP stack on the BBA, with the NGTP frames and the checksums of netgd-recv's
 * plan (docs/design/menu-app.md 8: the Dreamcast sends WANT, the PC answers DATA, up to 64 KiB per frame), and
 * the data goes into a sink (a 64 KiB buffer) instead of the SD card.
 *
 * Needs a Dreamcast + BBA started through dcload-ip (dc_auto.ps1 + the g2netbench preset). KOS is NOT started
 * with INIT_NET: with dcload-ip, KOS's net_init(ip) first asks DHCP for that IP and waits up to 60 s (kernel/net/
 * net_core.c: net_dhcp_request, 60 * 1000 ms), and fs_dclsocket_init() spins forever on the ARP lookup of the
 * PC (fs_dclsocket.c). With dbgio disabled meanwhile (arch_init_net_dcload_ip) the screen stays black and the
 * console silent, which is what the first hardware run looked like. So the network is brought up here, by hand,
 * like tools/vmu-workbench/dc/savemgr/net.c: set the IP on the netif, net_init(0) (no DHCP when the netif has
 * an address), bounded ARP lookup of the PC, then fs_dclsocket for the exit hand-off only.
 * Progress is drawn on the screen (bfont) with a once-a-second heartbeat in row 0: the console is not used
 * once KOS owns the NIC. The PC side is g2n_send.py (started by the preset); results travel as LINE frames.
 * Never touches the GD-ROM drive: INIT_CDROM is off, no G1 / ATA / cdrom call.
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
#include <stdarg.h>
#include <dc/biosfont.h>
#include <dc/dcload.h>
#include <dc/fs_dclsocket.h>
#include "g2b_core.h"
#include "g2n_core.h"
#include "../netgd-recv/src/crc32.h"

KOS_INIT_FLAGS(INIT_DEFAULT & ~INIT_CDROM);

#define WAIT_PC_MS     600000     /* wait this long for the PC sender to connect */
#define NET_INIT_MS    30000      /* net_init() (link-up wait included) must finish within this */
#define ARP_MS         8000       /* ARP lookup of the PC */
#define FRAME_TO_MS    15000      /* inside a frame (menu-app.md 8: 15 s) */
#define DATA_TO_MS     12000      /* WANT -> DATA: report and move on (menu-app.md 8 allows 30 s) */
#define DRAIN_IDLE_MS  5000       /* after a failed pass: discard what is still in flight until this much silence */
#ifndef G2N_LISTEN_RCVBUF
#define G2N_LISTEN_RCVBUF 65535   /* receive buffer = TCP window of accepted connections (set on the listener) */
#endif
#define PASS_LIMIT_US  (20ull * 1000000ull)
#define PASS_BYTES     (32ull << 20)

static uint64_t now_us(void) { return timer_us_gettime64(); }

static volatile uint64_t g_rx_total;           /* every byte recv_all() took */
static volatile unsigned g_frames_total;
static volatile int g_conn_dead;               /* the TCP connection is gone (closed / reset) */

/* ---- on-screen log (works when the console is dead) ---- */
#define SCR_W     640
#define SCR_ROWS  19                       /* rows 1..19 of 24 px; row 0 = heartbeat */
static int g_row = 1;
static int g_con = 1;                      /* mirror to the dcload console (only before KOS owns the NIC) */
static const char *g_stage = "start";
static volatile int g_hb_run = 1;

static void scr_put(int row, const char *t)
{
    char b[56];
    snprintf(b, sizeof b, "%-52.52s", t);   /* pad: overwrites the old row text (opaque background) */
    bfont_draw_str(vram_s + (size_t)row * BFONT_HEIGHT * SCR_W + 4, SCR_W, 1, b);
}

static void say(const char *fmt, ...)
{
    char b[160];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(b, sizeof b, fmt, ap);
    va_end(ap);
    if (g_con) printf("[g2net] %s\n", b);
    if (g_row > SCR_ROWS) { memset(vram_s + BFONT_HEIGHT * SCR_W, 0, (size_t)SCR_ROWS * BFONT_HEIGHT * SCR_W * 2); g_row = 1; }
    scr_put(g_row++, b);
}

static void *heartbeat(void *arg)
{
    (void)arg;
    unsigned n = 0;
    while (g_hb_run) {
        char b[64];
        snprintf(b, sizeof b, "%c %us %s rx=%luKB fr=%u", "|/-\\"[n & 3], (unsigned)(timer_ms_gettime64() / 1000), g_stage,
                 (unsigned long)(g_rx_total / 1024), g_frames_total);
        scr_put(0, b);
        n++;
        thd_sleep(500);
    }
    return NULL;
}

static void stage(const char *s) { g_stage = s; }

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
        if (w <= 0) { if (st) st->recv_us += now_us() - t0; if (w < 0) g_conn_dead = 1; errno = w == 0 ? ETIMEDOUT : errno; return -1; }
        ssize_t r = recv(fd, buf + got, n - got, 0);
        if (st) st->recv_calls++;
        if (r <= 0) { if (st) st->recv_us += now_us() - t0; g_conn_dead = 1; if (r == 0) errno = 0; return -1; }
        got += (size_t)r;
        g_rx_total += (uint64_t)r;
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
        if (n) { send_line(line); say("%s", line); }
        text += n; if (*text == '\n') text++;
    }
}

/* After a failed pass the PC may still be sending the DATA frame we gave up on: discard it until it is quiet,
   so the next pass starts on a frame boundary. Returns the bytes thrown away. */
static uint64_t drain(int idle_ms)
{
    uint64_t n = 0;
    for (;;) {
        int w = wait_readable(g_fd, idle_ms);
        if (w < 0) { g_conn_dead = 1; break; }
        if (w == 0) break;
        ssize_t r = recv(g_fd, g_sink, sizeof g_sink, 0);
        if (r <= 0) { g_conn_dead = 1; break; }
        n += (uint64_t)r;
        g_rx_total += (uint64_t)r;
    }
    return n;
}

/* ---- one TCP pass ---- */
typedef struct { uint32_t want, depth, rcvbuf; int ck; } pass_cfg;
static const pass_cfg PASSES[] = {   /* the 64 KiB frames (the ones that hung in the first run) come last */
    { 16384, 1, 0, 1 },
    { 16384, 1, 0, 0 },       /* no checksums: the network alone */
    { 65536, 1, 0, 1 },       /* netgd-recv's plan (menu-app.md D6) */
    { 65536, 1, 0, 0 },
    { 65536, 2, 0, 1 },       /* one WANT read ahead */
    { 65536, 2, 0, 0 },
};

static volatile unsigned g_pass_no;
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

    /* SO_RCVBUF on a connected KOS socket only reallocs the buffer, the advertised window (rcv.wnd) stays at the
       value from accept(): the size is therefore set once on the listener (main) */
    uint32_t got_rb = 0; socklen_t gl = sizeof got_rb;
    if (getsockopt(g_fd, SOL_SOCKET, SO_RCVBUF, &got_rb, &gl) != 0) got_rb = 0;
    kvp(pfx, "rcvbuf", got_rb);
    g_pass_no = n;
    uint64_t rx0 = g_rx_total;

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
        g_frames_total++;
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
    kvp(pfx, "rx_bytes_total", g_rx_total - rx0);   /* incl. a frame cut off by the failure */
    kvp(pfx, "outstanding", ps.outstanding);
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
extern void eth_init(void);                /* KOS hardware.c: registers the drivers (only INIT_NET calls it) */
extern uint32_t _fs_dclsocket_get_ip(void);
extern void fs_dclsocket_init_console(void);

static void set4(uint8_t *d, uint32_t v) { d[0] = (uint8_t)(v >> 24); d[1] = (uint8_t)(v >> 16); d[2] = (uint8_t)(v >> 8); d[3] = (uint8_t)v; }

static void ip_str(char *out, size_t n)
{
    snprintf(out, n, "?.?.?.?");
    if (net_default_dev)
        snprintf(out, n, "%u.%u.%u.%u", net_default_dev->ip_addr[0], net_default_dev->ip_addr[1],
                 net_default_dev->ip_addr[2], net_default_dev->ip_addr[3]);
}

static int g_net_rc, g_net_done;
static void *netinit_thd(void *arg) { (void)arg; g_net_rc = net_init(0); g_net_done = 1; return NULL; }

static int g_dcls;                         /* fs_dclsocket is up: shut it down at the end (sends DC00) */

/* Bring the BBA up under KOS. 0 = net_default_dev usable. Every step is on the screen. */
static int bring_up(void)
{
    uint32_t ip = 0, hip = 0, hport = 0;
    int dcl = dcload_type == DCLOAD_TYPE_IP;
    if (dcl) {
        ip = _fs_dclsocket_get_ip();                 /* our IP as dcload-ip has it */
        dcload_gethostinfo(&hip, &hport);            /* dc-tool-ip's IP and port */
    } else {
        ip = (192u << 24) | (168u << 16) | (10u << 8) | 2u;
        hip = (ip & 0xffffff00u) | 1u;
    }
    say("dcload type %d  DC %u.%u.%u.%u  PC %u.%u.%u.%u", dcload_type, ip >> 24, (ip >> 16) & 255, (ip >> 8) & 255, ip & 255,
        hip >> 24, (hip >> 16) & 255, (hip >> 8) & 255, hip & 255);
    if (ip == 0) { say("no IP from dcload: set one by hand"); return -1; }
    say("console off from here (KOS takes the BBA)");
    g_con = 0;
    if (dcl) dbgio_disable();

    stage("eth_init");
    eth_init();
    netif_t *nif;
    int nifs = 0;
    LIST_FOREACH(nif, net_get_if_list(), if_list) {
        set4(nif->ip_addr, ip); set4(nif->netmask, 0xffffff00u);
        set4(nif->broadcast, (ip & 0xffffff00u) | 0xffu); set4(nif->gateway, hip);
        nifs++;
    }
    say("netifs registered: %d", nifs);
    if (!nifs) return -1;

    stage("net_init (no DHCP)");
    say("net_init ... (link up wait, <= %d s)", NET_INIT_MS / 1000);
    uint64_t t0 = timer_ms_gettime64();
    kthread_t *th = thd_create(0, netinit_thd, NULL);
    while (th && !g_net_done && timer_ms_gettime64() - t0 < NET_INIT_MS) thd_sleep(100);
    if (!th || !g_net_done) { say("net_init STUCK after %d s (BBA / link?)", NET_INIT_MS / 1000); return -1; }
    say("net_init rc=%d in %u ms, default dev %s", g_net_rc, (unsigned)(timer_ms_gettime64() - t0),
        net_default_dev ? net_default_dev->name : "NONE");
    if (g_net_rc < 0 || !net_default_dev) return -1;

    /* the driver may have loaded the flashrom ISP settings: force ours */
    set4(net_default_dev->ip_addr, ip); set4(net_default_dev->netmask, 0xffffff00u);
    set4(net_default_dev->broadcast, (ip & 0xffffff00u) | 0xffu); set4(net_default_dev->gateway, hip);
    {
        const uint8_t *m = net_default_dev->mac_addr;
        say("MAC %02x:%02x:%02x:%02x:%02x:%02x", m[0], m[1], m[2], m[3], m[4], m[5]);
    }

    /* ARP of the PC, bounded (fs_dclsocket_init() would spin forever on a missing reply) */
    stage("ARP of the PC");
    uint8_t hb[4], mac[6];
    net_ipv4_parse_address(hip, hb);
    int err = -1;
    t0 = timer_ms_gettime64();
    while (timer_ms_gettime64() - t0 < ARP_MS) {
        err = net_arp_lookup(net_default_dev, hb, mac, NULL, NULL, 0);
        if (err == 0) break;
        thd_sleep(50);
    }
    if (err == 0) {
        say("PC MAC %02x:%02x:%02x:%02x:%02x:%02x (%u ms)", mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
            (unsigned)(timer_ms_gettime64() - t0));
        net_arp_insert(net_default_dev, mac, hb, 0);
        if (dcl) {
            fs_dclsocket_init_console();
            g_dcls = fs_dclsocket_init() == 0;         /* console stays OFF (dbgio not re-selected): only DC00 at the end */
            say("fs_dclsocket (exit hand-off): %s", g_dcls ? "up" : "failed");
        }
    } else {
        say("PC did not answer ARP in %d s: no route to %u.%u.%u.%u?", ARP_MS / 1000, hb[0], hb[1], hb[2], hb[3]);
        say("(continuing: the PC may reach us)");
    }
    return 0;
}

static void shutdown_net(void)
{
    if (g_dcls) { fs_dclsocket_shutdown(); g_dcls = 0; }   /* DC00: dc-tool-ip ends, dc_auto sees the run finished */
    net_shutdown();
}

static int fail(const char *why)
{
    say("FAILED: %s", why);
    stage("failed, returning in 20 s");
    thd_sleep(20000);
    g_hb_run = 0;
    return 1;
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
    bfont_set_encoding(BFONT_CODE_ISO8859_1);
    bfont_set_foreground_color(0xffff);
    bfont_set_background_color(0x0000);
    memset(vram_s, 0, 640 * 480 * 2);
    thd_create(0, heartbeat, NULL);
    say("g2netbench: no GD-ROM access (INIT_CDROM off)");

    if (bring_up() != 0) { int r = fail("network bring-up"); shutdown_net(); return r; }
    char ip[24];
    ip_str(ip, sizeof ip);
    say("IP %s  TCP %u  UDP %u", ip, G2N_TCP_PORT, G2N_UDP_PORT);

    int lfd = socket(AF_INET, SOCK_STREAM, 0);
    int ufd = socket(AF_INET, SOCK_DGRAM, 0);
    if (lfd < 0 || ufd < 0) { say("socket() failed errno=%d", errno); int r = fail("socket"); shutdown_net(); return r; }
    {   /* accepted connections inherit it, window included (before bind / listen: the socket is plain then) */
        uint32_t rb = G2N_LISTEN_RCVBUF;
        int rc = setsockopt(lfd, SOL_SOCKET, SO_RCVBUF, &rb, sizeof rb);
        say("listener SO_RCVBUF %u: %s", (unsigned)rb, rc == 0 ? "ok" : "failed");
    }
    struct sockaddr_in a; memset(&a, 0, sizeof a);
    a.sin_family = AF_INET; a.sin_addr.s_addr = htonl(INADDR_ANY);
    a.sin_port = htons(G2N_TCP_PORT);
    if (bind(lfd, (struct sockaddr *)&a, sizeof a) != 0 || listen(lfd, 1) != 0) {
        say("TCP bind/listen failed errno=%d", errno); int r = fail("listen"); shutdown_net(); return r;
    }
    a.sin_port = htons(G2N_UDP_PORT);
    if (bind(ufd, (struct sockaddr *)&a, sizeof a) != 0) { say("UDP bind failed errno=%d", errno); int r = fail("udp bind"); shutdown_net(); return r; }
    fcntl(ufd, F_SETFL, fcntl(ufd, F_GETFL, 0) | O_NONBLOCK);
    say("listening on %s:%u; waiting for the PC sender", ip, G2N_TCP_PORT);
    stage("waiting for the PC sender");

    if (wait_readable(lfd, WAIT_PC_MS) <= 0) { int r = fail("no PC connected"); shutdown_net(); return r; }
    struct sockaddr_in peer; socklen_t pl = sizeof peer;
    g_fd = accept(lfd, (struct sockaddr *)&peer, &pl);
    if (g_fd < 0) { say("accept failed errno=%d", errno); int r = fail("accept"); shutdown_net(); return r; }
    say("PC connected");
    stage("handshake");

    g2n_hdr h;
    if (read_request(G2N_T_HELLO, 256, &h) != 0) { int r = fail("bad HELLO"); shutdown_net(); return r; }
    uint8_t ok[44]; memset(ok, 0, sizeof ok);
    ok[0] = 1;                                         /* version */
    ok[4] = (uint8_t)G2N_MAX_DATA; ok[5] = (uint8_t)(G2N_MAX_DATA >> 8); ok[6] = (uint8_t)(G2N_MAX_DATA >> 16);
    memcpy(ok + 34, "g2netbench", 10);
    send_frame(G2N_T_HELLO_OK, ok, sizeof ok, NULL);

    /* PUT starts the measurement (its manifest is ignored) */
    if (read_request(G2N_T_PUT, 4096, &h) != 0) { int r = fail("no PUT"); shutdown_net(); return r; }
    say("PUT received: measuring");
    g2b_kv_init(&g_kv, g_text, sizeof g_text);
    g2b_kv_str(&g_kv, "tool", "g2netbench");
    g2b_kv_str(&g_kv, "dc_ip", ip);
    g2b_kv_u64(&g_kv, "pass_bytes", PASS_BYTES);
    g2b_kv_u64(&g_kv, "pass_limit_s", PASS_LIMIT_US / 1000000ull);
    g2b_kv_u64(&g_kv, "passes", sizeof PASSES / sizeof PASSES[0]);
    send_kv(g_text);

    int failed = 0, nfail = 0;
    for (unsigned i = 0; i < sizeof PASSES / sizeof PASSES[0] && !failed; i++) {
        char nm[32];
        snprintf(nm, sizeof nm, "TCP pass %u", i + 1);
        stage(nm);
        snprintf(g_text, sizeof g_text, "next_pass=p%u", i + 1);
        send_line(g_text);
        if (run_pass(i + 1, &PASSES[i]) != 0) {
            nfail++;
            stage("draining");
            uint64_t d = g_conn_dead ? 0 : drain(DRAIN_IDLE_MS);
            snprintf(g_text, sizeof g_text, "p%u_drained_bytes=%lu", i + 1, (unsigned long)d);
            send_line(g_text);
            say("p%u failed; drained %lu bytes%s", i + 1, (unsigned long)d, g_conn_dead ? " (connection lost)" : "");
            if (g_conn_dead) { failed = 1; break; }
        }
    }
    if (!failed) {
        static const unsigned pps[] = { 0, 2500, 1200 };
        for (unsigned i = 0; i < sizeof pps / sizeof pps[0]; i++) {
            char nm[32];
            snprintf(nm, sizeof nm, "UDP burst %u", i + 1);
            stage(nm);
            run_udp(ufd, i + 1, 4000, 1400, pps[i]);
        }
    }
    snprintf(g_text, sizeof g_text, "passes_failed=%d", nfail);
    send_line(g_text);
    send_line(failed ? "status=failed" : "status=done");
    uint8_t done[1] = { (uint8_t)!failed };
    send_frame(G2N_T_DONE, done, 1, NULL);
    wait_readable(g_fd, 5000);                         /* let the PC close first: our last frames are not cut off */
    say("done%s", failed ? " (connection lost)" : (nfail ? " (some passes failed)" : ""));
    stage(failed ? "done (failed)" : "done");
    close(g_fd); close(lfd); close(ufd);
    thd_sleep(3000);                                   /* the result is readable on the screen */
    g_hb_run = 0;
    shutdown_net();
    return failed;
}
