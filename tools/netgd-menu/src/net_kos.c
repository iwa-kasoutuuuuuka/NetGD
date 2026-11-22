/* KOS network layer (nm_net.h), shared by netgd-menu and netgd-t2 (t2 compiles it as menu_net_kos.o): the BBA is brought
   up BY HAND, never with INIT_NET / DHCP.  net_init(0) with DHCP waits for an answer that never comes on a direct PC -
   Dreamcast cable, and with INIT_NET under dcload-ip net_init() asks DHCP for up to 60 s while fs_dclsocket_init() spins
   forever on the ARP of the PC, the screen black (R175).  So, as tools/g2bench/main_net.c (feat/g2-transport): eth_init()
   registers the drivers, the IP is put on every netif (so net_init(0) does not ask DHCP), net_init(0) runs in a thread with
   a time limit, the ARP of the PC is bounded, and fs_dclsocket is started only for the exit hand-off to dc-tool-ip.  The
   steps and their limits are nm_netup.c (host-tested); this file only gives the KOS calls.  Every step goes to nm_net_log.
   The IP: dcload-ip's own when started from dc-tool-ip, else nm_net_static_ip (NM_NET_STATIC_IP; netgd-t2: T2_IP).
   The listener gets SO_RCVBUF = nm_net_rcvbuf BEFORE bind/listen (accepted sockets inherit it; set later the window stays
   at 8 KiB, R175).  KOS 2.3 net_tcp.c never frees that buffer of a listening socket (net_tcp_close, TCP_STATE_LISTEN, frees
   only listen.queue; data.rcvbuf is a separate union member it does not touch), so the listener is made once per port and
   kept, "parked", by nm_net_stop until nm_net_shutdown (see nm_net_listen).  A net_init that returned an error (no link) is
   final: KOS 2.3 aborts on a second one (nm_netup.h, init_again).  NOT run on hardware; the calls are the ones g2netbench
   made on KOS 2.3 (R175, R178). */
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <kos.h>
#include <kos/net.h>
#include <dc/dcload.h>
#include <dc/fs_dclsocket.h>
#include "nm_net.h"
#include "nm_netup.h"

extern void eth_init(void);         /* KOS hardware.c: registers the network drivers (only INIT_NET calls it) */
extern uint32_t _fs_dclsocket_get_ip(void);
extern void fs_dclsocket_init_console(void);

void (*nm_net_log)(const char *line);
uint32_t nm_net_rcvbuf = NM_NET_RCVBUF_DEFAULT;
uint32_t nm_net_static_ip = NM_NET_STATIC_IP;
uint32_t nm_net_arp_ms;

static nm_netup_t up;
static int dcls_up, eth_done, dbgio_off;
static int lsock = -1, csock = -1, usock = -1;
static uint16_t lport;          /* the port and SO_RCVBUF lsock was made with */
static uint32_t lrcvbuf;
static int lparked;             /* lsock stays listening between nm_net_stop and the next nm_net_listen (no accept) */

static uint64_t now_ms(void) { return timer_ms_gettime64(); }

/* ---- the operations of nm_netup_run ---- */

static void set4(uint8_t *d, uint32_t v) { d[0] = (uint8_t)(v >> 24); d[1] = (uint8_t)(v >> 16); d[2] = (uint8_t)(v >> 8); d[3] = (uint8_t)v; }

static void set_if(netif_t *nif, uint32_t ip, uint32_t mask, uint32_t bcast, uint32_t gw)
{
    set4(nif->ip_addr, ip);
    set4(nif->netmask, mask);
    set4(nif->broadcast, bcast);
    set4(nif->gateway, gw);
}

static volatile int g_net_rc, g_net_done, g_net_running;
static void *netinit_thd(void *arg) { (void)arg; g_net_rc = net_init(0); g_net_done = 1; g_net_running = 0; return NULL; }

static uint64_t op_now(void *c) { (void)c; return now_ms(); }
static void op_sleep(void *c, uint32_t ms) { (void)c; thd_sleep((int)ms); }

static int op_drivers(void *c, uint32_t ip, uint32_t mask, uint32_t bcast, uint32_t gw)
{
    netif_t *nif;
    int n = 0;
    (void)c;
    if (!eth_done) { eth_init(); eth_done = 1; }    /* once: a second bba_init() would set the registered netif up again */
    LIST_FOREACH(nif, net_get_if_list(), if_list) { set_if(nif, ip, mask, bcast, gw); n++; }
    return n;
}

