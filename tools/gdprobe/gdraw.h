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

#endif
