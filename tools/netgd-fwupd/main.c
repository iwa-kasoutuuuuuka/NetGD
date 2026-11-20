/* netgd-fwupd: update NetGD's firmware from the Dreamcast through the secondary IDE device
 * (vendor opcode 0xF9, mcu/opengdemu/src/fwupdate.rs + mcu/fwupdater). KOS 2.3.0 glue only;
 * everything that can be tested on a PC is in fwcore.c. NOT run on hardware yet: see README_ja.md.
 *
 * Flow: read the .ogfw from /pc/ -> check it on the console -> check that the secondary IDE
 * device is NetGD (IDENTIFY) -> confirmation screen, explicit button combo held for 3 s ->
 * handover, send each image, verify, reset (only after everything verified) -> result screen
 * and a result file in /pc/. */
#include <kos.h>
#include <dc/biosfont.h>
#include <dc/fs_dcload.h>
#include <dc/maple.h>
#include <dc/maple/controller.h>
#include <dc/video.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "fwcore.h"
#include "g1dev.h"
#include "resout.h"

#define FWUPD_VERSION "0.1"

/* No INIT_CDROM: nothing else may talk to the GD-ROM (primary) device while NetGD is in its
 * updater, and cdrom_init would send BIOS commands to it (same reasoning as tools/gdprobe). INIT_NO_DCLOAD
 * is NOT set: dcload-ip provides /pc/ over the BBA. */
KOS_INIT_FLAGS(INIT_DEFAULT & ~INIT_CDROM);

/* Folder on the PC as seen through dc-tool-ip's /pc/ (build.ps1 sets HOST_DIR_RAW, e.g.
 * /pc/work/OpenGDEmu/tools/netgd-fwupd/host). No spaces. */
#define STR_(x) #x
#define STR(x)  STR_(x)
#ifdef HOST_DIR_RAW
#define HOST_DIR STR(HOST_DIR_RAW)
#else
#define HOST_DIR "/pc"
#endif
/* The update image. Build-time default (make NG_FWIMAGE=/pc/dir/x.ogfw), or at run time an
 * "image=<path>" line in HOST_DIR/netgd-fwupd.cfg. */
#ifdef FW_IMAGE_RAW
#define FW_IMAGE_DEFAULT STR(FW_IMAGE_RAW)
#else
#define FW_IMAGE_DEFAULT HOST_DIR "/netgd.ogfw"
#endif
#define FW_CFG_PATH HOST_DIR "/netgd-fwupd.cfg"

#define COL_WHITE  0xFFFF
#define COL_RED    0xF800
#define COL_YELLOW 0xFFE0
#define COL_GREEN  0x07E0
#define COL_GREY   0xAD55

#define LINE_H     24
#define ROWS       19
#define MARGIN_X   16
#define SCREEN_CELLS 50           /* (640 - 2 * 16) / 12 */
#define HOLD_MS    3000

/* the combination that starts an update: A + X + START + D-pad UP. (Not A+B+X+Y+START,
 * which KOS treats as "exit".) */
#define COMBO (CONT_A | CONT_X | CONT_START | CONT_DPAD_UP)

static uint8_t g_file[FW_MAX_FILE + 1];
static fw_file g_fw;
static char g_model[48];
static char g_image_path[200];
static char g_result_path[220];

static resout g_res;
static int g_res_ok;

/* ------------------------------------------------------------------------ screen ---- */

static void clear_rows(int r0, int r1)
{
    int y0 = 12 + r0 * LINE_H, y1 = 12 + (r1 + 1) * LINE_H;
    if (y1 > 480) y1 = 480;
    if (y0 < y1) memset(vram_s + y0 * 640, 0, (size_t)(y1 - y0) * 640 * 2);
}

static void draw(int row, uint32_t color, const char *euc)
{
    if (row < 0 || row >= ROWS) return;
    bfont_set_foreground_color(color);
    bfont_draw_str(vram_s + (12 + row * LINE_H) * 640 + MARGIN_X, 640, 1, euc);
}

