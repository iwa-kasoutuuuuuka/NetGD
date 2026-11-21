/* KOS platform glue: nm_plat_* (nm_plat.h) and what the netgd-recv core wants (ng_plat.h, get_fattime).
   Thin on purpose: no decisions here.  NOT run on hardware.  Compile-checked by CI (kos.yaml, KOS 2.3.0). */
#include <stdio.h>
#include <string.h>
#include <time.h>
#include <kos.h>
#include <arch/arch.h>              /* KOS-API-UNVERIFIED: arch_menu(), arch_reboot() (design K1) */
#include <dc/video.h>
#include <dc/biosfont.h>
#include <dc/fs_dcload.h>
#include <dc/maple.h>
#include <dc/maple/controller.h>
#include "nm.h"
#include "nm_plat.h"
#include "nm_art.h"
#include "ng_plat.h"
#include "ff.h"

#define CELL_W 12
#define CELL_H 24
#define X0 2                        /* 640 - 53 * 12 = 4 px of margin; 20 rows fill the 480 lines (design H10: overscan) */
#define Y0 0

/* RGB565, the same values as tools/netgd-fwupd/main.c */
#define COL_WHITE 0xFFFF
#define COL_BLACK 0x0000
#define COL_GREY  0xAD55
#define COL_WARN  0xFFE0            /* yellow */

uint32_t nm_plat_millis(void) { return (uint32_t)timer_ms_gettime64(); }

void nm_plat_sleep(uint32_t ms) { thd_sleep((int)ms); }

/* The first controller found (ports A to D).  The analog stick counts as the D-pad past +-64, the triggers as buttons past 128. */
static cont_state_t *first_pad(void)
{
    maple_device_t *c = maple_enum_type(0, MAPLE_FUNC_CONTROLLER);
    return c ? (cont_state_t *)maple_dev_status(c) : NULL;
}

int nm_plat_controller(void) { return maple_enum_type(0, MAPLE_FUNC_CONTROLLER) != NULL; }

uint32_t nm_plat_buttons(void)
{
    cont_state_t *st = first_pad();
    uint32_t b = 0;
    if (!st) return 0;
    if (st->buttons & CONT_DPAD_UP) b |= NM_BTN_UP;
    if (st->buttons & CONT_DPAD_DOWN) b |= NM_BTN_DOWN;
    if (st->buttons & CONT_DPAD_LEFT) b |= NM_BTN_LEFT;
    if (st->buttons & CONT_DPAD_RIGHT) b |= NM_BTN_RIGHT;
    if (st->buttons & CONT_A) b |= NM_BTN_A;
    if (st->buttons & CONT_B) b |= NM_BTN_B;
    if (st->buttons & CONT_X) b |= NM_BTN_X;
    if (st->buttons & CONT_Y) b |= NM_BTN_Y;
    if (st->buttons & CONT_START) b |= NM_BTN_START;
    if (st->joyy < -64) b |= NM_BTN_UP;
    if (st->joyy > 64) b |= NM_BTN_DOWN;
    if (st->joyx < -64) b |= NM_BTN_LEFT;
    if (st->joyx > 64) b |= NM_BTN_RIGHT;
    if (st->ltrig > 128) b |= NM_BTN_LT;
    if (st->rtrig > 128) b |= NM_BTN_RT;
    return b;
}

/* ---- screen: only the cells that changed since the last call ---- */

/* ---- box art (design step 11): a 128 x 128 RGB565 picture written straight into the frame buffer, in the block of cells
   that the core keeps blank while a picture is set (nm_art.h).  The block is repainted as text (blank or not) whenever the
   picture changes, so a removed picture never leaves pixels behind. ---- */

#define ART_X (X0 + NM_ART_COL * CELL_W + 2)
#define ART_Y (Y0 + NM_ART_ROW * CELL_H + 8)

static const uint16_t *art_want, *art_shown;

void nm_plat_set_art(const uint16_t *px) { art_want = px; }

static char prev_ch[NM_ROWS][NM_COLS];
static uint8_t prev_attr[NM_ROWS][NM_COLS];
static int have_prev;

