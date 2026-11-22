#include <stdio.h>
#include <string.h>
#include "nm_ui.h"
#include "nm_plat.h"
#include "nm_text.h"
#include "ngdev.h"
#include "ngfs.h"
#include "ngtp.h"
#include "nm_net.h"
#include "nm_disc.h"
#include "nm_art.h"
#include "ng_plat.h"

#define LIST_ROW0 2
#define LIST_ROWS 15
#define STATUS_ROW 17
#define HINT_ROW 18
#define TITLE_COLS 38
#define TITLE_COLS_ART (NM_ART_COL - 12)    /* the row is 12 + title columns wide: it ends where the art block starts */

void nm_ui_reset_text(unsigned reset_goto, char *out, size_t n)
{
    if (reset_goto == 0) snprintf(out, n, "Reset: stays in the game (reset_goto off)");
    else if (reset_goto == 1) snprintf(out, n, "Reset: back to this menu");
    else snprintf(out, n, "Reset: goes to %u", reset_goto);
}

void nm_ui_init(nm_ui_t *ui)
{
    memset(ui, 0, sizeof *ui);
    ui->state = NM_S_BOOT;
    ui->handoff_mode = NM_HANDOFF_DEFAULT;
    ui->idle_ms = 300000;
    ui->controller = 1;
    nm_input_init(&ui->in);
    nm_art_init(&ui->art);
    nm_ini_defaults(&ui->ini);
    nm_screen_clear(&ui->scr);
}

/* ---- drawing helpers ---- */

static void draw_frame(nm_ui_t *ui, const char *hint1, const char *hint2)
{
    char right[32];
    nm_screen_row(&ui->scr, 0, "NetGD Menu " NM_VERSION, NM_ATTR_INVERT);
    if (ui->gdemu_fw)
        snprintf(right, sizeof right, "fw %x.%x.%x ", (unsigned)(ui->gdemu_fw >> 24) & 0xFF, (unsigned)(ui->gdemu_fw >> 16) & 0xFF,
                 (unsigned)(ui->gdemu_fw >> 8) & 0xFF);
    else if (ui->fw.fwstr[0])
        snprintf(right, sizeof right, "fw %.20s ", ui->fw.fwstr);
    else
        right[0] = 0;
    nm_screen_put_right(&ui->scr, 0, right, NM_ATTR_INVERT);
    nm_screen_row(&ui->scr, HINT_ROW, hint1, NM_ATTR_DIM);
    nm_screen_row(&ui->scr, HINT_ROW + 1, hint2, NM_ATTR_DIM);
}

static void game_row(const nm_game_t *g, int selected, int tcols, char *out, int *attr)
{
    char t[NM_TITLE_MAX + 24], fit[TITLE_COLS + 1], num[8];
    const char *kind = g->kind == NM_KIND_GDI ? "GDI" : g->kind == NM_KIND_CDI ? "CDI" : "   ";
    *attr = selected ? NM_ATTR_INVERT : NM_ATTR_NORMAL;
    if (g->number) snprintf(num, sizeof num, "%04u", g->number); else snprintf(num, sizeof num, "    ");
    if (g->state == NM_ST_PARTIAL) snprintf(t, sizeof t, "%s (incomplete)", g->title);
    else if (g->state == NM_ST_ERROR) snprintf(t, sizeof t, "%s (read error)", g->title);
    else snprintf(t, sizeof t, "%s", g->title);
    if (!selected && (g->state == NM_ST_PARTIAL || g->state == NM_ST_EMPTY || g->state == NM_ST_ERROR)) *attr = NM_ATTR_DIM;
    nm_text_fit(fit, sizeof fit, t, tcols);
    snprintf(out, NM_COLS + 1, "%c%s  %-*s %s %c", selected ? '>' : ' ', num, tcols, fit,
             g->state == NM_ST_COMPLETE ? kind : "   ", (g->flags & NM_F_DEFAULT) ? '*' : ' ');
}