static int op_init_start(void *c)
{
    (void)c;
    if (g_net_running) return 0;                    /* never two net_init threads */
    g_net_done = 0;
    g_net_running = 1;
    /* detached: nothing joins it (it may outlive a timed-out wait); it only sets the flags above */
    if (!thd_create(1, netinit_thd, NULL)) { g_net_running = 0; return -1; }
    return 0;
}

static int op_init_done(void *c, int *rc) { (void)c; if (!g_net_done) return 0; *rc = g_net_rc; return 1; }

static int op_device(void *c, uint32_t ip, uint32_t mask, uint32_t bcast, uint32_t gw, const char **name)
{
    (void)c;
    if (!net_default_dev) return -1;
    set_if(net_default_dev, ip, mask, bcast, gw);   /* the driver may have loaded the flashrom ISP settings */
    *name = net_default_dev->name;
    return 0;
}

static int op_arp(void *c, uint32_t pc)
{
    uint8_t hb[4], mac[6];
    (void)c;
    net_ipv4_parse_address(pc, hb);
    if (net_arp_lookup(net_default_dev, hb, mac, NULL, NULL, 0) != 0) return -1;
    net_arp_insert(net_default_dev, mac, hb, 0);
    return 0;
}

static void op_log(void *c, const char *line) { (void)c; if (nm_net_log) nm_net_log(line); }

int nm_net_up(char *ip, size_t n)
{
    /* init_again NULL: no second net_init after one that returned an error (KOS 2.3 asserts, nm_netup.h) */
    static const nm_netup_ops_t ops = { NULL, op_now, op_sleep, op_drivers, op_init_start, op_init_done, op_device, op_arp, op_log,
                                        NULL };
    nm_netup_cfg_t cfg;
    uint32_t port = 0;
    int was_up = up.stage == NM_NETUP_UP, r;
    if (n) ip[0] = 0;
    memset(&cfg, 0, sizeof cfg);
    cfg.dcload = dcload_type == DCLOAD_TYPE_IP;
    if (cfg.dcload) {
        cfg.dcload_ip = _fs_dclsocket_get_ip();
        dcload_gethostinfo(&cfg.dcload_pc, &port);
    }
    cfg.static_ip = nm_net_static_ip;
    cfg.arp_ms = nm_net_arp_ms;
    if (cfg.dcload && cfg.dcload_ip && !dbgio_off) {
        dbgio_disable();                            /* the console goes over the BBA: off once KOS owns it */
        dbgio_off = 1;
    }
    r = nm_netup_run(&up, &cfg, &ops);
    if (r != 0) return r;
    if (!was_up && cfg.dcload && up.arp_ok && !dcls_up) {
        fs_dclsocket_init_console();
        dcls_up = fs_dclsocket_init() == 0;         /* only for the exit hand-off (DC00 ends dc-tool-ip) */
    }
    nm_netup_ipstr(up.ip, ip, n);
    return 0;
}

const char *nm_net_error(void) { return up.err ? up.why : ""; }

int nm_net_final(void) { return up.stage == NM_NETUP_DEAD; }

static void close_listener(void)
{
    if (lsock >= 0) close(lsock);
    lsock = -1;
    lparked = 0;
}

void nm_net_shutdown(void)
{
    nm_net_close_client();
    nm_net_stop();
    close_listener();
    if (dcls_up) { fs_dclsocket_shutdown(); dcls_up = 0; }
    /* never while the net_init() thread may still run (it timed out): KOS would tear the stack down under it */
    if (up.stage == NM_NETUP_UP && !g_net_running) net_shutdown();
    memset(&up, 0, sizeof up);
}

static void set_nonblock(int fd)
{
    int fl = fcntl(fd, F_GETFL, 0);
    if (fl >= 0) fcntl(fd, F_SETFL, fl | O_NONBLOCK);
}

int nm_net_listen(uint16_t port)
{
    struct sockaddr_in a;
    uint32_t rb = nm_net_rcvbuf;
    char l[48];
    nm_net_stop();
    if (lsock >= 0 && lport == port && lrcvbuf == rb) {
        /* the parked listener again (no new SO_RCVBUF buffer to lose): drop what connected while it was parked - SYNs queued
           without an answer, from a PC that has given up by now (lsock is non-blocking: this ends when the queue is empty) */
        for (;;) {
            int c = accept(lsock, NULL, NULL);
            if (c < 0) break;
            close(c);
        }
        lparked = 0;
        return 0;
    }
    close_listener();                               /* another port or window (not done by the menu or netgd-t2) */
    lsock = socket(PF_INET, SOCK_STREAM, 0);
    if (lsock < 0) return -1;
    if (rb && setsockopt(lsock, SOL_SOCKET, SO_RCVBUF, &rb, sizeof rb) != 0 && nm_net_log) {
        snprintf(l, sizeof l, "SO_RCVBUF %u refused", (unsigned)rb);
        nm_net_log(l);
    }
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_port = htons(port);
    a.sin_addr.s_addr = htonl(INADDR_ANY);
    if (bind(lsock, (struct sockaddr *)&a, sizeof a) != 0 || listen(lsock, 1) != 0) { close_listener(); return -1; }
    set_nonblock(lsock);
    lport = port;
    lrcvbuf = rb;
    lparked = 0;
    return 0;
}

