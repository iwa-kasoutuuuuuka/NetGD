/* The NGTP server: takes the PC's requests from the network boundary (nm_net.h), runs them on the netgd-recv
   core (recv_core.h) and answers with the same NGD: lines the dcload-ip transport prints (design 8).
   Single threaded: ngtp_srv_service() is called once per UI frame; an operation blocks inside it and the screen
   is kept alive through the hooks.  Nothing here has run on a Dreamcast. */
#ifndef NGTP_SRV_H
#define NGTP_SRV_H
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
} ngtp_hooks_t;

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
    /* private */
    uint32_t seq;                   /* sequence of the request being served */
    int src_idx;
    uint64_t src_off;
    int abort_asked;                /* the PC set the abort flag on a DATA frame */
} ngtp_srv_t;

#define NGTP_EV_REQUEST 0x01        /* a request was handled */
#define NGTP_EV_CLOSED  0x02        /* the client went away or was dropped */
#define NGTP_EV_FINISH  0x04        /* FINISH was handled: leave the transfer screen */

void ngtp_srv_init(ngtp_srv_t *s, const ngtp_hooks_t *hk, const ng_fwinfo_t *fw, uint16_t code);
void ngtp_srv_new_code(ngtp_srv_t *s, uint16_t code);
/* Accept a client and handle at most one request.  Returns NGTP_EV_* bits. */
int ngtp_srv_service(ngtp_srv_t *s);
void ngtp_srv_drop(ngtp_srv_t *s);                              /* close the client (the session stays as it is) */
/* End the write session if one is open: mode as ng_session_close (1 = default image back).  Quiet. */
ng_err_t ngtp_srv_end(ngtp_srv_t *s, int mode);

#endif