static void status_text(const nm_ui_t *ui, char *out, size_t n)
{
    const nm_scan_t *sc = &ui->scan;
    char reset[64], line[160];
    int warn = 0;
    if (!ui->controller) { snprintf(out, n, "No controller"); return; }
    if (ui->msg[0] && (int32_t)(ui->now - ui->msg_until) < 0) { snprintf(out, n, "%s", ui->msg); return; }
    if (ui->state == NM_S_NET_IDLE || ui->state == NM_S_NET_BUSY) { out[0] = 0; return; }
    if (sc->state == NM_SCAN_RUNNING) {
        snprintf(out, n, "Scanning %d/%d ...", sc->next, sc->total);
        return;
    }
    if (sc->state == NM_SCAN_ABORTED) {
        snprintf(out, n, "SD read error (scan stopped)  %d games", ui->list.count);
        return;
    }
    snprintf(line, sizeof line, "%d game%s", ui->list.count, ui->list.count == 1 ? "" : "s");
    if (ui->list.truncated) { strcat(line, "  list truncated"); warn = 1; }
    if (ui->list.duplicate) { size_t l = strlen(line); snprintf(line + l, sizeof line - l, "  duplicate number %02u", ui->list.duplicate); warn = 1; }
    if (sc->failed) { size_t l = strlen(line); snprintf(line + l, sizeof line - l, "  %d unreadable", sc->failed); warn = 1; }
    if (!warn) {
        size_t l = strlen(line);
        nm_ui_reset_text(ui->ini.reset_goto, reset, sizeof reset);
        snprintf(line + l, sizeof line - l, "  %s", reset);
    }
    snprintf(out, n, "%s", line);
}

static void draw_status(nm_ui_t *ui, int attr)
{
    char s[160], fit[NM_COLS + 1];
    status_text(ui, s, sizeof s);
    nm_text_fit(fit, sizeof fit, s, NM_COLS);
    nm_screen_row(&ui->scr, STATUS_ROW, fit, attr);
}

static void draw_list(nm_ui_t *ui)
{
    nm_screen_clear(&ui->scr);
    for (int r = 0; r < LIST_ROWS; r++) {
        int i = ui->top + r;
        char line[NM_COLS + 8];
        int attr;
        if (i >= ui->list.count) break;
        game_row(&ui->list.items[i], i == ui->cursor, ui->art_layout ? TITLE_COLS_ART : TITLE_COLS, line, &attr);
        nm_screen_row(&ui->scr, LIST_ROW0 + r, line, attr);
        if (ui->art_layout)                                         /* the art block stays blank (the platform draws the picture there) */
            for (int c = NM_ART_COL; c < NM_COLS; c++) { ui->scr.ch[LIST_ROW0 + r][c] = ' '; ui->scr.attr[LIST_ROW0 + r][c] = NM_ATTR_NORMAL; }
    }
    if (ui->list.count == 0 && ui->scan.state == NM_SCAN_DONE)
        nm_screen_put(&ui->scr, LIST_ROW0 + 1, 2, "No game folders on the card.", NM_ATTR_NORMAL);
    draw_frame(ui, "A:Start  X:Info  Y:Network  START:BIOS menu", "Up/Down:Select  Left/Right, L/R:Page");
    draw_status(ui, NM_ATTR_NORMAL);
}

static void draw_info(nm_ui_t *ui)
{
    char reset[64], buf[96];
    const char *def = ui->scan.default_image[0] ? ui->scan.default_image : "(none)";
    nm_screen_clear(&ui->scr);
    draw_frame(ui, "A:Switch hand-off  B:Back", "");
    nm_screen_printf(&ui->scr, 2, 1, NM_ATTR_NORMAL, "Firmware      %.24s", ui->fw.fwstr);
    nm_screen_printf(&ui->scr, 3, 1, NM_ATTR_NORMAL, "Protocol      OGDM %u  features 0x%02X%s%s%s", ui->fw.proto, ui->fw.caps,
                     (ui->fw.caps & FW_CAP_SESSION) ? " session" : "", (ui->fw.caps & FW_CAP_CHECKSUM) ? " checksum" : "",
                     (ui->fw.caps & FW_CAP_DMA) ? " dma" : "");
    nm_ui_reset_text(ui->ini.reset_goto, reset, sizeof reset);
    nm_screen_printf(&ui->scr, 4, 1, NM_ATTR_NORMAL, "%s", reset);
    nm_screen_printf(&ui->scr, 5, 1, NM_ATTR_NORMAL, "GDEMU.INI      %s", ui->scan.have_ini ? "found" : "not found");
    snprintf(buf, sizeof buf, "open_time %u ms%s  detect_time %u ms%s", ui->ini.open_time, ui->scan.ini.open_time_set ? "" : " (default)",
             ui->ini.detect_time, ui->scan.ini.detect_time_set ? "" : " (default)");
    nm_screen_printf(&ui->scr, 6, 1, NM_ATTR_NORMAL, "%.51s", buf);
    nm_screen_printf(&ui->scr, 7, 1, NM_ATTR_NORMAL, "Default image %.37s", def);
    nm_screen_printf(&ui->scr, 8, 1, NM_ATTR_NORMAL, "Games         %d", ui->list.count);
    if (ui->stack_ok)                   /* a stack-hw firmware: high-water mark since its reset (PLAN A-1) */
        nm_screen_printf(&ui->scr, 9, 1, NM_ATTR_NORMAL, "Stack         %u of %u bytes used%s", (unsigned)ui->stack.used,
                         (unsigned)ui->stack.reserved, ui->stack.used >= ui->stack.reserved ? " OVERRUN" : "");
    nm_screen_printf(&ui->scr, 10, 1, NM_ATTR_NORMAL, "Hand-off      %s",
                     ui->handoff_mode == NM_HANDOFF_REBOOT ? "restart the console" : "BIOS menu");
    draw_status(ui, NM_ATTR_NORMAL);
}

