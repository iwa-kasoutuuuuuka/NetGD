/* netgd-t2 (KOS build): start, then run the steps of the core (t2_app.c).  NOT run on hardware.
   Start it with dc-tool-ip (dcload-ip) or from a disc; the PC runs netgd-send push --host <IP> --code <code>.
   No INIT_NET (the BBA is brought up by hand, ../netgd-menu/src/net_kos.c, shared with the menu), no INIT_CDROM (the drive gets raw ATAPI packets only, and only
   through the write gate: fwxfer_kos.c via fwctl.c, as netgd-recv). */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <kos.h>
#include <dc/video.h>
#include <dc/biosfont.h>
#include "nm_plat.h"
#include "t2_app.h"
#include "nm_net.h"

KOS_INIT_FLAGS(INIT_DEFAULT & ~INIT_CDROM);

#ifndef T2_TCPFIX
#define T2_TCPFIX 0                 /* 1: the patched net_tcp.c is linked (make TCPFIX=1) */
#endif
#ifndef T2_NO_DRIVE
#define T2_NO_DRIVE 0               /* 1: never send a packet to the drive (network-only smoke test on any GDEMU) */
#endif
#ifndef T2_CODE
#define T2_CODE -1                  /* a fixed pass code (0..9999); -1 = a new one every start */
#endif

#ifndef T2_IP
#define T2_IP 0xC0A80A02u           /* 192.168.10.2 when not started from dcload-ip */
#endif

static t2_app_t app;

static void net_log(const char *line)
{
    printf("NGT2: %s\n", line);     /* reaches dc-tool-ip until KOS takes the BBA */
    t2_log(&app, line);
}

int main(int argc, char **argv)
{
    t2_cfg_t cfg;
    (void)argc; (void)argv;
    vid_set_mode(DM_640x480, PM_RGB565);
    vid_empty();
    bfont_set_background_color(0x0000);
    t2_cfg_defaults(&cfg, T2_TCPFIX);
#if defined(T2_WANT) && T2_TCPFIX      /* an explicit window, never on stock TCP (R178) */
    cfg.want_bytes = T2_WANT;
#endif
#if defined(T2_DEPTH) && T2_TCPFIX
    cfg.depth = T2_DEPTH;
#endif
#ifdef T2_RCVBUF
    nm_net_rcvbuf = T2_RCVBUF;
#endif
    cfg.drive = !T2_NO_DRIVE;
    cfg.code = T2_CODE >= 0 ? (uint16_t)(T2_CODE % 10000) : (uint16_t)(nm_plat_entropy() % 10000u);
    printf("NGT2: netgd-t2 %s port %u code %04u want %u depth %d tcpfix %d drive %d\n", T2_VERSION, (unsigned)cfg.port,
           (unsigned)cfg.code, (unsigned)cfg.want_bytes, cfg.depth, T2_TCPFIX, cfg.drive);
    fflush(stdout);
    nm_net_log = net_log;
    nm_net_static_ip = T2_IP;
    t2_init(&app, &cfg);
    while (t2_step(&app)) thd_sleep(16);
    thd_sleep(5000);                /* the result stays on the screen */
    nm_net_shutdown();              /* DC00 to dc-tool-ip when started from it */
    exit(app.state == T2_S_DONE && app.result == T2_R_FINISH ? 0 : 1);
}
