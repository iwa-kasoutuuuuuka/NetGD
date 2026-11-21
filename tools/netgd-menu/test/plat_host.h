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
extern int nm_host_stop_after_polls;    /* ng_plat_user_stop() returns 1 from this poll on; <0 never */
extern int nm_host_polls;
extern int nm_host_realtime;            /* 1: the clock is the real one and nm_plat_sleep really sleeps (serve mode) */
extern uint32_t (*nm_host_buttons_hook)(void);      /* if set, nm_plat_buttons() returns what it says */
uint64_t nm_host_real_ms(void);         /* net_host.c */
void nm_host_real_sleep(uint32_t ms);
extern uint16_t nm_host_net_port;       /* net_host.c */
extern int nm_host_net_nodev;

#endif
