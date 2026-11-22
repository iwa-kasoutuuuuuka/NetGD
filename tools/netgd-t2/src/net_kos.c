/* netgd-t2 KOS network layer (nm_net.h, TCP only): the BBA is brought up BY HAND, never with INIT_NET / DHCP.
   With INIT_NET under dcload-ip, net_init() asks DHCP and waits up to 60 s, and fs_dclsocket_init() spins forever on
   the ARP of the PC, with the screen black (R175, first g2netbench run).  So, as in tools/g2bench/main_net.c
   (feat/g2-transport): eth_init() registers the drivers, the IP is put on every netif (so net_init(0) does not ask
   DHCP), net_init(0) runs in a thread with a time limit, the ARP of the PC is bounded, and fs_dclsocket is started
   only for the exit hand-off to dc-tool-ip.  Every step goes to t2_net_log (the screen).
   The IP: dcload-ip's own when started from dc-tool-ip, else T2_IP (build: make T2_IP=0xC0A80A02).
   The listener gets SO_RCVBUF = t2_net_rcvbuf BEFORE bind/listen (accepted sockets inherit it; set later the window
   stays at 8 KiB, R175).  NOT run on hardware; the calls are the ones g2netbench made on KOS 2.3 (R175, R178). */
#include <stdio.h>
#include <stdarg.h>
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
#include "t2_app.h"

#ifndef T2_IP
#define T2_IP 0xC0A80A02u           /* 192.168.10.2 when not started from dcload-ip */
#endif
#define NET_INIT_MS 30000           /* net_init() (link-up wait included) must finish within this */
#define ARP_MS      8000

extern void eth_init(void);         /* KOS hardware.c: registers the network drivers (only INIT_NET calls it) */
extern uint32_t _fs_dclsocket_get_ip(void);
extern void fs_dclsocket_init_console(void);

static int net_started, dcls_up;
static int lsock = -1, csock = -1;

static uint64_t now_ms(void) { return timer_ms_gettime64(); }

static void say(const char *fmt, ...)
{
    char b[80];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(b, sizeof b, fmt, ap);
    va_end(ap);
    if (t2_net_log) t2_net_log(b);
}

static void set4(uint8_t *d, uint32_t v) { d[0] = (uint8_t)(v >> 24); d[1] = (uint8_t)(v >> 16); d[2] = (uint8_t)(v >> 8); d[3] = (uint8_t)v; }

static void set_if(netif_t *nif, uint32_t ip, uint32_t gw)
{
    set4(nif->ip_addr, ip);
    set4(nif->netmask, 0xffffff00u);
    set4(nif->broadcast, (ip & 0xffffff00u) | 0xffu);
    set4(nif->gateway, gw);
}

static volatile int g_net_rc, g_net_done;
static void *netinit_thd(void *arg) { (void)arg; g_net_rc = net_init(0); g_net_done = 1; return NULL; }