static void draw_text_screen(nm_ui_t *ui, const char *l1, const char *l2, const char *l3, const char *l4, const char *l5)
{
    nm_screen_clear(&ui->scr);
    draw_frame(ui, "START:BIOS menu", "");
    nm_screen_put(&ui->scr, 3, 2, l1, NM_ATTR_WARN);
    if (l2) nm_screen_put(&ui->scr, 5, 2, l2, NM_ATTR_NORMAL);
    if (l3) nm_screen_put(&ui->scr, 6, 2, l3, NM_ATTR_NORMAL);
    if (l4) nm_screen_put(&ui->scr, 7, 2, l4, NM_ATTR_NORMAL);
    if (l5) nm_screen_put(&ui->scr, 8, 2, l5, NM_ATTR_NORMAL);
}

/* ---- cursor ---- */

static void remember_cursor(nm_ui_t *ui)
{
    if (ui->cursor >= 0 && ui->cursor < ui->list.count)
        snprintf(ui->cursor_folder, sizeof ui->cursor_folder, "%s", ui->list.items[ui->cursor].folder);
    else
        ui->cursor_folder[0] = 0;
}

static void fix_view(nm_ui_t *ui)
{
    int n = ui->list.count;
    if (ui->cursor >= n) ui->cursor = n - 1;
    if (ui->cursor < 0) ui->cursor = 0;
    if (ui->cursor < ui->top) ui->top = ui->cursor;
    if (ui->cursor >= ui->top + LIST_ROWS) ui->top = ui->cursor - LIST_ROWS + 1;
    if (ui->top > n - LIST_ROWS) ui->top = n - LIST_ROWS;
    if (ui->top < 0) ui->top = 0;
}

static void move_cursor(nm_ui_t *ui, int delta, int page)
{
    int n = ui->list.count;
    if (!n) return;
    ui->cursor += delta;
    if (page) ui->top += delta;
    if (ui->cursor < 0) ui->cursor = 0;
    if (ui->cursor >= n) ui->cursor = n - 1;
    fix_view(ui);
    remember_cursor(ui);
}

/* the scan removed items: find the item the cursor was on again */
static void relocate_cursor(nm_ui_t *ui)
{
    int i = ui->cursor_folder[0] ? nm_list_find_folder(&ui->list, ui->cursor_folder) : -1;
    if (i >= 0) ui->cursor = i;
    fix_view(ui);
    remember_cursor(ui);
}

/* ---- the states ---- */

static void to_nosd(nm_ui_t *ui, const char *why)
{
    snprintf(ui->why, sizeof ui->why, "%s", why);
    ui->state = NM_S_NOSD;
}

static void start_scan(nm_ui_t *ui)
{
    ui->cursor = ui->top = 0;
    nm_art_cancel(&ui->art);
    if (nm_scan_begin(&ui->scan, &ui->list, NM_SHOW_SLOT1) != 0) { to_nosd(ui, "read error"); return; }
    ui->ini = ui->scan.ini;
    ui->scan_gen_seen = ui->scan.generation;
    remember_cursor(ui);
    ui->state = NM_S_LIST;
}

static void boot_work(nm_ui_t *ui)
{
    uint32_t boot = 0, fwv = 0;
    char why[96];
    if (fw_version(&ui->fw) != 0) { ui->state = NM_S_NOFW; return; }
    if (ngdev_init() != 0) { to_nosd(ui, "no disk"); return; }
    if (ngfs_mount_light(why, sizeof why) != NG_OK) { to_nosd(ui, why); return; }
    ui->sd_mounted = 1;
    if (fw_gdemu_version(&boot, &fwv) == 0) ui->gdemu_fw = fwv;
    start_scan(ui);
}

/* ---- launching ---- */

static void folder_label(const nm_ui_t *ui, char *out, size_t n)
{
    if (ui->dlg_number) snprintf(out, n, "%02u", ui->dlg_number);
    else snprintf(out, n, "%s", ui->dlg_folder);
}

static void set_msg(nm_ui_t *ui, const char *text, uint32_t ms)
{
    snprintf(ui->msg, sizeof ui->msg, "%s", text);
    ui->msg_until = ui->now + ms;
}

static void show_dialog(nm_ui_t *ui, int kind)
{
    ui->dialog = kind;
    ui->state = NM_S_DIALOG;
}

