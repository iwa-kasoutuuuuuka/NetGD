/* netgd-recv (KOS build): write a game image to the NetGD SD card, T1 (dcload-ip /pc/).
   Design 6: start, screen, state.  Start with dc_auto + netgd-send (see README_ja.md).
   NOT run on hardware yet. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <kos.h>
#include <dc/video.h>
#include <dc/biosfont.h>
#include <dc/fs_dcload.h>
#include "xport_pc.h"
#include "ng_plat.h"

/* No INIT_NET, no INIT_NO_DCLOAD: dcload-ip keeps the BBA (design 3.3).  No INIT_CDROM:
   nothing may talk to the GD-ROM while the firmware has it ejected for the write session. */
KOS_INIT_FLAGS(INIT_DEFAULT & ~INIT_CDROM);

#define LINE_H 24
static int ui_ready;

static void draw(int row, const char *text)
{
    char buf[64];
    int y = 24 + row * LINE_H;
    memset(buf, ' ', 52);                       /* blank the old line */
    buf[52] = 0;
    bfont_draw_str(vram_s + y * 640 + 16, 640, 1, buf);
    bfont_draw_str(vram_s + y * 640 + 16, 640, 1, text);
}

static void ui_init(void)
{
    vid_set_mode(DM_640x480, PM_RGB565);
    vid_empty();
    ui_ready = 1;
    draw(0, "netgd-recv " NG_APP_VERSION "  (SD write over BBA)");
    draw(1, "B: stop safely   (resume later)");
}

static void ui_progress(const ng_ui_t *u)
{
    char l[64];
    int pct;
    if (!ui_ready) return;
    snprintf(l, sizeof l, "%s %d/%d", u->op, u->fi, u->fn);
    draw(3, l);
    snprintf(l, sizeof l, "%.40s", u->file ? u->file : "");
    draw(4, l);
    pct = u->total ? (int)(u->cum * 100 / u->total) : 0;
    {
        char bar[34];
        int n = pct * 30 / 100;
        memset(bar, '.', 30);
        memset(bar, '#', (size_t)n);
        bar[30] = 0;
        snprintf(l, sizeof l, "[%s] %3d%%", bar, pct);
    }
    draw(5, l);
    snprintf(l, sizeof l, "%u KB/s  %u/%u MB", u->rate / 1024, (unsigned)(u->cum >> 20), (unsigned)(u->total >> 20));
    draw(6, l);
    if (u->rate) {
        unsigned left = (unsigned)((u->total - u->cum) / u->rate);
        snprintf(l, sizeof l, "left about %u min %u s", left / 60, left % 60);
        draw(7, l);
    }
}

int main(int argc, char **argv)
{
    char jobdir[160];
    int rc;
    (void)argc; (void)argv;

    ui_init();
    if (dcload_type != DCLOAD_TYPE_IP) {
        draw(3, "Start me with dc-tool-ip (dcload-ip).");
        draw(4, "T2 (standalone) is not implemented.");
        thd_sleep(5000);
        return 0;
    }
    if (!ng_pc_find_jobdir(jobdir, sizeof jobdir)) {
        draw(3, "No job folder: /pc/netgd/<unit>/job.ngj");
        draw(4, "Run netgd-send prepare first.");
        printf("NGD:ERROR job code=ENOENT msg=\"no job.ngj under /pc/netgd\"\n");
        thd_sleep(5000);
        return 0;
    }
    draw(2, jobdir);
    rc = ng_pc_run(jobdir, ui_progress);
    draw(9, rc == 0 ? "DONE: all operations succeeded" :
            rc == 2 ? "STOPPED: nothing lost, run again to resume" : "FAILED: see the console / result.ngr");
    thd_sleep(3000);
    exit(0);                                    /* back to dcload so the PC can send the next thing */
}
