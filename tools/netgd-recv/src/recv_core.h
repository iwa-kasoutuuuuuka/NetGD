/* The receive core: put / resume / verify / delete / list / finish, independent of the
   transport (design 6.1 recv_core.c).  The transport supplies the file bytes (ng_src_t)
   and takes the NGD: lines (ng_sink_t). */
#ifndef NG_RECV_CORE_H
#define NG_RECV_CORE_H
#include <stdint.h>
#include "ng.h"
#include "manifest.h"
#include "fwctl.h"

typedef struct {
    const char *op;         /* "put" "verify" ... */
    const char *file;
    int fi, fn;
    uint64_t cum, total;
    uint32_t rate;          /* bytes/s */
} ng_ui_t;

typedef struct {
    void *ctx;
    /* One complete "NGD:..." line without the newline.  throttled=1 lines are progress
       chatter the sink may drop (the console wants at most one per second). */
    void (*line)(void *ctx, const char *line, int throttled);
    /* 0 = go on, 1 = reload.req / cancel.req was seen: stop at the next safe point */
    int (*poll)(void *ctx);
    void (*ui)(void *ctx, const ng_ui_t *u);        /* may be NULL */
    uint32_t (*millis)(void *ctx);
} ng_sink_t;

typedef struct {
    void *ctx;
    /* position the source for manifest file `idx` at byte `off` */
    int (*open)(void *ctx, int idx, uint64_t off);
    /* bytes read; 0 = end of file; <0 = error */
    int32_t (*read)(void *ctx, void *buf, uint32_t n);
    void (*close)(void *ctx);
} ng_src_t;

enum { NG_PUT_NEW = 0, NG_PUT_RESUME = 1, NG_PUT_OVERWRITE = 2 };

/* Session = write session on the firmware + mounted FatFs + HELLO line. */
ng_err_t ng_session_open(const ng_sink_t *sk);
/* mode: 0 none, 1 default, 2 mount `folder` (only if it is a complete image). */
ng_err_t ng_session_close(const ng_sink_t *sk, int mode, const char *folder, int quiet);
const ng_fwinfo_t *ng_session_fw(void);

ng_err_t ng_put(const ng_sink_t *sk, const char *tag, const ng_manifest_t *m, const char *mtext, size_t mlen,
                const ng_src_t *src, int mode);
ng_err_t ng_verify(const ng_sink_t *sk, const char *folder);
ng_err_t ng_delete(const ng_sink_t *sk, const char *folder);
ng_err_t ng_list(const ng_sink_t *sk, const char *folder);
ng_err_t ng_statfs(const ng_sink_t *sk);

/* a put-time error line: NGD:ERROR <tag> code=... */
void ng_emit_error(const ng_sink_t *sk, const char *tag, ng_err_t e, int file, uint64_t off, const char *msg);
void ng_emitf(const ng_sink_t *sk, int throttled, const char *fmt, ...);

#endif
