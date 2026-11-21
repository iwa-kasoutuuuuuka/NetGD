/* gdprobe: raw G1 ATAPI access (KallistiOS only, not host-testable).
 *
 * Mirrors what openMenu's gdemu_sdk.c does (PACKET command 0xA0, six command
 * words, wait for the GD-ROM interrupt, read status/data), with two
 * differences: every wait has a timeout, and everything runs under the KOS
 * G1 ATA mutex so the cdrom driver cannot interleave a command of its own.
 */
#ifndef GDPROBE_GDRAW_H
#define GDPROBE_GDRAW_H
#include "gdtypes.h"

/* Fill *ops with the hardware implementation. */
void gdraw_get_ops(gd_ops *ops);

/* Make the G1 ATA registers readable without KOS's cdrom_init (R117).
 * If the Holly G1 bus protection has not passed (0x005F74EC != 3), redo the
 * BIOS check that cdrom_init does (kernel/arch/dreamcast/hardware/cdrom.c,
 * cdrom_init: write the size to 0x005F74E4, then read the BIOS across the bus).
 * This talks to the bus controller only: no register of the drive is written.
 * Writes a one-line summary (no newline) to buf. Returns the final status. */
unsigned gdraw_bus_prepare(char *buf, unsigned len);

#endif