static void enter_handoff(nm_ui_t *ui, const char *msg, int target)
{
    snprintf(ui->start_msg, sizeof ui->start_msg, "%s", msg);
    ui->handoff_target = target;
    ui->handoff_since = ui->now;
    ui->state = NM_S_HANDOFF;
}

static void begin_launch(nm_ui_t *ui)
{
    const nm_game_t *g;
    if (ui->list.count == 0) return;
    g = &ui->list.items[ui->cursor];
    ui->dlg_number = g->number;
    snprintf(ui->dlg_folder, sizeof ui->dlg_folder, "%s", g->folder);
    if (g->state == NM_ST_EMPTY) { show_dialog(ui, NM_DLG_NOIMAGE); return; }
    if (g->state == NM_ST_PARTIAL) { show_dialog(ui, NM_DLG_INCOMPLETE); return; }
    nm_art_cancel(&ui->art);                                        /* its file must be closed before the volume goes */
    nm_launch_begin(&ui->launch, g->number, g->folder, nm_ini_ready_limit_ms(&ui->ini));
    ui->sd_mounted = 0;
    ui->state = NM_S_LAUNCH;
}

/* back to the list after a dialog: the light mount comes back (the menu's own image is still the mounted one) */
static void back_to_list(nm_ui_t *ui)
{
    char why[96];
    if (!ui->sd_mounted) {
        if (ngfs_mount_light(why, sizeof why) != NG_OK) { to_nosd(ui, why); return; }
        ui->sd_mounted = 1;
    }
    ui->state = NM_S_LIST;
}

static void launch_frame(nm_ui_t *ui)
{
    char label[NM_FOLDER_MAX + 1], msg[NM_COLS + 1];
    switch (nm_launch_step(&ui->launch)) {
    case NM_L_OK:
        folder_label(ui, label, sizeof label);
        snprintf(msg, sizeof msg, "Starting %s ...", label);
        enter_handoff(ui, msg, ui->handoff_mode);
        break;
    case NM_L_NOIMAGE: show_dialog(ui, NM_DLG_NOIMAGE); break;
    case NM_L_FWERR: show_dialog(ui, NM_DLG_FWERR); break;
    case NM_L_NOTREADY: show_dialog(ui, NM_DLG_NOTREADY); break;
    default: break;
    }
}

/* ---- the transfer screen (design 7.4) ---- */

static void draw_net(nm_ui_t *ui)
{
    char line[NM_COLS + 40];
    const ng_ui_t *u = &ui->prog;
    int busy = ui->state == NM_S_NET_BUSY;
    nm_screen_clear(&ui->scr);
    draw_frame(ui, busy ? "B:Stop safely (a later push resumes)" : "B:Leave this screen", "Send with: netgd-send push <image> --host <IP> --code <code>");
    nm_screen_printf(&ui->scr, 2, 1, NM_ATTR_NORMAL, "IP %s  port %u  code %04u", ui->ip, (unsigned)NGTP_PORT, (unsigned)ui->code);
    if (!ui->srv.connected) snprintf(line, sizeof line, "waiting for the PC%s", ui->srv.session ? " (session open)" : "");
    else if (!ui->srv.hello) snprintf(line, sizeof line, "client connected");
    else snprintf(line, sizeof line, "client %.20s%s", ui->srv.client, ui->srv.session ? "  session open" : "");
    nm_screen_put(&ui->scr, 3, 1, line, NM_ATTR_NORMAL);
    if (ui->prog_valid) {
        int pct = u->total ? (int)(u->cum * 100 / u->total) : 0;
        nm_screen_printf(&ui->scr, 5, 1, NM_ATTR_NORMAL, "%s %d/%d", ui->prog_op, u->fi, u->fn);
        nm_screen_printf(&ui->scr, 6, 1, NM_ATTR_NORMAL, "%.50s", ui->prog_file);
        nm_screen_bar(&ui->scr, 7, 1, 40, pct, NM_ATTR_NORMAL);
        nm_screen_printf(&ui->scr, 7, 42, NM_ATTR_NORMAL, "%3d%%", pct);
        nm_screen_printf(&ui->scr, 8, 1, NM_ATTR_NORMAL, "%u KB/s  %u/%u MB", (unsigned)(u->rate / 1024), (unsigned)(u->cum >> 20),
                         (unsigned)(u->total >> 20));
        if (u->rate && u->total > u->cum) {
            unsigned left = (unsigned)((u->total - u->cum) / u->rate);
            nm_screen_printf(&ui->scr, 9, 1, NM_ATTR_NORMAL, "left about %u min %u s", left / 60, left % 60);
        }
    }
    for (int i = 0; i < ui->nlog; i++) nm_screen_printf(&ui->scr, 11 + i, 1, NM_ATTR_NORMAL, "%.51s", ui->log[i]);
    draw_status(ui, NM_ATTR_NORMAL);
}

