#include <stdio.h>
#include <string.h>
#include "nm_scan.h"
#include "nm_slot.h"
#include "nm_text.h"
#include "nm_ipbin.h"
#include "manifest.h"

#define SMALL_MAX 4096

static int ieq(const char *a, const char *b)
{
    for (; *a && *b; a++, b++) {
        char x = *a, y = *b;
        if (x >= 'a' && x <= 'z') x = (char)(x - 32);
        if (y >= 'a' && y <= 'z') y = (char)(y - 32);
        if (x != y) return 0;
    }
    return *a == *b;
}

static const char *ext_of(const char *name)
{
    const char *d = strrchr(name, '.');
    return d ? d + 1 : "";
}

/* the first `cap` bytes of a file (one f_read).  0 ok, -1 cannot be read */
static int read_small(const char *path, void *buf, UINT cap, UINT *got)
{
    FIL f;
    UINT br = 0;
    FRESULT r;
    *got = 0;
    if (f_open(&f, path, FA_READ) != FR_OK) return -1;
    r = f_read(&f, buf, cap, &br);
    f_close(&f);
    if (r != FR_OK) return -1;
    *got = br;
    return 0;
}

int nm_scan_begin(nm_scan_t *sc, nm_list_t *list, int show_slot1)
{
    DIR d;
    FILINFO fi;
    static char ini_buf[SMALL_MAX];
    UINT got;
    memset(sc, 0, sizeof *sc);
    sc->list = list;
    sc->show_slot1 = show_slot1;
    nm_ini_defaults(&sc->ini);
    nm_list_init(list);
    if (f_opendir(&d, "0:/") != FR_OK) { sc->state = NM_SCAN_ABORTED; return -1; }
    for (;;) {
        fi.fname[0] = 0;
        if (f_readdir(&d, &fi) != FR_OK) { f_closedir(&d); sc->state = NM_SCAN_ABORTED; return -1; }
        if (!fi.fname[0]) break;
        if (!(fi.fattrib & AM_DIR) || (fi.fattrib & (AM_HID | AM_SYS)) || fi.fname[0] == '.') continue;
        {
            unsigned n = nm_slot_number(fi.fname);
            char err[8];
            if (n) {
                if (n == 1 && !show_slot1) continue;            /* folder 01 is this menu */
            } else if (ng_path_ok(fi.fname, err, sizeof err)) {
                continue;                                       /* not a name the firmware can be given */
            }
            nm_list_add(list, fi.fname, n);
        }
    }
    f_closedir(&d);
    if (read_small("0:/GDEMU.INI", ini_buf, sizeof ini_buf, &got) == 0) {
        sc->have_ini = 1;
        nm_ini_parse(&sc->ini, ini_buf, got);
    }
    if (ngfs_default_image(sc->default_image, sizeof sc->default_image))
        ngfs_norm_path(sc->default_image, sc->default_image, sizeof sc->default_image);
    else
        sc->default_image[0] = 0;
    nm_list_sort(list);
    for (int i = 0; i < list->count; i++)
        if (sc->default_image[0] && ieq(list->items[i].folder, sc->default_image)) list->items[i].flags |= NM_F_DEFAULT;
    sc->total = list->count;
    sc->state = list->count ? NM_SCAN_RUNNING : NM_SCAN_DONE;
    return 0;
}

