/* Starting a game: mount it in the firmware, wait until the drive is READY (design 5.3).
   One call of nm_launch_step() does one small step, so the main loop never blocks.
   Handing control to the BIOS is not here: the UI does that (nm_plat_handoff) after NM_L_OK. */
#ifndef NM_LAUNCH_H
#define NM_LAUNCH_H
#include <stdint.h>

typedef enum {
    NM_L_BUSY = 0,
    NM_L_OK,            /* mounted and READY (two TEST UNIT READY in a row) */
    NM_L_NOIMAGE,       /* the firmware has no mountable image there (sense key 5) */
    NM_L_FWERR,         /* any other failure of the mount command */
    NM_L_NOTREADY       /* still not READY after the limit */
} nm_launch_result_t;

typedef struct {
    int phase;                  /* 0 mount, 1 wait for READY, 2 finished */
    int result;                 /* nm_launch_result_t */
    unsigned number;            /* 0 = by name */
    char folder[97];
    uint32_t limit_ms;          /* 10000 + open_time + detect_time (the keys the INI set) */
    uint32_t wait_start_ms, last_poll_ms;
    int ready_streak;
    int polls;                  /* TEST UNIT READY sent so far (this wait) */
    uint8_t sense;              /* the sense key of the last failure */
} nm_launch_t;

/* Unmounts the light FatFs mount (ngfs_unmount) and prepares the launch. */
void nm_launch_begin(nm_launch_t *l, unsigned number, const char *folder, uint32_t limit_ms);
nm_launch_result_t nm_launch_step(nm_launch_t *l);
/* after NM_L_NOTREADY: wait again for another `limit_ms` (the mount is not repeated) */
void nm_launch_wait_again(nm_launch_t *l);
/* seconds the wait has run (for the screen) */
uint32_t nm_launch_waited_ms(const nm_launch_t *l);

#endif