static void draw_net_confirm(nm_ui_t *ui)
{
    uint32_t held = nm_input_held_ms(&ui->in, NM_BTN_A, ui->now);
    nm_screen_clear(&ui->scr);
    draw_frame(ui, "Hold A for 1 second: allow", "B (or 30 seconds): refuse");
    nm_screen_printf(&ui->scr, 3, 2, NM_ATTR_WARN, "The PC asks to %s a protected folder:", ui->confirm_what);
    nm_screen_printf(&ui->scr, 5, 2, NM_ATTR_NORMAL, "%.48s", ui->confirm_folder);
    nm_screen_put(&ui->scr, 7, 2, "The folder is the card's default image or 01.", NM_ATTR_NORMAL);
    nm_screen_bar(&ui->scr, 9, 2, 20, (int)(held > 1000 ? 100 : held / 10), NM_ATTR_NORMAL);
}

static void draw_state(nm_ui_t *ui)
{
    switch (ui->state) {
    case NM_S_LIST: draw_list(ui); break;
    case NM_S_INFO: draw_info(ui); break;
    case NM_S_NOFW:
        draw_text_screen(ui, "NetGD firmware not found", "This menu works with the NetGD firmware only.",
                         "The console did not answer as a NetGD board", "(a stock GDEMU is not supported).", "Nothing was read from the SD card.");
        break;
    case NM_S_NOSD: {
        char w[120];
        snprintf(w, sizeof w, "SD card: %s", ui->why);
        draw_text_screen(ui, w, "The menu needs a FAT32 card (first partition)", "in the NetGD board's SD slot.", NULL, NULL);
        break;
    }
    case NM_S_LAUNCH: {
        char label[NM_FOLDER_MAX + 1];
        folder_label(ui, label, sizeof label);
        nm_screen_clear(&ui->scr);
        draw_frame(ui, "", "");
        nm_screen_printf(&ui->scr, 3, 2, NM_ATTR_NORMAL, "Mounting %s ...", label);
        if (ui->launch.phase == 1)
            nm_screen_printf(&ui->scr, 5, 2, NM_ATTR_NORMAL, "Waiting for the drive ... %u s",
                             (unsigned)(nm_launch_waited_ms(&ui->launch) / 1000));
        break;
    }
    case NM_S_DIALOG: {
        char label[NM_FOLDER_MAX + 1];
        folder_label(ui, label, sizeof label);
        nm_screen_clear(&ui->scr);
        draw_frame(ui, ui->dialog == NM_DLG_NOTREADY ? "A:Wait again  B:Back to the list" : "A:Back to the list", "");
        switch (ui->dialog) {
        case NM_DLG_NOIMAGE:
            nm_screen_printf(&ui->scr, 4, 2, NM_ATTR_WARN, "No image in folder %s", label);
            nm_screen_put(&ui->scr, 6, 2, "The firmware found no disc image to mount there.", NM_ATTR_NORMAL);
            break;
        case NM_DLG_INCOMPLETE:
            nm_screen_printf(&ui->scr, 4, 2, NM_ATTR_WARN, "Image in folder %s is incomplete", label);
            nm_screen_put(&ui->scr, 6, 2, "Send it again (netgd-send push, resume).", NM_ATTR_NORMAL);
            break;
        case NM_DLG_FWERR:
            nm_screen_printf(&ui->scr, 4, 2, NM_ATTR_WARN, "Firmware error while mounting %s", label);
            nm_screen_printf(&ui->scr, 6, 2, NM_ATTR_NORMAL, "sense key %u", ui->launch.sense);
            break;
        default:
            nm_screen_put(&ui->scr, 4, 2, "Drive not ready", NM_ATTR_WARN);
            nm_screen_printf(&ui->scr, 6, 2, NM_ATTR_NORMAL, "%s did not become READY in %u s.", label,
                             (unsigned)(ui->launch.limit_ms / 1000));
            break;
        }
        break;
    }
    case NM_S_CONFIRM_BIOS:
        nm_screen_clear(&ui->scr);
        draw_frame(ui, "A:Yes, go to the BIOS menu", "B:No, stay in the menu");
        nm_screen_put(&ui->scr, 4, 2, "Go back to the BIOS menu?", NM_ATTR_WARN);
        break;
    case NM_S_HANDOFF:
        nm_screen_clear(&ui->scr);
        draw_frame(ui, "", "");
        nm_screen_put(&ui->scr, 4, 2, ui->start_msg, NM_ATTR_NORMAL);
        break;
    case NM_S_NET_IDLE:
    case NM_S_NET_BUSY: draw_net(ui); break;
    case NM_S_NET_CONFIRM: draw_net_confirm(ui); break;
    case NM_S_FATAL:
        draw_text_screen(ui, ui->fatal, "The console is left as it is.", "Reset it: the firmware brings the card back.", NULL, NULL);
        break;
    default: break;
    }
}