int nm_net_accept(uint32_t timeout_ms)
{
    uint64_t t0 = now_ms();
    if (lsock < 0 || lparked) return -1;
    for (;;) {
        int c = accept(lsock, NULL, NULL);
        if (c >= 0) {
            if (csock >= 0) nm_net_close_client();
            csock = c;
            set_nonblock(csock);
            return 1;
        }
        if (errno != EAGAIN && errno != EWOULDBLOCK) return -1;
        if (now_ms() - t0 >= timeout_ms) return 0;
        thd_sleep(2);
    }
}

int nm_net_recv_all(void *buf, uint32_t n, uint32_t timeout_ms)
{
    uint8_t *p = (uint8_t *)buf;
    uint32_t got = 0;
    uint64_t t0 = now_ms();
    if (csock < 0) return -1;
    while (got < n) {
        int r = recv(csock, p + got, n - got, 0);
        if (r > 0) { got += (uint32_t)r; t0 = now_ms(); continue; }
        if (r == 0) return -1;                                      /* the peer closed */
        if (errno != EAGAIN && errno != EWOULDBLOCK) return -1;
        /* the first byte waits timeout_ms; once data has started the rest gets at least 5 s (nm_net.h) */
        if (now_ms() - t0 >= (got ? (timeout_ms > 5000 ? timeout_ms : 5000) : timeout_ms)) return got ? -1 : NM_NET_AGAIN;
        thd_sleep(1);
    }
    return 0;
}

int nm_net_send_all(const void *buf, uint32_t n)
{
    const uint8_t *p = (const uint8_t *)buf;
    uint32_t sent = 0;
    uint64_t t0 = now_ms();
    if (csock < 0) return -1;
    while (sent < n) {
        int r = send(csock, p + sent, n - sent, 0);
        if (r > 0) { sent += (uint32_t)r; t0 = now_ms(); continue; }
        if (r == 0 || (errno != EAGAIN && errno != EWOULDBLOCK)) return -1;
        if (now_ms() - t0 >= 15000) return -1;                      /* the PC does not read */
        thd_sleep(1);
    }
    return 0;
}

void nm_net_close_client(void)
{
    if (csock >= 0) close(csock);
    csock = -1;
}

void nm_net_stop(void)
{
    if (lsock >= 0) lparked = 1;                    /* not closed: its SO_RCVBUF buffer would be lost (see the top) */
    if (usock >= 0) close(usock);
    usock = -1;
}

/* ---- UDP discovery (the menu's step 9; netgd-t2 does not use it).  A failure only means that the PC has to be given the
   IP address by hand. ---- */

static struct sockaddr_in udp_peer;
static int udp_have_peer;

int nm_net_udp_listen(uint16_t port)
{
    struct sockaddr_in a;
    if (usock >= 0) { close(usock); usock = -1; }
    udp_have_peer = 0;
    usock = socket(PF_INET, SOCK_DGRAM, 0);
    if (usock < 0) return -1;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_port = htons(port);
    a.sin_addr.s_addr = htonl(INADDR_ANY);
    if (bind(usock, (struct sockaddr *)&a, sizeof a) != 0) { close(usock); usock = -1; return -1; }
    set_nonblock(usock);
    return 0;
}

int nm_net_udp_recv(void *buf, size_t cap)
{
    socklen_t l = sizeof udp_peer;
    int r;
    if (usock < 0) return -1;
    r = (int)recvfrom(usock, buf, cap, 0, (struct sockaddr *)&udp_peer, &l);
    if (r > 0) { udp_have_peer = 1; return r; }
    if (r < 0 && errno != EAGAIN && errno != EWOULDBLOCK) return -1;
    return 0;
}

int nm_net_udp_reply(const void *buf, size_t n)
{
    if (usock < 0 || !udp_have_peer) return -1;
    return sendto(usock, buf, n, 0, (struct sockaddr *)&udp_peer, sizeof udp_peer) == (int)n ? 0 : -1;
}
