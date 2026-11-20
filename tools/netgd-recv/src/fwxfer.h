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

#endif