/* ---- the transfer: hooks for the NGTP server and the screens' flow ---- */

static void net_redraw(nm_ui_t *ui)
{
    ui->now = nm_plat_millis();
    draw_state(ui);
    nm_plat_present(&ui->scr);
}

static int line_worth_showing(const char *l)
{
    static const char *const keep[] = { "NGD:ERROR", "NGD:DONE", "NGD:FINISH", "NGD:MOUNT pct=100", "NGD:FILE", "NGD:STOP",
                                        "NGD:VERIFY", "NGD:DELETE", "NGD:RESUME", "NGD:NOTE", "NGD:HELLO" };
    for (unsigned i = 0; i < sizeof keep / sizeof keep[0]; i++)
        if (!strncmp(l, keep[i], strlen(keep[i]))) return 1;
    return 0;
}

static void hk_line(void *ctx, const char *line, int throttled)
{
    nm_ui_t *ui = (nm_ui_t *)ctx;
    if (throttled || !line_worth_showing(line)) return;
    if (ui->nlog == 5) {
        for (int i = 1; i < 5; i++) memcpy(ui->log[i - 1], ui->log[i], sizeof ui->log[0]);
        ui->nlog = 4;
    }
    snprintf(ui->log[ui->nlog++], sizeof ui->log[0], "%s", line + 4);          /* without the "NGD:" */
}

static void hk_progress(void *ctx, const ng_ui_t *u)
{
    nm_ui_t *ui = (nm_ui_t *)ctx;
    ui->prog = *u;
    snprintf(ui->prog_op, sizeof ui->prog_op, "%s", u->op ? u->op : "");
    snprintf(ui->prog_file, sizeof ui->prog_file, "%s", u->file ? u->file : "");
    ui->prog_valid = 1;
    net_redraw(ui);
}

static int hk_stop(void *ctx)
{
    nm_ui_t *ui = (nm_ui_t *)ctx;
    int stop = ng_plat_user_stop();
    if (stop) ui->b_wait_release = 1;       /* the same press must not also leave the screen when the operation has stopped */
    return stop;
}

static void hk_busy(void *ctx, int busy)
{
    nm_ui_t *ui = (nm_ui_t *)ctx;
    ui->state = busy ? NM_S_NET_BUSY : NM_S_NET_IDLE;
    if (busy) ui->prog_valid = 0;
    net_redraw(ui);
}

static void hk_idle(void *ctx)
{
    nm_ui_t *ui = (nm_ui_t *)ctx;
    net_redraw(ui);
}

static uint32_t hk_millis(void *ctx) { (void)ctx; return nm_plat_millis(); }

static void hk_before_session(void *ctx)
{
    nm_ui_t *ui = (nm_ui_t *)ctx;
    nm_art_cancel(&ui->art);
    if (ui->sd_mounted) { ngfs_unmount(); ui->sd_mounted = 0; }
}

/* The PC asked for force on a protected folder: A held for a second allows it, B or 30 s refuses (design 8.3). */
static int hk_confirm(void *ctx, const char *what, const char *folder)
{
    nm_ui_t *ui = (nm_ui_t *)ctx;
    int saved = ui->state, allow = 0;
    uint32_t t0 = nm_plat_millis();
    snprintf(ui->confirm_what, sizeof ui->confirm_what, "%s", what);
    snprintf(ui->confirm_folder, sizeof ui->confirm_folder, "%s", folder);
    ui->state = NM_S_NET_CONFIRM;
    ui->confirm_since = t0;
    for (;;) {
        uint32_t ev = nm_input_update(&ui->in, nm_plat_buttons(), nm_plat_millis());
        ui->now = nm_plat_millis();
        if (ev & NM_BTN_B) break;
        if (nm_input_held_ms(&ui->in, NM_BTN_A, ui->now) >= 1000) { allow = 1; break; }
        if (ui->now - t0 >= 30000) break;
        net_redraw(ui);
        nm_plat_sleep(16);
    }
    ui->state = saved;
    net_redraw(ui);
    return allow;
}

uint16_t nm_ui_code_from_seed(uint32_t seed)
{
    seed = seed * 1103515245u + 12345u;
    return (uint16_t)((seed >> 8) % 10000u);
}

