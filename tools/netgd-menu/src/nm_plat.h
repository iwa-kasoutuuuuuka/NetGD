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
/* The same bits and, as the result, which pad they came from: 0 = no pad (*buttons is 0), otherwise a number that stays the same
   for one port and unit.  One read, so the bits always belong to that pad (ng_btnguard.h). */
uint32_t nm_plat_pad(uint32_t *buttons);
void nm_plat_present(const nm_screen_t *s);     /* draw the grid (only the cells that changed) */
void nm_plat_handoff(int mode);                 /* does not return on the Dreamcast; the host prints HANDOFF and exits */
int nm_plat_dcload(void);                       /* 1 when started from dcload-ip */
uint32_t nm_plat_entropy(void);                 /* seed for the pass code */
/* The box art to show (design step 11): NM_ART_PW x NM_ART_PH RGB565 pixels in the block of the grid that starts at
   (NM_ART_ROW, NM_ART_COL) (nm_art.h), or NULL for none.  Called before every nm_plat_present of a frame; the core keeps the
   cells of the block blank while a picture is set.  The pixels stay valid until the next call that changes them (the core
   always sets NULL in between). */
void nm_plat_set_art(const uint16_t *px);
int nm_plat_controller(void);                   /* 1 when a controller is connected (an addition to design 4.4: "No controller") */

#endif