/* One folder.  0 ok, -1 read error. */
static int scan_folder(nm_game_t *g)
{
    static uint8_t buf[SMALL_MAX];
    char dir[NG_PATH_BUF], path[NG_PATH_BUF + 16];
    char gdi_name[NG_FNAME_BUF] = "";
    int has_gdi = 0, has_cdi = 0, has_ngp = 0, has_ckp = 0, has_cfg = 0, has_nametxt = 0;
    DIR d;
    FILINFO fi;
    UINT got;
    char title[NM_TITLE_MAX + 1];

    snprintf(dir, sizeof dir, "0:/%s", g->folder);
    if (f_opendir(&d, dir) != FR_OK) return -1;
    for (;;) {
        fi.fname[0] = 0;
        if (f_readdir(&d, &fi) != FR_OK) { f_closedir(&d); return -1; }
        if (!fi.fname[0]) break;
        if (fi.fattrib & AM_DIR) continue;
        if (ieq(fi.fname, "NETGD.CKP")) has_ckp = 1;
        else if (ieq(fi.fname, "GAME.CFG")) has_cfg = 1;
        else if (ieq(fi.fname, "NAME.TXT")) has_nametxt = 1;
        else if (ieq(ext_of(fi.fname), "GDI")) {
            if (!has_gdi) snprintf(gdi_name, sizeof gdi_name, "%s", fi.fname);
            has_gdi = 1;
        } else if (ieq(ext_of(fi.fname), "CDI")) has_cdi = 1;
        else if (ieq(ext_of(fi.fname), "NGP")) has_ngp = 1;
    }
    f_closedir(&d);

    g->kind = has_gdi ? NM_KIND_GDI : has_cdi ? NM_KIND_CDI : NM_KIND_UNKNOWN;
    if (has_gdi || has_cdi) g->state = NM_ST_COMPLETE;
    else if (has_ngp || has_ckp) g->state = NM_ST_PARTIAL;
    else g->state = NM_ST_EMPTY;

    if (g->state == NM_ST_EMPTY) {
        snprintf(g->title, sizeof g->title, "(empty)");
        return 0;
    }
    if (g->state != NM_ST_COMPLETE) return 0;

    /* the title: GAME.CFG, NAME.TXT, the GDI's IP.BIN, else the folder name (already there) */
    title[0] = 0;
    if (has_cfg) {
        snprintf(path, sizeof path, "%s/GAME.CFG", dir);
        if (read_small(path, buf, sizeof buf, &got) == 0) nm_gamecfg_title((const char *)buf, got, title, sizeof title);
    }
    if (!title[0] && has_nametxt) {
        snprintf(path, sizeof path, "%s/NAME.TXT", dir);
        if (read_small(path, buf, sizeof buf, &got) == 0) nm_nametxt_title((const char *)buf, got, title, sizeof title);
    }
    if (!title[0] && has_gdi) {
        char track[NM_FOLDER_MAX + 1];
        unsigned ss = 0;
        snprintf(path, sizeof path, "%s/%s", dir, gdi_name);
        if (read_small(path, buf, sizeof buf, &got) == 0 && nm_gdi_track3((const char *)buf, got, track, sizeof track, &ss)) {
            snprintf(path, sizeof path, "%s/%s", dir, track);
            if (read_small(path, buf, ss, &got) == 0) nm_ipbin_title(buf, got, ss, title, sizeof title);
        }
    }
    if (title[0]) snprintf(g->title, sizeof g->title, "%s", title);
    return 0;
}

int nm_scan_step(nm_scan_t *sc)
{
    nm_list_t *l = sc->list;
    nm_game_t *g;
    if (sc->state != NM_SCAN_RUNNING) return sc->state == NM_SCAN_ABORTED ? -1 : 0;
    if (sc->next >= l->count) { sc->state = NM_SCAN_DONE; return 0; }
    g = &l->items[sc->next];
    if (scan_folder(g) != 0) {
        g->state = NM_ST_ERROR;
        sc->failed++;
        sc->fail_streak++;
        sc->next++;
        if (sc->fail_streak >= 3) { sc->state = NM_SCAN_ABORTED; return -1; }
    } else {
        sc->fail_streak = 0;
        if (g->state == NM_ST_EMPTY && !g->number) {            /* a named folder without an image is not a game */
            memmove(g, g + 1, (size_t)(l->count - sc->next - 1) * sizeof *g);
            l->count--;
            sc->total--;
            sc->generation++;
        } else {
            sc->next++;
        }
    }
    if (sc->next >= l->count) sc->state = NM_SCAN_DONE;
    return sc->state == NM_SCAN_RUNNING ? 1 : 0;
}
