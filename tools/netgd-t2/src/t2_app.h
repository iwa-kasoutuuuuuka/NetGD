/* netgd-t2: the standalone T2 receiver (NGTP over KOS TCP, docs/design/menu-app.md 8, bba-sd-write.md T2).
   The PC (tools/netgd-send push / remote) sends files; the receive core of netgd-recv writes them to the NetGD SD
   card through the firmware's write session, exactly as T1 does.  This is the host-testable core: the state machine,
   the screen and the write gate.  The KOS layer (main_kos.c, ../netgd-menu/src/net_kos.c) only draws, reads the pad and talks to the BBA.
   NOT run on hardware.  NetGD is not on the hardware yet: the write gate refuses every drive that does not identify
   as NetGD (VERSION "OGDM" with FW_CAP_SESSION and FW_CAP_CHECKSUM), and the build T2_NO_DRIVE=1 never talks to the
   drive at all. */
#ifndef T2_APP_H
#define T2_APP_H
#include <stdint.h>
#include "nm_screen.h"
#include "ngtp_srv.h"
#include "ng_btnguard.h"

#define T2_VERSION "0.1.0"

typedef struct {
    uint16_t port;                  /* NGTP_PORT */
    uint16_t code;                  /* pass code the PC must send (0..9999) */
    uint32_t want_bytes;            /* WANT size (ngtp_srv_set_window) */
    int depth;                      /* WANTs outstanding */
    uint32_t idle_close_ms;         /* a session open and no frame for this long: close it (design 7.2: 5 min) */
    uint32_t idle_exit_ms;          /* no frame for this long: close everything and exit (bounded wait) */
    int drive;                      /* 0: never send anything to the drive (build T2_NO_DRIVE=1) */
    int tcpfix;                     /* the patched net_tcp.c is linked (shown on the screen only) */
    uint32_t confirm_max_ms;        /* a force question with no answer is refused after this long */
} t2_cfg_t;

void t2_cfg_defaults(t2_cfg_t *c, int tcpfix);

enum { T2_S_NETUP = 0, T2_S_LISTEN, T2_S_SERVE, T2_S_CLOSE, T2_S_DONE, T2_S_FAILED };

/* why the program ended (t2_app_t.result) */
enum { T2_R_RUNNING = 0, T2_R_FINISH, T2_R_USER, T2_R_IDLE, T2_R_NONET, T2_R_NOLISTEN };

typedef struct {
    t2_cfg_t cfg;
    int state, result;
    ngtp_srv_t srv;
    nm_screen_t scr;
    char ip[24];
    /* the write gate */
    int fw_checked, fw_ok;
    ng_fwinfo_t fw;
    char fw_why[64];
    uint32_t gd_fw;                 /* the firmware word of 52 00 (0 = none) */
    /* screen */
    char log[5][NM_COLS + 1];
    int nlog;
    char op[NM_COLS + 1];
    int busy, b_was_down;
    ng_btnguard_t bguard;           /* B counts only after it was seen released (a pad can report it all the time) */
    uint32_t last_ui_ms;
    ng_err_t close_err;
} t2_app_t;

void t2_init(t2_app_t *a, const t2_cfg_t *cfg);
/* one step of the main loop (called every frame).  1 = still running, 0 = done (see a->state / a->result) */
int t2_step(t2_app_t *a);
/* the screen as it is now (also handed to nm_plat_present by t2_step) */
const nm_screen_t *t2_screen(const t2_app_t *a);

/* a line in the log block of the screen (the network bring-up), shown at once */
void t2_log(t2_app_t *a, const char *text);
/* the write gate on its own (host tests): 1 = the drive is NetGD with a write session; why gets the reason otherwise */
int t2_gate(t2_app_t *a, char *why, size_t whyn);

/* The BBA bring-up and the receive window are netgd-menu's (nm_net.h: nm_net_log, nm_net_rcvbuf, nm_net_static_ip; the KOS
   code is ../netgd-menu/src/net_kos.c, shared with the menu).  R178: SO_RCVBUF 20480, a 16 KiB frame and its headers fit;
   14 segments, what the BBA took without loss. */

#endif