static void net_start(nm_ui_t *ui)
{
    ngtp_hooks_t hk;
    int r;
    if (nm_plat_dcload()) { set_msg(ui, "Started from dcload: use netgd-recv", 4000); return; }
    /* net_dhcp_request can block for a long time: say so before it starts (R104 finding 1) */
    nm_screen_clear(&ui->scr);
    draw_frame(ui, "", "");
    nm_screen_put(&ui->scr, 3, 2, "Starting network ...", NM_ATTR_NORMAL);
    nm_screen_put(&ui->scr, 5, 2, "(getting an IP address by DHCP; this can take up to a minute)", NM_ATTR_DIM);
    nm_plat_set_art(NULL);
    nm_plat_present(&ui->scr);
    r = nm_net_up(ui->ip, sizeof ui->ip);
    if (r == NM_NET_NODEV) { set_msg(ui, "BBA not found", 3000); return; }
    if (r != 0) { set_msg(ui, "No IP address (Y: try again)", 3000); return; }
    if (nm_net_listen(NGTP_PORT) != 0) { set_msg(ui, "Cannot listen on the port", 3000); return; }
    nm_net_udp_listen(NGTP_PORT);                                   /* discovery is optional: no message when it fails */
    nm_art_cancel(&ui->art);
    if (ui->sd_mounted) { ngfs_unmount(); ui->sd_mounted = 0; }     /* the light mount and a write session never coexist */
    ui->code = nm_ui_code_from_seed(nm_plat_entropy());
    memset(&hk, 0, sizeof hk);
    hk.ctx = ui; hk.line = hk_line; hk.progress = hk_progress; hk.stop = hk_stop; hk.confirm = hk_confirm;
    hk.busy = hk_busy; hk.idle = hk_idle; hk.millis = hk_millis; hk.before_session = hk_before_session;
    ngtp_srv_init(&ui->srv, &hk, &ui->fw, ui->code);
    ui->nlog = 0;
    ui->prog_valid = 0;
    ui->msg[0] = 0;
    ui->state = NM_S_NET_IDLE;
}

/* Leave the transfer screen: close the session if it is open, stop the network, read the card again. */
static void net_close(nm_ui_t *ui)
{
    char why[96];
    int closed_anyway = 0;
    ng_err_t e = ngtp_srv_end(&ui->srv, 1);
    nm_net_close_client();
    nm_net_stop();
    if (e != NG_OK) {
        /* The end command failed, but the failure may have come after the firmware closed the session (the mode 2 mount of a
           folder, a lost answer): ask it.  Closed = the card is the console's again, so go back to the list; still open or no
           answer = the card is not safe to read (R104 C). */
        ng_fwstatus_t st;
        if (fw_status(&st) != 0 || (st.flags & FW_ST_SESSION)) {
            snprintf(ui->fatal, sizeof ui->fatal, "Could not close the session: reset the console");
            ui->state = NM_S_FATAL;
            return;
        }
        closed_anyway = 1;
    }
    if (ngfs_mount_light(why, sizeof why) != NG_OK) { to_nosd(ui, why); return; }
    ui->sd_mounted = 1;
    ui->msg[0] = 0;
    start_scan(ui);
    if (closed_anyway) set_msg(ui, "The end command failed, but the session is closed", 6000);
}

static void net_frame(nm_ui_t *ui)
{
    int ev;
    if (ui->events & NM_BTN_B) { net_close(ui); return; }
    nm_disc_service(ui->srv.connected);
    ev = ngtp_srv_service(&ui->srv);
    if (ui->srv.code_expired) {
        ui->code = nm_ui_code_from_seed(nm_plat_entropy() + (uint32_t)ui->now);
        ngtp_srv_new_code(&ui->srv, ui->code);
    }
    if (ui->state != NM_S_NET_IDLE && ui->state != NM_S_NET_BUSY) return;
    ui->state = NM_S_NET_IDLE;
    if (ev & NGTP_EV_FINISH) { net_close(ui); return; }
    if ((ev & NGTP_EV_CLOSED) && !(ev & NGTP_EV_FINISH)) set_msg(ui, "connection closed", 4000);
    ui->now = nm_plat_millis();
    /* signed: a request may have finished after this frame's clock was read */
    if (ui->srv.session && (int32_t)(ui->now - ui->srv.last_frame_ms) >= (int32_t)ui->idle_ms) net_close(ui);
}

/* The box art of the game under the cursor: choose, load a piece per frame (design step 11).  Only in the list. */
static void art_frame(nm_ui_t *ui)
{
    const char *want = (ui->sd_mounted && ui->list.count > 0 && ui->cursor < ui->list.count) ? ui->list.items[ui->cursor].folder : NULL;
    nm_art_select(&ui->art, want, ui->now);
    nm_art_step(&ui->art, ui->now);
    if (nm_art_pixels(&ui->art)) ui->art_layout = 1;
}