/* Draw wrapped text from `row`; returns the next free row. */
static int draw_wrap(int row, uint32_t color, const char *euc)
{
    char lines[10][112];
    int n = fw_text_wrap(euc, SCREEN_CELLS, &lines[0][0], sizeof lines[0], 10), i;
    for (i = 0; i < n; i++) draw(row + i, color, lines[i]);
    return row + n;
}

static int drawf(int row, uint32_t color, const char *fmt, ...)
{
    char b[160];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(b, sizeof b, fmt, ap);
    va_end(ap);
    return draw_wrap(row, color, b);
}

static void ui_init(void)
{
    vid_set_mode(DM_640x480, PM_RGB565);
    vid_empty();
    bfont_set_encoding(BFONT_CODE_EUC);
    bfont_set_background_color(0x0000);
    bfont_set_foreground_color(COL_WHITE);
}

static void ui_title(void)
{
    clear_rows(0, ROWS - 1);
    draw(0, COL_WHITE, fw_msg_euc(FWM_UI_TITLE));
}

static uint32_t pad_buttons(void)
{
    uint32_t b = 0;
    int i;
    for (i = 0; i < 4; i++) {
        maple_device_t *c = maple_enum_type(i, MAPLE_FUNC_CONTROLLER);
        cont_state_t *st;
        if (!c) break;
        st = (cont_state_t *)maple_dev_status(c);
        if (st) b |= st->buttons;
    }
    return b;
}

static void wait_start_and_exit(int code)
{
    draw(ROWS - 1, COL_GREY, fw_msg_euc(FWM_UI_PRESS_START));
    while (pad_buttons() & CONT_START) thd_sleep(30);
    while (!(pad_buttons() & CONT_START)) thd_sleep(30);
    g1dev_deselect();
    exit(code);
}

/* ------------------------------------------------------------------ result file ---- */

static void res_text(const char *s)
{
    if (!g_res_ok) return;
    if (resout_write(&g_res, s, strlen(s)) != 0 || resout_sync(&g_res) != 0) {
        g_res_ok = 0;
        printf("[netgd-fwupd] WARNING: writing the result file failed\n");
    }
}

static void res_line(const char *s)
{
    res_text(s);
    res_text("\n");
}

static void open_result_file(void)
{
    int n;
    FILE *f;
    g_res_ok = 0;
    for (n = 1; n <= 999; n++) {
        snprintf(g_result_path, sizeof g_result_path, HOST_DIR "/fwupd_result_%03d.txt", n);
        f = fopen(g_result_path, "rb");
        if (!f) break;                      /* first free number */
        fclose(f);
    }
    if (resout_open(&g_res, resout_stdio_io(), g_result_path) == 0) {
        g_res_ok = 1;
        printf("[netgd-fwupd] result file: %s\n", g_result_path);
    } else {
        printf("[netgd-fwupd] cannot open %s (does the folder exist on the PC?)\n", g_result_path);
    }
}

/* ----------------------------------------------------------------------- run env ---- */

static void env_log(void *ctx, const char *line)
{
    (void)ctx;
    printf("[netgd-fwupd] %s\n", line);
    res_line(line);
}

static void env_progress(void *ctx, const fw_progress *p)
{
    char bar[44];
    int pct = p->blocks_total ? (int)((uint64_t)p->blocks_done * 100u / p->blocks_total) : 0, n = pct * 40 / 100;
    int row;
    (void)ctx;
    clear_rows(1, ROWS - 1);
    draw(1, COL_RED, fw_msg_euc(FWM_UI_SENDING));
    row = 3;
    row = drawf(row, COL_WHITE, "%s %d/%d", fw_msg_euc(FWM_UI_PROGRESS), p->image + 1, p->nimages);
    row = draw_wrap(row, COL_WHITE, fw_msg_euc(p->msgid));
    memset(bar, '.', 40);
    memset(bar, '#', (size_t)n);
    bar[40] = 0;
    row = drawf(row, COL_GREEN, "[%s] %3d%%", bar, pct);
    row = drawf(row, COL_WHITE, "%lu / %lu (512 %s)", (unsigned long)p->blocks_done,
                (unsigned long)p->blocks_total, fw_msg_euc(FWM_UI_BYTES));
    if (p->attempt > 0)
        row = drawf(row, COL_YELLOW, "%s (%d)", fw_msg_euc(FWM_UI_RETRY), p->attempt);
    draw_wrap(ROWS - 4, COL_YELLOW, fw_msg_euc(FWM_UI_WARN_POWER));
    (void)row;
}

