/* T1: take the job from dcload-ip's /pc/ (design 5.3) and report with NGD: lines. */
#ifndef NG_XPORT_PC_H
#define NG_XPORT_PC_H
#include "recv_core.h"

/* NG_JOBDIR_RAW arrives unquoted from the Makefile (-DNG_JOBDIR_RAW=/pc/netgd/dc1) */
#define NG_STR_(x) #x
#define NG_STR(x)  NG_STR_(x)
#ifdef NG_JOBDIR_RAW
#define NG_JOBDIR_DEFAULT NG_STR(NG_JOBDIR_RAW)
#endif
#ifndef NG_JOBDIR_DEFAULT
#define NG_JOBDIR_DEFAULT "/pc/netgd/dc1"
#endif

/* The job folder: NG_JOBDIR_DEFAULT if it holds job.ngj, else the only /pc/netgd/<unit>
   that does.  NULL if none / ambiguous.  (netgd-send puts it at <drive>:\netgd\<unit>.) */
const char *ng_pc_find_jobdir(char *buf, size_t n);

/* Run job.ngj.  0 = every operation ok, 1 = something failed, 2 = stopped early
   (reload.req / cancel.req) or cannot continue; nothing is lost in that case. */
int ng_pc_run(const char *jobdir, void (*ui)(const ng_ui_t *));

#endif
