/* netgd-recv: common definitions.  Design: docs/design/bba-sd-write.md */
#ifndef NG_H
#define NG_H

#include <stdint.h>
#include <stddef.h>

#define NG_APP_VERSION "1.0.0"

/* Error codes: the names are the `code=` of NGD:ERROR lines (design 5.4 list). */
typedef enum {
    NG_OK = 0,
    NG_E_VERSION, NG_E_AUTH, NG_E_BUSY, NG_E_BADCRC, NG_E_MANIFEST, NG_E_EXIST,
    NG_E_NOENT, NG_E_NOSPC, NG_E_FRAG, NG_E_CRC, NG_E_IO, NG_E_BUS,
    NG_E_PROTECTED, NG_E_FW, NG_E_STATE,
    NG_E_FS,            /* not a first-partition FAT32 volume (design 5.5) -- name is ours */
    /* not errors of the image: the run is stopped on purpose */
    NG_E_STOP,          /* reload.req or cancel.req seen: state saved, leave */
    NG_E_JOB,           /* job.ngj unreadable / unsupported */
    NG_E_SRC            /* cannot read a source file under /pc/ */
} ng_err_t;

const char *ng_err_name(ng_err_t e);

/* tunables (runtime so the host tests can shrink them) */
typedef struct {
    uint32_t ckp_bytes;     /* NETGD.CKP interval; design: 8 MiB */
    uint32_t read_bytes;    /* /pc/ read size; design: 64 KiB */
    int readback;           /* read every file back through FatFs and compare CRC32 (4.5) */
    int allow_frag;         /* fall back to a fragmented allocation when no contiguous run exists */
    int force;              /* may overwrite / delete protected folders (5.5) */
    int force_slow;         /* tests: use the FatFs write path even when the allocation is contiguous */
    uint32_t headroom;      /* free space kept beyond the need; design: 16 MiB */
    uint32_t mount_line_ms; /* least time between two NGD:MOUNT progress lines; 1000 */
} ng_cfg_t;

extern ng_cfg_t ng_cfg;
void ng_cfg_defaults(void);

#endif