/* ---------------------------------------------------------------------- file I/O ---- */

static void *rd_open(const char *path) { return fopen(path, "rb"); }
static size_t rd_read(void *f, void *b, size_t n) { return fread(b, 1, n, (FILE *)f); }
static int rd_error(void *f) { return ferror((FILE *)f); }
static void rd_close(void *f) { fclose((FILE *)f); }
static const fw_rd g_rd = { rd_open, rd_read, rd_error, rd_close };

static void choose_image_path(void)
{
    char cfg[512];
    size_t n;
    FILE *f;
    strncpy(g_image_path, FW_IMAGE_DEFAULT, sizeof g_image_path - 1);
    g_image_path[sizeof g_image_path - 1] = 0;
    f = fopen(FW_CFG_PATH, "rb");
    if (!f) return;
    n = fread(cfg, 1, sizeof cfg - 1, f);
    cfg[n] = 0;
    fclose(f);
    if (fw_cfg_image_path(cfg, g_image_path, sizeof g_image_path))
        printf("[netgd-fwupd] image path from %s\n", FW_CFG_PATH);
}

/* ------------------------------------------------------------------------- flows ---- */

static void finish_without_update(fw_outcome oc, int msgid)
{
    fw_report rep;
    char sum[1024];
    memset(&rep, 0, sizeof rep);
    rep.failed_image = -1;
    rep.outcome = oc;
    fw_summary_text(sum, sizeof sum, &rep);
    res_text(sum);
    draw_wrap(3, COL_YELLOW, fw_msg_euc(msgid));
    if (g_res_ok) resout_close(&g_res);
    wait_start_and_exit(1);
}

static void show_image_error(const fw_vres *res)
{
    char line[256];
    int m = fw_verr_msgid(res->err), row;
    ui_title();
    row = draw_wrap(2, COL_RED, fw_msg_euc(FWM_UI_IMAGE_BAD));
    row = draw_wrap(row + 1, COL_RED, fw_msg_euc(m));
    drawf(row + 1, COL_GREY, "%s (image %d)", g_image_path, res->index);
    snprintf(line, sizeof line, "image_error=%d index=%d en=%s", (int)res->err, res->index, fw_msg_en(m));
    res_line(line);
    snprintf(line, sizeof line, "image_error_ja=%s", fw_msg_ja(m));
    res_line(line);
    if (g_res_ok) resout_close(&g_res);
    wait_start_and_exit(1);
}

