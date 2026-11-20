/* netgd-fwupd: raw G1 ATA access to the secondary (disk) device. KOS only, not host-testable. */
#ifndef NETGD_FWUPD_G1DEV_H
#define NETGD_FWUPD_G1DEV_H
#include "fwcore.h"

/* Fill the device half of *env (f9, identify, prepare, now_ms, sleep_ms). The caller
 * sets progress / log / ctx. */
void g1dev_fill_env(fw_env *env);

/* Select the primary (GD-ROM) device again, as KOS's g1_ata_shutdown does. */
void g1dev_deselect(void);

#endif
