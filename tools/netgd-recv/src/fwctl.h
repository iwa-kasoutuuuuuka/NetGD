/* The command layer to the NetGD firmware: 0xE0 IMAGE_CTL on the primary (GD-ROM)
   device -- VERSION, MOUNT_NAME, and the F6 write session (design 4.2).

   ALL of the firmware-facing packet layouts live in fwctl.c (KOS build) so that when
   the firmware side settles (mcu/opengdemu/src/ide.rs), this is the one file to fix.
   The host tests link test/fwctl_sim.c instead. */
#ifndef NG_FWCTL_H
#define NG_FWCTL_H
#include <stdint.h>

/* VERSION reply byte 5 */
#define FW_CAP_SESSION  0x01    /* WSESSION_BEGIN / END / STATUS */
#define FW_CAP_CHECKSUM 0x02    /* STATUS carries the FNV-1a of the last write command */
#define FW_CAP_DMA      0x04    /* IDENTIFY advertises MWDMA2 */

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

#define FW_ST_MEDIA   0x01
#define FW_ST_SESSION 0x02
#define FW_ST_LASTOK  0x04

/* 0 on success.  <0: no answer / not "OGDM" (a stock GDEMU, or no firmware). */
int fw_version(ng_fwinfo_t *out);

/* Eject the GD-ROM image and open the write session.  With a firmware lacking
   FW_CAP_SESSION this falls back to an ATAPI START STOP UNIT eject. */
int fw_session_begin(const ng_fwinfo_t *fw);

/* End it.  mode 0: mount nothing, 1: previous selection / GDEMU.CFG default,
   2: mount the folder `name` (MOUNT_NAME chunks, then the end).  0 on success. */
int fw_session_end(const ng_fwinfo_t *fw, int mode, const char *name);

int fw_status(ng_fwstatus_t *out);

#endif