/* The confirmation screen. Returns 1 to go, 0 to cancel. */
static int confirm(void)
{
    int row, i;
    uint32_t held = 0;
    uint32_t b;
    ui_title();
    row = 2;
    row = drawf(row, COL_WHITE, "%s: %s", fw_msg_euc(FWM_UI_DEVICE), g_model);
    {
        size_t pl = strlen(g_image_path);
        row = drawf(row, COL_WHITE, "%s: %s%s", fw_msg_euc(FWM_UI_FILE), pl > 40 ? "..." : "",
                    g_image_path + (pl > 40 ? pl - 40 : 0));
    }
    row = drawf(row, COL_WHITE, "%s: %s", fw_msg_euc(FWM_UI_TARGET), fw_part_name(g_fw.part));
    for (i = 0; i < g_fw.n; i++) {
        const fw_header *h = &g_fw.img[i].h;
        row = drawf(row, COL_WHITE, "[%d] %s", i, fw_msg_euc(h->kind == FW_KIND_CODE ? FWM_UI_IMG_CODE : FWM_UI_IMG_BITSTREAM));
        row = drawf(row, COL_GREY, "    %lu %s  %s %lu  crc %08lx", (unsigned long)h->len, fw_msg_euc(FWM_UI_BYTES),
                    fw_msg_euc(FWM_UI_VERSION), (unsigned long)h->version, (unsigned long)h->crc);
    }
    if (g_fw.part == FW_PART_4E && g_fw.n == 1)
        row = draw_wrap(row, COL_YELLOW, fw_msg_euc(FWM_UI_WARN_NO_BITSTREAM));
    row = draw_wrap(row, COL_RED, fw_msg_euc(FWM_UI_WARN_POWER));
    row = draw_wrap(row, COL_RED, fw_msg_euc(FWM_UI_WARN_SWD));
    row = draw_wrap(row, COL_YELLOW, fw_msg_euc(FWM_UI_WARN_UNTESTED));
    /* worst case (two images, wrapped warnings) ends at row 13: see tests (layout) */
    draw_wrap(ROWS - 5, COL_WHITE, fw_msg_euc(FWM_UI_CONFIRM_TITLE));
    draw_wrap(ROWS - 4, COL_GREEN, fw_msg_euc(FWM_UI_CONFIRM_HOLD));
    draw_wrap(ROWS - 1, COL_GREY, fw_msg_euc(FWM_UI_CONFIRM_CANCEL));

    /* the buttons must be let go first, so a press from the previous screen cannot count */
    while (pad_buttons() & (COMBO | CONT_B)) thd_sleep(30);
    for (;;) {
        b = pad_buttons();
        if ((b & COMBO) == COMBO) {
            held += 30;
            clear_rows(ROWS - 2, ROWS - 2);
            {
                char bar[34];
                int n = (int)(held * 30 / HOLD_MS);
                if (n > 30) n = 30;
                memset(bar, '.', 30);
                memset(bar, '#', (size_t)n);
                bar[30] = 0;
                drawf(ROWS - 2, COL_YELLOW, "[%s] %s", bar, fw_msg_euc(FWM_UI_HOLDING));
            }
            if (held >= HOLD_MS) return 1;
        } else {
            if (held) clear_rows(ROWS - 2, ROWS - 2);
            held = 0;
            if (b & CONT_B) return 0;
        }
        thd_sleep(30);
    }
}

static void show_final(const fw_report *rep)
{
    int row, mid = fw_outcome_msgid(rep);
    uint32_t col = (rep->outcome == FWR_OK || rep->outcome == FWR_OK_UNCONFIRMED) ? COL_GREEN : COL_RED;
    ui_title();
    row = drawf(2, COL_WHITE, "%s", fw_msg_euc(FWM_UI_RESULT));
    row = draw_wrap(row, col, fw_msg_euc(mid));
    row++;
    if (rep->failed_image >= 0 || rep->io_rc != FWIO_OK) {
        if (rep->io_rc != FWIO_OK) {
            row = draw_wrap(row, COL_YELLOW, fw_msg_euc(fw_io_msgid(rep->io_rc)));
        } else {
            row = drawf(row, COL_YELLOW, "image %d  status 0x%02x  error 0x%02x  %s", rep->failed_image,
                        rep->dev_status, rep->dev_error, fw_status_name(rep->dev_error));
            row = draw_wrap(row, COL_YELLOW, fw_msg_euc(fw_status_msgid(rep->dev_error)));
        }
    }
    if (rep->retries_used) row = drawf(row, COL_GREY, "%s x%d", fw_msg_euc(FWM_UI_RETRY), rep->retries_used);
    if (g_res_ok) drawf(ROWS - 3, COL_GREY, "%s: %s", fw_msg_euc(FWM_UI_RESULT_FILE), g_result_path);
    else draw_wrap(ROWS - 3, COL_YELLOW, fw_msg_euc(FWM_UI_RESULT_FILE_FAIL));
}

