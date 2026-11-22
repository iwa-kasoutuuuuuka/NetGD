/* The command layer to the NetGD firmware: 0xE0 IMAGE_CTL on the primary (GD-ROM)
   device -- VERSION, MOUNT_NAME, and the F6 write session (design 4.2).

   ALL of the firmware-facing packet layouts live in fwctl.c so that when the firmware
   side settles (mcu/netgd/src/ide.rs), this is the one file to fix.  fwctl.c sends
   its packets through fw_xfer (fwxfer.h): fwxfer_kos.c on the Dreamcast, the packet-level
   firmware model test/fwctl_sim.c in the host tests. */
#ifndef NG_FWCTL_H
#define NG_FWCTL_H
#include <stdint.h>

/* VERSION reply byte 5 */
#define FW_CAP_SESSION  0x01    /* WSESSION_BEGIN / END / STATUS */
#define FW_CAP_CHECKSUM 0x02    /* STATUS carries the FNV-1a of the last write command */
#define FW_CAP_DMA      0x04    /* IDENTIFY advertises MWDMA2 */
#define FW_CAP_STACK    0x08    /* STACK (sub-command 9): stack high-water mark (firmware feature stack-hw) */

typedef struct {
    uint8_t proto;              /* OGDM protocol version */
    uint8_t caps;               /* FW_CAP_* */
    char fwstr[25];             /* firmware version string */
} ng_fwinfo_t;

typedef struct {
    uint8_t flags;              /* bit0 media present, bit1 session, bit2 last write ok */
    uint32_t sectors_written;
    uint32_t failed_cmds;
    uint32_t last_lba;
    uint32_t last_count;
    uint32_t last_fnv;
    uint32_t part_lba;          /* first partition start (protected area boundary), 0 = unknown */
} ng_fwstatus_t;

/* STACK reply (common/stackmark/src/lib.rs, Report::encode).  Byte counts. */
typedef struct {
    uint8_t format;             /* reply layout version (1) */
    uint32_t reserved;          /* _stack_start - __sheap: the whole stack region */
    uint32_t used;              /* high-water mark since the firmware's reset; == reserved: overrun */
    uint32_t unpainted;         /* top bytes never painted (counted as used): the smallest `used` can show */
} ng_fwstack_t;

#define FW_ST_MEDIA   0x01
#define FW_ST_SESSION 0x02
#define FW_ST_LASTOK  0x04

/* 0 on success.  <0: no answer / not "OGDM" (a stock GDEMU, or no firmware). */
int fw_version(ng_fwinfo_t *out);

/* Eject the GD-ROM image and open the write session (WSESSION_BEGIN).  There is no fallback
   for a firmware without FW_CAP_SESSION: ng_session_open refuses to write to one. */
int fw_session_begin(const ng_fwinfo_t *fw);

/* End it.  mode 0: mount nothing, 1: previous selection / GDEMU.CFG default,
   2: mount the folder `name` (MOUNT_NAME chunks, then the end).  0 on success. */
int fw_session_end(const ng_fwinfo_t *fw, int mode, const char *name);

int fw_status(ng_fwstatus_t *out);

/* E0 09: the firmware's stack high-water mark.  Read-only.  Ask only when VERSION has FW_CAP_STACK
   (any other firmware answers ILLEGAL REQUEST).  0 on success, -1: no answer / not "NGSK". */
int fw_stack(ng_fwstack_t *out);

/* ---- commands for the menu (netgd-menu), design docs/design/menu-app.md 5.2 ----
   All return 0 = success, -1 = failure.  `sense` (may be NULL) gets the sense key of the failure:
   5 ILLEGAL REQUEST (no such folder / no image / mount failed), 2 NOT READY, 6 UNIT ATTENTION.
   A UNIT ATTENTION is repeated once inside (not executed), except in fw_test_unit_ready. */
int fw_gdemu_mount(uint16_t n, uint8_t *sense);             /* 52 82 lo hi; n = 0: the default image */
int fw_gdemu_step(int prev, uint8_t *sense);                /* 52 81 55 (next) / 52 81 44 (prev) */
int fw_gdemu_version(uint32_t *boot, uint32_t *fw);         /* 52 00 00 00 08: two LE u32 (major<<24|minor<<16|patch<<8) */
int fw_mount_named(const char *name, uint8_t *sense);       /* E0 05 chunks, then E0 02 00 */
int fw_mount_default(uint8_t *sense);                       /* E0 02 FF */
int fw_test_unit_ready(uint8_t *sense);                     /* 00: 0 = READY; never repeated on UNIT ATTENTION */

/* ---- identification that is safe on a STOCK GDEMU (netgd-t2's write gate) ----
   1. 52 00 GDEMU_VERSION with the CDB gdprobe sent to the stock GDEMU 6.0 (52 00 00 .. 00, R-record 08: it answered
      8 bytes 00 00 05 02 00 05 20 05 = bootloader 2.5.0, firmware 5.20.5).  Repeated once on UNIT ATTENTION.
   2. Only when the firmware word is NetGD's (FW_NETGD_GDEMU_WORD: NetGD reports GDEMU 5.20.0, mcu/netgd/src/ide.rs
      GDEMU_COMPAT_VERSION): E0 03 VERSION, once, NOT repeated on UNIT ATTENTION, must answer "OGDM".
   Every packet is limited to FW_PROBE_LIMIT_MS (fw_xfer_limit_ms).  IDENTIFY PACKET and REQ_MODE cannot tell the
   two apart: NetGD answers them with the stock bytes on purpose.  A stock firmware that reported exactly 5.20.0
   would get the one E0 03 (none known: the record is 5.20.5). */
#define FW_NETGD_GDEMU_WORD 0x05200000u
#define FW_PROBE_LIMIT_MS   3000u
enum { FW_PROBE_NETGD = 0, FW_PROBE_NOANSWER = -1, FW_PROBE_OTHER = -2, FW_PROBE_NOTOGDM = -3 };
/* out: the VERSION reply (FW_PROBE_NETGD only).  boot / fwv (may be NULL): the 52 00 words when they were read. */
int fw_probe_netgd(ng_fwinfo_t *out, uint32_t *boot, uint32_t *fwv);

#endif
