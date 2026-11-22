/* The BBA bring-up without DHCP, as steps over a small set of operations (shared by netgd-menu and netgd-t2).
   net_kos.c gives the KOS operations (eth_init, net_init in a thread, net_arp_lookup); the host tests give fakes, so the
   order, the time limits and the error texts are tested without a Dreamcast.  Every wait is bounded: nm_netup_run never
   loops without a time limit and never blocks for longer than init_ms + arp_ms (plus one poll step).
   The IP: dcload-ip's own when started from dc-tool-ip, else the static one (build option); netmask 255.255.255.0,
   broadcast x.y.z.255, gateway = the PC (dcload-ip's host, else x.y.z.1).  NOT run on hardware. */
#ifndef NM_NETUP_H
#define NM_NETUP_H
#include <stddef.h>
#include <stdint.h>

/* the results of nm_net_up (nm_net.h) */
#define NM_NET_NODEV  1         /* no BBA (no network driver registered, or net_init found no device) */
#define NM_NET_NOADDR 2         /* no IP address to use */
#define NM_NET_NOLINK 3         /* net_init did not finish in time or failed: no link (cable, PC port down) */

#define NM_NETUP_INIT_MS 30000u     /* net_init() (the driver's link-up wait included) must finish within this */
#define NM_NETUP_ARP_MS  8000u      /* the ARP of the PC (not fatal: the PC may still connect) */
#define NM_NETUP_POLL_MS 100u

typedef struct {
    void *ctx;
    uint64_t (*now_ms)(void *ctx);
    void (*sleep_ms)(void *ctx, uint32_t ms);
    /* register the drivers and put ip / mask / broadcast / gateway on every netif (so that net_init does not ask DHCP);
       the number of netifs (0 = no BBA) */
    int (*drivers)(void *ctx, uint32_t ip, uint32_t mask, uint32_t bcast, uint32_t gw);
    int (*init_start)(void *ctx);                       /* start net_init(0) in a thread: 0 / -1 */
    int (*init_done)(void *ctx, int *rc);               /* 1 = net_init returned (rc = its result), 0 = still running */
    /* after net_init: the default device exists; put the addresses on it again (the driver may have loaded the flashrom
       ISP settings) and give its name.  0 = there is a device, -1 = none */
    int (*device)(void *ctx, uint32_t ip, uint32_t mask, uint32_t bcast, uint32_t gw, const char **name);
    int (*arp)(void *ctx, uint32_t pc);                 /* one ARP lookup of the PC: 0 = answered (inserted), else not yet */
    void (*log)(void *ctx, const char *line);           /* every step (the screen); may be NULL */
} nm_netup_ops_t;

typedef struct {
    int dcload;                 /* started from dcload-ip: dcload_ip / dcload_pc are its addresses */
    uint32_t dcload_ip, dcload_pc;
    uint32_t static_ip;         /* the IP otherwise (host byte order, a.b.c.d = a << 24 | ...) */
    uint32_t init_ms, arp_ms;   /* 0 = NM_NETUP_INIT_MS / NM_NETUP_ARP_MS */
} nm_netup_cfg_t;

enum { NM_NETUP_NEW = 0, NM_NETUP_WAITING, NM_NETUP_UP, NM_NETUP_DEAD };

typedef struct {
    int stage;                  /* NM_NETUP_*: WAITING = net_init ran out of time but its thread may still finish */
    int err;                    /* the last result (0 or NM_NET_*) */
    int arp_ok;                 /* the PC answered the ARP */
    uint32_t ip, pc, mask, bcast, gw;
    char why[72];               /* the text of the last failure, for the screen ("" when up) */
} nm_netup_t;

/* the addresses of an IP on a /24 with this PC (pc = 0: x.y.z.1) */
void nm_netup_addrs(uint32_t ip, uint32_t pc, uint32_t *mask, uint32_t *bcast, uint32_t *gw, uint32_t *pc_out);
/* a.b.c.d */
void nm_netup_ipstr(uint32_t ip, char *out, size_t n);
/* Bring the network up (the first call) or report how it stands (later calls; a WAITING one is looked at again, without
   another wait: if net_init has finished meanwhile the bring-up completes).  0 or NM_NET_*; st->why has the reason. */
int nm_netup_run(nm_netup_t *st, const nm_netup_cfg_t *cfg, const nm_netup_ops_t *ops);

#endif