int nm_net_up(char *ip, size_t n)
{
    uint32_t me, pc = 0, port = 0;
    int dcl = dcload_type == DCLOAD_TYPE_IP, nifs = 0, err = -1;
    netif_t *nif;
    uint8_t hb[4], mac[6];
    uint64_t t0;
    if (n) ip[0] = 0;
    if (net_started) {
        if (!net_default_dev) return NM_NET_NODEV;
        snprintf(ip, n, "%u.%u.%u.%u", net_default_dev->ip_addr[0], net_default_dev->ip_addr[1], net_default_dev->ip_addr[2],
                 net_default_dev->ip_addr[3]);
        return 0;
    }
    net_started = 1;
    if (dcl) {
        me = _fs_dclsocket_get_ip();
        dcload_gethostinfo(&pc, &port);
    } else {
        me = T2_IP;
    }
    if (!pc) pc = (me & 0xffffff00u) | 1u;
    if (!me) { say("no IP address (dcload gave none)"); return NM_NET_NOADDR; }
    say("DC %u.%u.%u.%u  PC %u.%u.%u.%u%s", me >> 24, (me >> 16) & 255, (me >> 8) & 255, me & 255,
        pc >> 24, (pc >> 16) & 255, (pc >> 8) & 255, pc & 255, dcl ? "  (dcload-ip)" : "");
    if (dcl) dbgio_disable();                       /* the console goes over the BBA: off once KOS owns it */

    eth_init();
    LIST_FOREACH(nif, net_get_if_list(), if_list) { set_if(nif, me, pc); nifs++; }
    say("network drivers: %d", nifs);
    if (!nifs) return NM_NET_NODEV;

    say("net_init (no DHCP, <= %d s)", NET_INIT_MS / 1000);
    t0 = now_ms();
    if (!thd_create(0, netinit_thd, NULL)) return NM_NET_NODEV;
    while (!g_net_done && now_ms() - t0 < NET_INIT_MS) thd_sleep(100);
    if (!g_net_done) { say("net_init did not return in %d s", NET_INIT_MS / 1000); return NM_NET_NODEV; }
    if (g_net_rc < 0 || !net_default_dev) { say("net_init rc=%d, no device", g_net_rc); return NM_NET_NODEV; }
    set_if(net_default_dev, me, pc);                /* the driver may have loaded the flashrom ISP settings */
    say("net_init ok in %u ms (%s)", (unsigned)(now_ms() - t0), net_default_dev->name);

    net_ipv4_parse_address(pc, hb);
    t0 = now_ms();
    while (now_ms() - t0 < ARP_MS) {
        err = net_arp_lookup(net_default_dev, hb, mac, NULL, NULL, 0);
        if (err == 0) break;
        thd_sleep(50);
    }
    if (err == 0) {
        net_arp_insert(net_default_dev, mac, hb, 0);
        say("PC answered ARP in %u ms", (unsigned)(now_ms() - t0));
        if (dcl) {
            fs_dclsocket_init_console();
            dcls_up = fs_dclsocket_init() == 0;     /* only for the exit hand-off (DC00 ends dc-tool-ip) */
        }
    } else {
        say("no ARP answer from the PC in %d s (it may still connect)", ARP_MS / 1000);
    }
    set4(hb, me);
    snprintf(ip, n, "%u.%u.%u.%u", hb[0], hb[1], hb[2], hb[3]);
    return 0;
}

void t2_net_shutdown(void)
{
    nm_net_close_client();
    nm_net_stop();
    if (dcls_up) { fs_dclsocket_shutdown(); dcls_up = 0; }
    if (net_started) net_shutdown();
    net_started = 0;
}

static void set_nonblock(int fd)
{
    int fl = fcntl(fd, F_GETFL, 0);
    if (fl >= 0) fcntl(fd, F_SETFL, fl | O_NONBLOCK);
}

int nm_net_listen(uint16_t port)
{
    struct sockaddr_in a;
    uint32_t rb = t2_net_rcvbuf;
    nm_net_stop();
    lsock = socket(PF_INET, SOCK_STREAM, 0);
    if (lsock < 0) return -1;
    if (setsockopt(lsock, SOL_SOCKET, SO_RCVBUF, &rb, sizeof rb) != 0) say("SO_RCVBUF %u refused", (unsigned)rb);
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_port = htons(port);
    a.sin_addr.s_addr = htonl(INADDR_ANY);
    if (bind(lsock, (struct sockaddr *)&a, sizeof a) != 0 || listen(lsock, 1) != 0) { nm_net_stop(); return -1; }
    set_nonblock(lsock);
    return 0;
}

int nm_net_accept(uint32_t timeout_ms)
{
    uint64_t t0 = now_ms();
    if (lsock < 0) return -1;
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
        if (now_ms() - t0 >= 15000) return -1;
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
    if (lsock >= 0) close(lsock);
    lsock = -1;
}