static void scan_frame(nm_ui_t *ui)
{
    nm_scan_step(&ui->scan);
    if (ui->scan.generation != ui->scan_gen_seen) {
        ui->scan_gen_seen = ui->scan.generation;
        relocate_cursor(ui);
    } else {
        fix_view(ui);
    }
}

void nm_ui_step(nm_ui_t *ui)
{
    ui->now = nm_plat_millis();
    ui->controller = nm_plat_controller();
    ui->events = nm_input_update(&ui->in, nm_plat_buttons(), ui->now);
    if (ui->b_wait_release) {           /* B stopped an operation: the press that did it must not count as a new B (R104 D) */
        if (nm_plat_buttons() & NM_BTN_B) ui->events &= ~(uint32_t)NM_BTN_B;
        else ui->b_wait_release = 0;
    }

    switch (ui->state) {
    case NM_S_BOOT:
        if (ui->boot_stage == 0) {
            nm_screen_clear(&ui->scr);
            draw_frame(ui, "", "");
            nm_screen_put(&ui->scr, 3, 2, "Looking for the NetGD firmware ...", NM_ATTR_NORMAL);
            ui->boot_stage = 1;
        } else {
            boot_work(ui);
            if (ui->state == NM_S_BOOT) ui->state = NM_S_NOFW;     /* not reached */
        }
        break;

    case NM_S_LIST:
        /* One transition per frame (R104 B): when several buttons come in the same frame, the first of A, Y, X, START wins and
           the others are dropped, so "X and A together" never shows the info screen and starts the game too.  The cursor
           moves only when no transition was asked for. */
        if (ui->events & NM_BTN_A) begin_launch(ui);
        else if (ui->events & NM_BTN_Y) net_start(ui);
        else if (ui->events & NM_BTN_X) {
            ui->state = NM_S_INFO;
            /* read once per visit: one 20-byte data-in, only from a firmware that announces it */
            ui->stack_ok = (ui->fw.caps & FW_CAP_STACK) && fw_stack(&ui->stack) == 0;
        }
        else if (ui->events & NM_BTN_START) ui->state = NM_S_CONFIRM_BIOS;
        else {
            if (ui->events & NM_BTN_UP) move_cursor(ui, -1, 0);
            if (ui->events & NM_BTN_DOWN) move_cursor(ui, 1, 0);
            if (ui->events & (NM_BTN_LEFT | NM_BTN_LT)) move_cursor(ui, -LIST_ROWS, 1);
            if (ui->events & (NM_BTN_RIGHT | NM_BTN_RT)) move_cursor(ui, LIST_ROWS, 1);
        }
        if (ui->state == NM_S_LIST || ui->state == NM_S_INFO || ui->state == NM_S_CONFIRM_BIOS) scan_frame(ui);
        if (ui->state == NM_S_LIST) art_frame(ui);
        break;

    case NM_S_INFO:
        if (ui->events & NM_BTN_B) ui->state = NM_S_LIST;
        else if (ui->events & NM_BTN_A) ui->handoff_mode = ui->handoff_mode == NM_HANDOFF_REBOOT ? NM_HANDOFF_BIOS : NM_HANDOFF_REBOOT;
        scan_frame(ui);
        break;

    case NM_S_LAUNCH:
        launch_frame(ui);
        break;

    case NM_S_DIALOG:
        if (ui->dialog == NM_DLG_NOTREADY && (ui->events & NM_BTN_A)) {
            nm_launch_wait_again(&ui->launch);
            ui->state = NM_S_LAUNCH;
        } else if (ui->events & (NM_BTN_A | NM_BTN_B)) {
            back_to_list(ui);
        }
        break;

    case NM_S_CONFIRM_BIOS:
        if (ui->events & NM_BTN_A) {
            nm_art_cancel(&ui->art);
            if (ui->sd_mounted) { ngfs_unmount(); ui->sd_mounted = 0; }
            enter_handoff(ui, "Returning to the BIOS menu ...", NM_HANDOFF_BIOS);
        } else if (ui->events & NM_BTN_B) {
            ui->state = NM_S_LIST;
        }
        if (ui->state == NM_S_CONFIRM_BIOS) scan_frame(ui);
        break;

    case NM_S_HANDOFF:
        if (ui->now - ui->handoff_since >= 100) nm_plat_handoff(ui->handoff_target);
        break;

    case NM_S_NET_IDLE:
        net_frame(ui);
        break;

    case NM_S_NOFW:
    case NM_S_NOSD:
        if (ui->events & NM_BTN_START) nm_plat_handoff(NM_HANDOFF_BIOS);
        break;

    default:
        break;
    }

    nm_plat_set_art(ui->state == NM_S_LIST ? nm_art_pixels(&ui->art) : NULL);
    draw_state(ui);
    nm_plat_present(&ui->scr);
}
