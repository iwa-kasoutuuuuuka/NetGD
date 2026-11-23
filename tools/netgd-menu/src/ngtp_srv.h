/* The NGTP server: takes the PC's requests from the network boundary (nm_net.h), runs them on the netgd-recv
   core (recv_core.h) and answers with the same NGD: lines the dcload-ip transport prints (design 8).
   Single threaded: ngtp_srv_service() is called once per UI frame; an operation blocks inside it and the screen
   is kept alive through the hooks.  Nothing here has run on a Dreamcast. */
#ifndef NGTP_SRV_H
#define NGTP_SRV_H
#include <stddef.h>
#include <stdint.h>
#include "recv_core.h"
#include "fwctl.h"

typedef struct {
    void *ctx;
    void (*line)(void *ctx, const char *line, int throttled);       /* every NGD: line, for the screen's log */
    void (*progress)(void *ctx, const ng_ui_t *u);                  /* put / verify / mount progress */
    int (*stop)(void *ctx);                                         /* 1 = the user wants to stop at the next safe point */
    /* The PC asked for `force` on a protected folder: 1 = the user allowed it on the console.  `what` is "overwrite" or "delete". */
    int (*confirm)(void *ctx, const char *what, const char *folder);
    void (*busy)(void *ctx, int busy);                              /* an operation starts / ends */
    void (*idle)(void *ctx);                                        /* while waiting for the PC's DATA: keep the screen alive */
    uint32_t (*millis)(void *ctx);
    /* Before the first operation opens the write session (the UI drops its read-only mount). */
    void (*before_session)(void *ctx);
    /* The write gate (may be NULL = allowed): asked before every attempt to open the write session.  0 = refuse: the
       operation ends with NGD:ERROR session code=EFW msg=<why> and DONE EFW, and nothing is sent to the drive by this
       module (ng_session_open is not called).  The standalone T2 receiver uses it to write only to a drive that identified
       as NetGD; the menu has checked the firmware at start-up and leaves it NULL. */
    int (*may_write)(void *ctx, char *why, size_t whyn);
} ngtp_hooks_t;

/* The data window (menu-app.md 8.3 WANT): the largest WANT and how many WANTs may be outstanding at once.
   Stock KOS 2.3 TCP: 16 KiB x 1 (R178: frames bigger than the receive window stall, a wider window corrupts the stream).
   With the patched net_tcp.c linked in (tools/kos-patches, docs/research/g2-transport.md 3) the design value is 64 KiB x 2, but
   the default is 16 KiB x 1 there too for now: g2netbench pass p4 (the same 64 KiB x 2 shape) hung the real Dreamcast twice, cause
   unknown, and p3 (64 KiB x 1) was slower than 16 KiB.  This avoids that shape; it does not fix the hang.  A build can still ask
   for a window explicitly (netgd-t2: T2_WANT / T2_DEPTH, netgd-menu: NM_WANT / NM_DEPTH). */
#define NGTP_WANT_STOCK   16384u
#define NGTP_DEPTH_STOCK  1
#define NGTP_WANT_TCPFIX  16384u
#define NGTP_DEPTH_TCPFIX 1
#define NGTP_DEPTH_MAX    4

typedef struct {
    ngtp_hooks_t hk;
    const ng_fwinfo_t *fw;
    uint16_t code;                  /* the pass code the PC must send */
    int connected;                  /* a client is attached */
    int hello;                      /* it has said HELLO */
    int session;                    /* the write session is open */
    int auth_fail;
    int code_expired;               /* 3 wrong codes: the UI makes a new one (ngtp_srv_new_code) */
    int broken;                     /* the connection failed during an operation */
    uint32_t last_frame_ms;         /* when the last request arrived / was answered */
    char client[32];
    uint32_t want_max;              /* the largest WANT (1 .. NGTP_MAX_DATA); ngtp_srv_set_window */
    int depth;                      /* WANTs outstanding at most (1 .. NGTP_DEPTH_MAX) */
    uint32_t wants, data_frames;    /* counters for the screen / tests */
    /* private */
    uint32_t seq;                   /* sequence of the request being served */
    int src_idx;
    uint64_t src_size;              /* manifest size of the file being read */
    uint64_t issue_off;             /* next offset to ask for */
    int src_eof;                    /* a short or empty DATA ended the file early */
    int nq, qh;                     /* outstanding WANTs: count and head index (ring of NGTP_DEPTH_MAX) */
    struct { uint64_t off; uint32_t len; } q[NGTP_DEPTH_MAX];
    uint32_t held_len, held_pos;    /* DATA bytes received but not yet handed to the core */
    int abort_asked;                /* the PC set the abort flag on a DATA frame */
} ngtp_srv_t;

#define NGTP_EV_REQUEST 0x01        /* a request was handled */
#define NGTP_EV_CLOSED  0x02        /* the client went away or was dropped */
#define NGTP_EV_FINISH  0x04        /* FINISH was handled: leave the transfer screen */

void ngtp_srv_init(ngtp_srv_t *s, const ngtp_hooks_t *hk, const ng_fwinfo_t *fw, uint16_t code);
void ngtp_srv_new_code(ngtp_srv_t *s, uint16_t code);
/* WANT size and depth (clamped to 1..NGTP_MAX_DATA, 1..NGTP_DEPTH_MAX).  ngtp_srv_init sets NGTP_MAX_DATA x 1 (the menu's
   behaviour so far: one WANT per read of the receive core). */
void ngtp_srv_set_window(ngtp_srv_t *s, uint32_t want_max, int depth);
/* Accept a client and handle at most one request.  Returns NGTP_EV_* bits. */
int ngtp_srv_service(ngtp_srv_t *s);
void ngtp_srv_drop(ngtp_srv_t *s);                              /* close the client (the session stays as it is) */
/* End the write session if one is open: mode as ng_session_close (1 = default image back).  Quiet. */
ng_err_t ngtp_srv_end(ngtp_srv_t *s, int mode);

#endif
