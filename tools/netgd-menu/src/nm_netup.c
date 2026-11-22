/* The BBA bring-up steps: see nm_netup.h.  NOT run on hardware. */
#include <stdio.h>
#include <stdarg.h>
#include <string.h>
#include "nm_netup.h"

void nm_netup_ipstr(uint32_t ip, char *out, size_t n)
{
    snprintf(out, n, "%u.%u.%u.%u", (unsigned)(ip >> 24), (unsigned)((ip >> 16) & 255), (unsigned)((ip >> 8) & 255),
             (unsigned)(ip & 255));
}

void nm_netup_addrs(uint32_t ip, uint32_t pc, uint32_t *mask, uint32_t *bcast, uint32_t *gw, uint32_t *pc_out)
{
    if (!pc) pc = (ip & 0xffffff00u) | 1u;
    *mask = 0xffffff00u;
    *bcast = (ip & 0xffffff00u) | 0xffu;
    *gw = pc;
    *pc_out = pc;
}

static void say(const nm_netup_ops_t *ops, const char *fmt, ...)
{
    char b[80];
    va_list ap;
    if (!ops->log) return;
    va_start(ap, fmt);
    vsnprintf(b, sizeof b, fmt, ap);
    va_end(ap);
    ops->log(ops->ctx, b);
}

static int fail(nm_netup_t *st, int stage, int err, const char *why)
{
    st->stage = stage;
    st->err = err;
    snprintf(st->why, sizeof st->why, "%s", why);
    return err;
}

/* wait for net_init's thread at most ms; 1 = it returned (rc), 0 = still running */
static int wait_init(const nm_netup_ops_t *ops, uint32_t ms, int *rc, uint32_t *took)
{
    uint64_t t0 = ops->now_ms(ops->ctx);
    for (;;) {
        uint64_t el;
        if (ops->init_done(ops->ctx, rc)) { *took = (uint32_t)(ops->now_ms(ops->ctx) - t0); return 1; }
        el = ops->now_ms(ops->ctx) - t0;
        if (el >= ms) { *took = (uint32_t)el; return 0; }
        ops->sleep_ms(ops->ctx, NM_NETUP_POLL_MS);
    }
}

/* net_init has returned rc: finish (device, ARP) or fail */
static int after_init(nm_netup_t *st, const nm_netup_cfg_t *cfg, const nm_netup_ops_t *ops, int rc, uint32_t took)
{
    const char *name = "?";
    char pcs[16];
    uint32_t arp_ms = cfg->arp_ms ? cfg->arp_ms : NM_NETUP_ARP_MS;
    uint64_t t0;
    int err = -1;
    if (rc < 0) {
        /* KOS: net_init returns -1 when no device started; the BBA's start waits 10 s for the link and gives up */
        say(ops, "net_init rc=%d after %u ms: no link", rc, (unsigned)took);
        if (ops->init_again && ops->init_again(ops->ctx)) return fail(st, NM_NETUP_NEW, NM_NET_NOLINK, NM_NETUP_MSG_RETRY);
        /* KOS: a second net_init would abort (nm_netup.h, init_again), so this is final */
        return fail(st, NM_NETUP_DEAD, NM_NET_NOLINK, NM_NETUP_MSG_FINAL);
    }
    if (ops->device(ops->ctx, st->ip, st->mask, st->bcast, st->gw, &name) != 0) {
        say(ops, "net_init rc=%d, no device", rc);
        return fail(st, NM_NETUP_DEAD, NM_NET_NODEV, "BBA not found (net_init found no device)");
    }
    say(ops, "net_init ok in %u ms (%s)", (unsigned)took, name ? name : "?");
    nm_netup_ipstr(st->pc, pcs, sizeof pcs);
    t0 = ops->now_ms(ops->ctx);
    for (;;) {
        err = ops->arp(ops->ctx, st->pc);
        if (err == 0 || ops->now_ms(ops->ctx) - t0 >= arp_ms) break;
        ops->sleep_ms(ops->ctx, 50);
    }
    st->arp_ok = err == 0;
    if (st->arp_ok) say(ops, "PC %s answered ARP in %u ms", pcs, (unsigned)(ops->now_ms(ops->ctx) - t0));
    else say(ops, "no ARP answer from %s in %u s (it may still connect)", pcs, (unsigned)(arp_ms / 1000));
    st->stage = NM_NETUP_UP;
    st->err = 0;
    st->why[0] = 0;
    return 0;
}

int nm_netup_run(nm_netup_t *st, const nm_netup_cfg_t *cfg, const nm_netup_ops_t *ops)
{
    uint32_t init_ms = cfg->init_ms ? cfg->init_ms : NM_NETUP_INIT_MS, took = 0;
    char a[16], b[16];
    int rc = -1, n;

    switch (st->stage) {
    case NM_NETUP_UP:
        return 0;
    case NM_NETUP_DEAD:
        return st->err;
    case NM_NETUP_WAITING:
        /* the last call ran out of time with net_init still running: never start a second one, wait for this one */
        say(ops, "waiting for net_init again (<= %u s)", (unsigned)(init_ms / 1000));
        if (!wait_init(ops, init_ms, &rc, &took)) {
            say(ops, "net_init still running after %u s", (unsigned)(init_ms / 1000));
            return fail(st, NM_NETUP_WAITING, NM_NET_NOLINK, "Network start hangs: restart the console");
        }
        return after_init(st, cfg, ops, rc, took);
    default:
        break;
    }

    /* NEW: the first call, or a retry after net_init returned an error (no link; only when ops->init_again allowed it) */
    st->arp_ok = 0;
    st->ip = cfg->dcload ? cfg->dcload_ip : cfg->static_ip;
    if (!st->ip) return fail(st, NM_NETUP_DEAD, NM_NET_NOADDR,
                             cfg->dcload ? "No IP address (dcload-ip gave none)" : "No IP address (none built in)");
    nm_netup_addrs(st->ip, cfg->dcload ? cfg->dcload_pc : 0, &st->mask, &st->bcast, &st->gw, &st->pc);
    nm_netup_ipstr(st->ip, a, sizeof a);
    nm_netup_ipstr(st->pc, b, sizeof b);
    say(ops, "DC %s  PC %s%s", a, b, cfg->dcload ? "  (dcload-ip)" : "  (static, no DHCP)");

    n = ops->drivers(ops->ctx, st->ip, st->mask, st->bcast, st->gw);
    say(ops, "network drivers: %d", n);
    if (n <= 0) return fail(st, NM_NETUP_DEAD, NM_NET_NODEV, "BBA not found");

    say(ops, "net_init (no DHCP, <= %u s)", (unsigned)(init_ms / 1000));
    if (ops->init_start(ops->ctx) != 0) return fail(st, NM_NETUP_DEAD, NM_NET_NODEV, "BBA: cannot start net_init");
    if (!wait_init(ops, init_ms, &rc, &took)) {
        say(ops, "net_init did not return in %u s", (unsigned)(init_ms / 1000));
        return fail(st, NM_NETUP_WAITING, NM_NET_NOLINK, "No network link yet (Y: wait again)");
    }
    return after_init(st, cfg, ops, rc, took);
}