int main(int argc, char **argv)
{
    fw_env env;
    fw_options opt;
    fw_report rep;
    fw_vres vres;
    char text[1536];
    size_t flen = 0;
    int rc, ident;
    (void)argc;
    (void)argv;

    ui_init();
    ui_title();
    printf("\n[netgd-fwupd] version %s, KOS %s\n", FWUPD_VERSION, KOS_VERSION_STRING);
    if (dcload_type == DCLOAD_TYPE_NONE) {
        draw_wrap(2, COL_RED, fw_msg_euc(FWM_UI_NO_DCLOAD));
        printf("[netgd-fwupd] %s\n", fw_msg_en(FWM_UI_NO_DCLOAD));
        thd_sleep(8000);
        return 1;
    }

    choose_image_path();
    open_result_file();
    res_line("# netgd-fwupd " FWUPD_VERSION);

    /* ---- 1. the image ---- */
    draw_wrap(2, COL_WHITE, fw_msg_euc(FWM_UI_LOADING));
    printf("[netgd-fwupd] image: %s\n", g_image_path);
    rc = fw_read_all(&g_rd, g_image_path, g_file, sizeof g_file, &flen);
    if (rc != 0) {
        ui_title();
        draw_wrap(2, COL_RED, fw_msg_euc(rc == -1 ? FWM_V_NOFILE : FWM_V_READ));
        drawf(5, COL_GREY, "%s", g_image_path);
        res_line(rc == -1 ? "image_error=nofile" : "image_error=read");
        res_line(rc == -1 ? fw_msg_ja(FWM_V_NOFILE) : fw_msg_ja(FWM_V_READ));
        if (g_res_ok) resout_close(&g_res);
        wait_start_and_exit(1);
    }
    if (fw_parse_file(g_file, flen, &g_fw, &vres) != 0) show_image_error(&vres);
    fw_describe_text(text, sizeof text, g_image_path, flen, &g_fw, NULL);
    res_text(text);

    /* ---- 2. is the secondary device NetGD? ---- */
    memset(&env, 0, sizeof env);
    g1dev_fill_env(&env);
    env.log = env_log;
    env.progress = env_progress;
    draw_wrap(4, COL_WHITE, fw_msg_euc(FWM_UI_CHECKING));
    ident = fw_check_device(&env, g_model, sizeof g_model);
    printf("[netgd-fwupd] secondary device: \"%s\" (%s)\n", g_model, ident == 0 ? "NetGD" : ident > 0 ? "other" : "no answer");
    snprintf(text, sizeof text, "device_model=%s", g_model);
    res_line(text);
    if (ident != 0) finish_without_update(ident > 0 ? FWR_NOT_NETGD : FWR_IDENT_FAIL,
                                          ident > 0 ? FWM_OUT_NOT_NETGD : FWM_OUT_IDENT_FAIL);

    /* ---- 3. confirmation: nothing has been sent to the F9 opcode so far ---- */
    if (!confirm()) finish_without_update(FWR_CANCELLED, FWM_OUT_CANCELLED);

    /* ---- 4. the update ---- */
    clear_rows(1, ROWS - 1);
    draw(1, COL_RED, fw_msg_euc(FWM_UI_SENDING));
    fw_options_default(&opt);
    res_line("update=start");
    rc = fw_update(&env, &g_fw, &opt, &rep);
    (void)rc;
    fw_summary_text(text, sizeof text, &rep);
    res_text(text);
    show_final(&rep);
    if (g_res_ok && resout_close(&g_res) != 0) printf("[netgd-fwupd] WARNING: closing the result file failed\n");
    printf("[netgd-fwupd] finished: outcome %d\n", (int)rep.outcome);
    wait_start_and_exit(rep.outcome == FWR_OK || rep.outcome == FWR_OK_UNCONFIRMED ? 0 : 1);
    return 0;
}
