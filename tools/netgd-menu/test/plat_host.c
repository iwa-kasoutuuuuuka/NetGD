/* The platform boundary for the host tests: a virtual clock, scripted buttons, the last screen kept for dumping.
   Also the few functions the netgd-recv core wants from its platform (ng_plat.h) and FatFs's get_fattime. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "nm_plat.h"
#include "ng_plat.h"
#include "ff.h"
#include "plat_host.h"

uint32_t nm_host_now;
uint32_t nm_host_buttons;
int nm_host_controller = 1;
nm_screen_t nm_host_screen;
int nm_host_frames;
int nm_host_dcload;
uint32_t nm_host_entropy = 4821;
int nm_host_user_stop;
int nm_host_stop_after_polls = -1;
int nm_host_polls;
int nm_host_realtime;
uint32_t (*nm_host_buttons_hook)(void);
static uint64_t real_t0;

uint32_t nm_plat_millis(void)
{
    if (nm_host_realtime) {
        if (!real_t0) real_t0 = nm_host_real_ms() - 1000;
        return (uint32_t)(nm_host_real_ms() - real_t0);
    }
    return nm_host_now;
}

void nm_plat_sleep(uint32_t ms)
{
    if (nm_host_realtime) nm_host_real_sleep(ms);
    else nm_host_now += ms;
}

uint32_t nm_plat_buttons(void) { return nm_host_buttons_hook ? nm_host_buttons_hook() : nm_host_buttons; }
int nm_plat_controller(void) { return nm_host_controller; }
int nm_plat_dcload(void) { return nm_host_dcload; }
uint32_t nm_plat_entropy(void) { return nm_host_entropy; }

void nm_plat_present(const nm_screen_t *s)
{
    nm_host_screen = *s;
    nm_host_frames++;
}

void nm_plat_handoff(int mode)
{
    char row[NM_COLS + 1];
    for (int r = 0; r < NM_ROWS; r++) {                 /* what was on the screen when the BIOS took over */
        nm_screen_row_text(&nm_host_screen, r, row);
        if (row[0]) printf("H%02d|%s\n", r, row);
    }
    printf("HANDOFF %s\n", mode == NM_HANDOFF_REBOOT ? "reboot" : "bios");
    fflush(stdout);
    exit(0);                    /* atexit() prints the SIM summary line */
}

/* ---- for the netgd-recv core ---- */
uint32_t ng_plat_millis(void) { return nm_plat_millis(); }
const char *ng_plat_pcpath(const char *pc, char *buf, size_t n) { (void)buf; (void)n; return pc; }
void ng_plat_console(const char *line) { (void)line; }
int ng_plat_user_stop(void)
{
    nm_host_polls++;
    if (nm_host_stop_after_polls >= 0 && nm_host_polls > nm_host_stop_after_polls) {
        nm_host_stop_after_polls = -1;          /* one B press: the next run is not stopped */
        return 1;
    }
    return nm_host_user_stop;
}
DWORD get_fattime(void) { return ((DWORD)(2025 - 1980) << 25) | (6u << 21) | (1u << 16); }
