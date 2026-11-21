/* nm_net.h for the host tests: a real TCP listener on 127.0.0.1 (Winsock on Windows, BSD sockets elsewhere), so that the
   NGTP server is tested with the same byte stream a PC sends.  Also the real clock the serve mode uses. */
#include <stdio.h>
#include <string.h>
#include <stdint.h>
#ifdef _WIN32
#include <winsock2.h>
#include <ws2tcpip.h>
typedef SOCKET sock_t;
#define BAD_SOCK INVALID_SOCKET
#define CLOSESOCK closesocket
#else
#include <sys/socket.h>
#include <sys/select.h>
#include <sys/time.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <arpa/inet.h>
#include <unistd.h>
#include <signal.h>
#include <time.h>
typedef int sock_t;
#define BAD_SOCK (-1)
#define CLOSESOCK close
#endif
#include "nm_net.h"
#include "plat_host.h"

uint16_t nm_host_net_port;              /* listen here instead of the port asked for (0 = as asked) */
int nm_host_net_nodev;                  /* nm_net_up answers "no BBA" */
int nm_host_no_udp;                     /* nm_net_udp_listen fails */

static sock_t lsock = BAD_SOCK, csock = BAD_SOCK, usock = BAD_SOCK;
static struct sockaddr_in udp_peer;
static int udp_have_peer;

uint64_t nm_host_real_ms(void)
{
#ifdef _WIN32
    return (uint64_t)GetTickCount64();
#else
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000u + (uint64_t)ts.tv_nsec / 1000000u;
#endif
}

void nm_host_real_sleep(uint32_t ms)
{
#ifdef _WIN32
    Sleep(ms);
#else
    struct timespec ts = { ms / 1000, (long)(ms % 1000) * 1000000L };
    nanosleep(&ts, NULL);
#endif
}

int nm_net_up(char *ip, size_t n)
{
    static int started;
    if (nm_host_net_nodev) { if (n) ip[0] = 0; return NM_NET_NODEV; }
    if (!started) {
#ifdef _WIN32
        WSADATA wd;
        if (WSAStartup(MAKEWORD(2, 2), &wd)) return NM_NET_NODEV;
#else
        signal(SIGPIPE, SIG_IGN);
#endif
        started = 1;
    }
    snprintf(ip, n, "127.0.0.1");
    return 0;
}

int nm_net_listen(uint16_t port)
{
    struct sockaddr_in a;
    int one = 1;
    nm_net_stop();
    lsock = socket(AF_INET, SOCK_STREAM, 0);
    if (lsock == BAD_SOCK) return -1;
    setsockopt(lsock, SOL_SOCKET, SO_REUSEADDR, (const char *)&one, sizeof one);
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    a.sin_port = htons(nm_host_net_port ? nm_host_net_port : port);
    if (bind(lsock, (struct sockaddr *)&a, sizeof a) || listen(lsock, 2)) { nm_net_stop(); return -1; }
    return 0;
}

/* 1 when `s` is readable (or closed) within ms */
static int wait_readable(sock_t s, uint32_t ms)
{
    fd_set fds;
    struct timeval tv;
    FD_ZERO(&fds);
    FD_SET(s, &fds);
    tv.tv_sec = (long)(ms / 1000);
    tv.tv_usec = (long)(ms % 1000) * 1000;
    return select((int)s + 1, &fds, NULL, NULL, &tv) > 0;
}

int nm_net_accept(uint32_t timeout_ms)
{
    int one = 1;
    if (lsock == BAD_SOCK) return -1;
    if (!wait_readable(lsock, timeout_ms)) return 0;
    if (csock != BAD_SOCK) nm_net_close_client();
    csock = accept(lsock, NULL, NULL);
    if (csock == BAD_SOCK) return -1;
    setsockopt(csock, IPPROTO_TCP, TCP_NODELAY, (const char *)&one, sizeof one);
    return 1;
}

int nm_net_recv_all(void *buf, uint32_t n, uint32_t timeout_ms)
{
    uint8_t *p = (uint8_t *)buf;
    uint32_t got = 0;
    if (csock == BAD_SOCK) return -1;
    while (got < n) {
        int r;
        /* the first byte waits timeout_ms; once data has started, the rest gets at least 5 s (a header may arrive in pieces) */
        if (!wait_readable(csock, got ? (timeout_ms > 5000 ? timeout_ms : 5000) : timeout_ms)) return got ? -1 : NM_NET_AGAIN;
        r = recv(csock, (char *)p + got, (int)(n - got), 0);
        if (r <= 0) return -1;
        got += (uint32_t)r;
    }
    return 0;
}

int nm_net_send_all(const void *buf, uint32_t n)
{
    const uint8_t *p = (const uint8_t *)buf;
    uint32_t sent = 0;
    if (csock == BAD_SOCK) return -1;
    while (sent < n) {
#ifdef _WIN32
        int r = send(csock, (const char *)p + sent, (int)(n - sent), 0);
#else
        int r = (int)send(csock, p + sent, n - sent, MSG_NOSIGNAL);
#endif
        if (r <= 0) return -1;
        sent += (uint32_t)r;
    }
    return 0;
}

void nm_net_close_client(void)
{
    if (csock != BAD_SOCK) CLOSESOCK(csock);
    csock = BAD_SOCK;
}

void nm_net_stop(void)
{
    if (lsock != BAD_SOCK) CLOSESOCK(lsock);
    lsock = BAD_SOCK;
    if (usock != BAD_SOCK) CLOSESOCK(usock);
    usock = BAD_SOCK;
}

/* ---- UDP discovery: a datagram socket on 127.0.0.1 (the same port number as the TCP one) ---- */

int nm_net_udp_listen(uint16_t port)
{
    struct sockaddr_in a;
    if (usock != BAD_SOCK) CLOSESOCK(usock);
    usock = BAD_SOCK;
    udp_have_peer = 0;
    if (nm_host_no_udp) return -1;
    usock = socket(AF_INET, SOCK_DGRAM, 0);
    if (usock == BAD_SOCK) return -1;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    a.sin_port = htons(nm_host_net_port ? nm_host_net_port : port);
    if (bind(usock, (struct sockaddr *)&a, sizeof a)) { CLOSESOCK(usock); usock = BAD_SOCK; return -1; }
    return 0;
}

int nm_net_udp_recv(void *buf, size_t cap)
{
    int r;
#ifdef _WIN32
    int l = (int)sizeof udp_peer;
#else
    socklen_t l = sizeof udp_peer;
#endif
    if (usock == BAD_SOCK) return -1;
    if (!wait_readable(usock, 0)) return 0;
    r = (int)recvfrom(usock, (char *)buf, (int)cap, 0, (struct sockaddr *)&udp_peer, &l);
    if (r < 0) return 0;            /* Windows reports an ICMP "port unreachable" of an earlier reply as an error here */
    udp_have_peer = 1;
    return r;
}

int nm_net_udp_reply(const void *buf, size_t n)
{
    if (usock == BAD_SOCK || !udp_have_peer) return -1;
    return sendto(usock, (const char *)buf, (int)n, 0, (struct sockaddr *)&udp_peer, (int)sizeof udp_peer) == (int)n ? 0 : -1;
}
