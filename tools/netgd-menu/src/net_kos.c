/* KOS network layer (nm_net.h): the BBA is started on demand (design D12), one TCP listener, one client.
   Thin on purpose: no decisions here.  Every KOS call whose name or signature is not confirmed against the KOS 2.3.0
   headers is marked KOS-API-UNVERIFIED (design 10, K2 to K4); the CI build (kos.yaml) is the first check, the second is a
   real console (design 14, H6/H7).  NOT run on hardware. */
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <kos.h>
#include <kos/net.h>                /* KOS-API-UNVERIFIED: net_init, net_default_dev (K2, K4) */
#include "nm_net.h"

static int net_started;
static int lsock = -1, csock = -1, usock = -1;

static uint64_t now_ms(void) { return timer_ms_gettime64(); }

int nm_net_up(char *ip, size_t n)
{
    const uint8_t *a;
    if (n) ip[0] = 0;
    if (!net_started) {
        /* KOS-API-UNVERIFIED (K2): net_init(uint32 ip) in KOS 2.x (0 = DHCP); some versions take no argument.  Called through
           an unprototyped pointer so that either compiles; the extra argument is harmless when there is none.  Whether it may
           be called after start-up without INIT_NET, and how long it takes without a BBA, is H7. */
        int (*init_fn)() = (int (*)())net_init;
        init_fn(0);
        net_started = 1;
    }
    if (!net_default_dev) return NM_NET_NODEV;                 /* KOS-API-UNVERIFIED (K4) */
    a = net_default_dev->ip_addr;                              /* KOS-API-UNVERIFIED (K4): uint8 ip_addr[4] */
    if (!(a[0] | a[1] | a[2] | a[3])) return NM_NET_NOADDR;
    snprintf(ip, n, "%u.%u.%u.%u", a[0], a[1], a[2], a[3]);
    return 0;
}

static void set_nonblock(int fd)
{
    int fl = fcntl(fd, F_GETFL, 0);                            /* KOS-API-UNVERIFIED (K3): O_NONBLOCK on KOS sockets */
    if (fl >= 0) fcntl(fd, F_SETFL, fl | O_NONBLOCK);
}

int nm_net_listen(uint16_t port)
{
    struct sockaddr_in a;
    nm_net_stop();
    lsock = socket(PF_INET, SOCK_STREAM, 0);                   /* KOS-API-UNVERIFIED (K3): TCP listen/accept in KOS 2.3.0 */
    if (lsock < 0) return -1;
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
    if (lsock >= 0) close(lsock);
    lsock = -1;
    if (usock >= 0) close(usock);
    usock = -1;
}

/* ---- UDP discovery (step 9).  KOS-API-UNVERIFIED (K3): SOCK_DGRAM, recvfrom/sendto and O_NONBLOCK on a datagram socket
   in KOS 2.3.0; a failure only means that the PC has to be given the IP address by hand. ---- */

static struct sockaddr_in udp_peer;
static int udp_have_peer;

int nm_net_udp_listen(uint16_t port)
{
    struct sockaddr_in a;
    if (usock >= 0) { close(usock); usock = -1; }
    udp_have_peer = 0;
    usock = socket(PF_INET, SOCK_DGRAM, 0);                    /* KOS-API-UNVERIFIED (K3) */
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
    socklen_t l = sizeof udp_peer;                              /* KOS-API-UNVERIFIED (K3): socklen_t */
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
