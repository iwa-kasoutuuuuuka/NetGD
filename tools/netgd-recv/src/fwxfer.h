/* One ATAPI PACKET command to the primary (GD-ROM) device = the transport under fwctl.c.
   KOS build: fwxfer_kos.c (G1 registers).  Host tests: test/fwctl_sim.c (a packet-level model
   of the firmware), so fwctl.c itself -- the packet layouts and the UNIT ATTENTION retry --
   is built and tested on the host. */
#ifndef NG_FWXFER_H
#define NG_FWXFER_H
#include <stdint.h>

/* data/alloc: data-in buffer (alloc 0 = none).  Returns 0 ok, -1 error/timeout.
   *err receives the ATA error register after a CHECK CONDITION (sense key = *err >> 4),
   0 for a timeout / transport failure. */
int fw_xfer(const uint8_t pkt[12], void *data, uint32_t alloc, uint32_t *got, uint8_t *err);

/* 0 (default): the fixed waits of fw_xfer (up to about 39 s for one command, 30 s of it for BSY after the packet).
   Otherwise every command ends within this many ms however the drive behaves (netgd-t2's identification of the
   drive: a stock GDEMU that stays BSY must not hold the PC's connection past its timeout). */
extern uint32_t fw_xfer_limit_ms;

#endif
