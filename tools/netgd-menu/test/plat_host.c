/* The platform boundary for the host tests: a virtual clock, scripted buttons, the last screen kept for dumping.
   Also the few functions the netgd-recv core wants from its platform (ng_plat.h) and FatFs's get_fattime. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "nm_plat.h"
#include "nm_art.h"
#include "ng_plat.h"
#include "ff.h"
#include "plat_host.h"

uint32_t nm_host_now;
uint32_t nm_host_buttons;
int nm_host_controller = 1;
nm_screen_t nm_host_screen;
const uint16_t *nm_host_art;           /* what the core asked to show */
int nm_host_art_dirty;                  /* presents in which a cell of the art block was not blank while a picture was set */
int nm_host_frames;
int nm_host_dcload;
uint32_t nm_host_entropy = 4821;
int nm_host_user_stop;
int nm_host_stop_after_polls = -1;
int nm_host_polls;
uint32_t nm_host_b_hold_ms;             /* when ng_plat_user_stop() says stop: B stays down this long (the press that did it) */
static uint32_t b_down_until;
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

uint32_t nm_plat_buttons(void)
{
    uint32_t b = nm_host_buttons_hook ? nm_host_buttons_hook() : nm_host_buttons;
    if (b_down_until && (int32_t)(b_down_until - nm_plat_millis()) > 0) b |= NM_BTN_B;
    return b;
}
uint32_t nm_plat_pad(uint32_t *buttons)
{
    uint32_t b = nm_plat_buttons();     /* first: a scripted hook may plug or unplug the pad */
    *buttons = nm_host_controller ? b : 0;
    return (uint32_t)nm_host_controller;
}
int nm_plat_controller(void) { return nm_host_controller; }
int nm_plat_dcload(void) { return nm_host_dcload; }
uint32_t nm_plat_entropy(void) { return nm_host_entropy; }

void nm_plat_set_art(const uint16_t *px) { nm_host_art = px; }

void nm_plat_present(const nm_screen_t *s)
{
    if (nm_host_art)
        for (int r = NM_ART_ROW; r < NM_ART_ROW + NM_ART_ROWS; r++)
            for (int c = NM_ART_COL; c < NM_COLS; c++)
                if (s->ch[r][c] != ' ' || s->attr[r][c] != NM_ATTR_NORMAL) nm_host_art_dirty++;
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
int ng_plat_user_stop(uint32_t *pad)
{
    uint32_t b = nm_host_buttons_hook ? nm_host_buttons_hook() : nm_host_buttons;   /* a scripted pad (not the --hold-b-ms tail) */
    *pad = (uint32_t)nm_host_controller;
    nm_host_polls++;
    if (!nm_host_controller) return 0;
    if (nm_host_stop_after_polls >= 0 && nm_host_polls > nm_host_stop_after_polls) {
        nm_host_stop_after_polls = -1;          /* one B press: the next run is not stopped */
        if (nm_host_b_hold_ms) b_down_until = nm_plat_millis() + nm_host_b_hold_ms;
        return 1;
    }
    return nm_host_user_stop || (b & NM_BTN_B);
}
DWORD get_fattime(void) { return ((DWORD)(2025 - 1980) << 25) | (6u << 21) | (1u << 16); }