static void set_colors(int attr)
{
    switch (attr) {
    case NM_ATTR_INVERT: bfont_set_foreground_color(COL_BLACK); bfont_set_background_color(COL_WHITE); break;
    case NM_ATTR_DIM:    bfont_set_foreground_color(COL_GREY);  bfont_set_background_color(COL_BLACK); break;
    case NM_ATTR_WARN:   bfont_set_foreground_color(COL_WARN);  bfont_set_background_color(COL_BLACK); break;
    default:             bfont_set_foreground_color(COL_WHITE); bfont_set_background_color(COL_BLACK); break;
    }
}

static int changed(const nm_screen_t *s, int r, int c)
{
    return !have_prev || s->ch[r][c] != prev_ch[r][c] || s->attr[r][c] != prev_attr[r][c];
}

void nm_plat_present(const nm_screen_t *s)
{
    if (art_want != art_shown && have_prev) {
        for (int r = NM_ART_ROW; r < NM_ART_ROW + NM_ART_ROWS; r++)
            for (int c = NM_ART_COL; c < NM_COLS; c++) prev_ch[r][c] = 0;       /* never a screen character: drawn again below */
    }
    for (int r = 0; r < NM_ROWS; r++) {
        int c = 0;
        while (c < NM_COLS) {
            char run[NM_COLS + 1];
            int end;
            uint8_t attr;
            if (!changed(s, r, c)) { c++; continue; }
            attr = s->attr[r][c];
            end = c;
            while (end < NM_COLS && s->attr[r][end] == attr && changed(s, r, end)) end++;
            memcpy(run, &s->ch[r][c], (size_t)(end - c));
            run[end - c] = 0;
            set_colors(attr);
            bfont_draw_str(vram_s + (Y0 + r * CELL_H) * 640 + X0 + c * CELL_W, 640, 1, run);
            c = end;
        }
    }
    memcpy(prev_ch, s->ch, sizeof prev_ch);
    memcpy(prev_attr, s->attr, sizeof prev_attr);
    have_prev = 1;
    if (art_want && art_want != art_shown) {
        for (int y = 0; y < NM_ART_PH; y++) memcpy(vram_s + (ART_Y + y) * 640 + ART_X, art_want + y * NM_ART_PW, NM_ART_PW * 2);
    }
    art_shown = art_want;
}

void nm_plat_handoff(int mode)
{
    if (mode == NM_HANDOFF_REBOOT) arch_reboot();
    arch_menu();
    for (;;) thd_sleep(1000);       /* not reached */
}

int nm_plat_dcload(void) { return dcload_type == DCLOAD_TYPE_IP; }

uint32_t nm_plat_entropy(void)
{
    return (uint32_t)timer_ms_gettime64() * 2654435761u ^ (uint32_t)time(NULL);
}

/* ---- for the netgd-recv core (ng_plat.h) ---- */

uint32_t ng_plat_millis(void) { return (uint32_t)timer_ms_gettime64(); }

const char *ng_plat_pcpath(const char *pc, char *buf, size_t n)
{
    (void)buf; (void)n;
    return pc;
}

void ng_plat_console(const char *line)
{
    printf("%s\n", line);
    fflush(stdout);
}

/* B held on the controller: stop at the next safe point */
int ng_plat_user_stop(void) { return (nm_plat_buttons() & NM_BTN_B) ? 1 : 0; }

/* FatFs clock (FF_FS_NORTC = 0) */
DWORD get_fattime(void)
{
    time_t t = time(NULL);
    struct tm *tm = localtime(&t);
    if (!tm || tm->tm_year < 80) return ((DWORD)(2025 - 1980) << 25) | (1u << 21) | (1u << 16);
    return ((DWORD)(tm->tm_year - 80) << 25) | ((DWORD)(tm->tm_mon + 1) << 21) | ((DWORD)tm->tm_mday << 16) |
           ((DWORD)tm->tm_hour << 11) | ((DWORD)tm->tm_min << 5) | ((DWORD)tm->tm_sec >> 1);
}
