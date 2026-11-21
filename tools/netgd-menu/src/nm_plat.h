/* The platform boundary (design 4.4).  KOS: plat_kos.c.  Host tests: test/plat_host.c (virtual clock). */
#ifndef NM_PLAT_H
#define NM_PLAT_H
#include <stdint.h>
#include "nm_screen.h"

#define NM_HANDOFF_BIOS   0     /* back to the BIOS menu */
#define NM_HANDOFF_REBOOT 1     /* restart the console */

uint32_t nm_plat_millis(void);                  /* ms since start */
void nm_plat_sleep(uint32_t ms);                /* host: advances the virtual clock */
uint32_t nm_plat_buttons(void);                 /* NM_BTN_* bits that are down now */
void nm_plat_present(const nm_screen_t *s);     /* draw the grid (only the cells that changed) */
void nm_plat_handoff(int mode);                 /* does not return on the Dreamcast; the host prints HANDOFF and exits */
int nm_plat_dcload(void);                       /* 1 when started from dcload-ip */
uint32_t nm_plat_entropy(void);                 /* seed for the pass code */
int nm_plat_controller(void);                   /* 1 when a controller is connected (an addition to design 4.4: "No controller") */

#endif
