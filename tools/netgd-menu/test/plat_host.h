#ifndef NM_PLAT_HOST_H
#define NM_PLAT_HOST_H
#include <stdint.h>
#include "nm_screen.h"

extern uint32_t nm_host_now;            /* the virtual clock, ms */
extern uint32_t nm_host_buttons;        /* what nm_plat_buttons() returns */
extern int nm_host_controller;
extern nm_screen_t nm_host_screen;      /* the last presented screen */
extern int nm_host_frames;
extern int nm_host_dcload;
extern uint32_t nm_host_entropy;
extern int nm_host_user_stop;           /* what ng_plat_user_stop() returns */

#endif
