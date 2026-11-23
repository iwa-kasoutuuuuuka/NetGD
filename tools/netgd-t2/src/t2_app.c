/* netgd-t2 core: see t2_app.h.  NOT run on hardware. */
#include <stdio.h>
#include <string.h>
#include "t2_app.h"
#include "nm_net.h"
#include "ngtp.h"
#include "nm_plat.h"
#include "fwctl.h"
#include "ngfs.h"
#include "ng_plat.h"


#define CONFIRM_HOLD_MS 1000
#define UI_EVERY_MS     200

void t2_cfg_defaults(t2_cfg_t *c, int tcpfix)
{
    memset(c, 0, sizeof *c);
    c->port = NGTP_PORT;
    c->code = 0;
    c->want_bytes = tcpfix ? NGTP_WANT_TCPFIX : NGTP_WANT_STOCK;
    c->depth = tcpfix ? NGTP_DEPTH_TCPFIX : NGTP_DEPTH_STOCK;
    c->idle_close_ms = 5u * 60u * 1000u;
    c->idle_exit_ms = 15u * 60u * 1000u;
    c->drive = 1;
    c->tcpfix = tcpfix;
    c->confirm_max_ms = 30000;
}

/* ---- the screen ---- */

static const char *state_text(const t2_app_t *a)
{
    switch (a->state) {
    case T2_S_NETUP: return "starting the network";
    case T2_S_LISTEN: return "opening the port";
    case T2_S_SERVE:
        if (a->busy) return "working";
        if (a->srv.connected) return a->srv.session ? "PC connected, session open" : "PC connected";
        return a->srv.session ? "waiting for the PC (session open)" : "waiting for the PC";
    case T2_S_CLOSE: return "closing";
    case T2_S_DONE: return "done";
    default: return "FAILED";
    }
}

static const char *result_text(int r)
{
    switch (r) {
    case T2_R_FINISH: return "finished by the PC";
    case T2_R_USER: return "left with B";
    case T2_R_IDLE: return "no frame from the PC for too long";
    case T2_R_NONET: return "no network (BBA / link / address)";
    case T2_R_NOLISTEN: return "cannot listen on the port";
    default: return "";
    }
}

static void draw(t2_app_t *a)
{
    nm_screen_t *s = &a->scr;
    char l[NM_COLS + 1];
    nm_screen_clear(s);
    nm_screen_row(s, 0, "netgd-t2 " T2_VERSION "  SD write over NGTP (T2)", NM_ATTR_INVERT);
    nm_screen_put_right(s, 0, a->cfg.tcpfix ? "tcpfix" : "stock tcp", NM_ATTR_INVERT);
    if (a->ip[0]) {
        snprintf(l, sizeof l, "IP %s  port %u  code %04u", a->ip, (unsigned)a->cfg.port, (unsigned)a->srv.code);
        nm_screen_row(s, 2, l, NM_ATTR_NORMAL);
    }
    snprintf(l, sizeof l, "WANT %u B x %d  rcvbuf %u", (unsigned)a->srv.want_max, a->srv.depth, (unsigned)nm_net_rcvbuf);
    nm_screen_row(s, 3, l, NM_ATTR_DIM);
    nm_screen_row(s, 4, state_text(a), a->state == T2_S_FAILED ? NM_ATTR_WARN : NM_ATTR_NORMAL);
    if (!a->cfg.drive) snprintf(l, sizeof l, "drive: not used (T2_NO_DRIVE build): no writes");
    else if (!a->fw_checked) snprintf(l, sizeof l, "drive: checked at the first write");
    else if (a->fw_ok) snprintf(l, sizeof l, "drive: NetGD OGDM/%u %.24s", (unsigned)a->fw.proto, a->fw.fwstr);
    else snprintf(l, sizeof l, "drive: NOT NetGD - writing refused (52 00 fw %08X)", (unsigned)a->gd_fw);
    nm_screen_row(s, 5, l, a->fw_checked && !a->fw_ok ? NM_ATTR_WARN : NM_ATTR_NORMAL);
    if (a->srv.client[0]) {
        snprintf(l, sizeof l, "client %.32s", a->srv.client);
        nm_screen_row(s, 6, l, NM_ATTR_DIM);
    }
    if (a->op[0]) nm_screen_row(s, 7, a->op, NM_ATTR_NORMAL);
    for (int i = 0; i < a->nlog; i++) nm_screen_row(s, 13 + i, a->log[i], NM_ATTR_DIM);
    if (a->state == T2_S_DONE || a->state == T2_S_FAILED) {
        snprintf(l, sizeof l, "%s: %s", a->state == T2_S_DONE ? "END" : "FAILED", result_text(a->result));
        nm_screen_row(s, 18, l, a->state == T2_S_FAILED ? NM_ATTR_WARN : NM_ATTR_NORMAL);
    } else {
        nm_screen_row(s, 18, ng_btnguard_stuck(&a->bguard) ? "B is stuck: ignored. Finish from the PC or reset"
                             : a->busy ? "B: stop safely (resume later)" : "B: close the session and leave", NM_ATTR_DIM);
    }
}

static void present(t2_app_t *a)
{
    draw(a);
    nm_plat_present(&a->scr);
    a->last_ui_ms = nm_plat_millis();
}

static void add_log(t2_app_t *a, const char *text)
{
    if (a->nlog == 5) { memmove(a->log[0], a->log[1], sizeof a->log[0] * 4); a->nlog = 4; }
    snprintf(a->log[a->nlog++], sizeof a->log[0], "%s", text);
}

void t2_log(t2_app_t *a, const char *text)
{
    add_log(a, text);
    present(a);
}

/* ---- the NGTP server's hooks ---- */

static void hk_line(void *ctx, const char *line, int throttled)
{
    t2_app_t *a = (t2_app_t *)ctx;
    if (throttled) return;
    add_log(a, line);
    present(a);
}

static void hk_progress(void *ctx, const ng_ui_t *u)
{
    t2_app_t *a = (t2_app_t *)ctx;
    unsigned pct = u->total ? (unsigned)(u->cum * 100 / u->total) : 0;
    snprintf(a->op, sizeof a->op, "%s %d/%d %3u%% %u KB/s %u/%u MB", u->op, u->fi, u->fn, pct, (unsigned)(u->rate / 1024),
             (unsigned)(u->cum >> 20), (unsigned)(u->total >> 20));
    if (nm_plat_millis() - a->last_ui_ms >= UI_EVERY_MS) present(a);
}

static int hk_stop(void *ctx)
{
    t2_app_t *a = (t2_app_t *)ctx;
    uint32_t pad = 0;
    int down = ng_plat_user_stop(&pad);
    return ng_btnguard_update(&a->bguard, pad, down, nm_plat_millis());
}

/* force on a protected folder: A released and then held for a second allows it, B or 30 s refuses.  A pad that reports A all the
   time never approves (ng_holdconfirm_t). */
static int hk_confirm(void *ctx, const char *what, const char *folder)
{
    t2_app_t *a = (t2_app_t *)ctx;
    uint32_t t0 = nm_plat_millis();
    ng_holdconfirm_t hold;
    char l[NM_COLS + 1];
    memset(&hold, 0, sizeof hold);
    for (;;) {
        uint32_t b, pad = nm_plat_pad(&b), now = nm_plat_millis();
        int refuse = ng_btnguard_update(&a->bguard, pad, (b & NM_BTN_B) != 0, now);
        draw(a);
        snprintf(l, sizeof l, "PC asks to %s the protected %.24s", what, folder);
        nm_screen_row(&a->scr, 9, l, NM_ATTR_WARN);
        nm_screen_row(&a->scr, 10, ng_btnguard_stuck(&hold.g) ? "Release A, then hold it 1 s to allow"
                                   : ng_btnguard_stuck(&a->bguard) ? "Hold A for 1 s to allow (B stuck: wait to refuse)"
                                   : "Hold A for 1 s to allow, B to refuse", NM_ATTR_WARN);
        nm_plat_present(&a->scr);
        if (refuse) return 0;
        if (ng_holdconfirm_update(&hold, pad, (b & NM_BTN_A) != 0, now, CONFIRM_HOLD_MS)) return 1;
        if (now - t0 >= a->cfg.confirm_max_ms) return 0;
        nm_plat_sleep(20);
    }
}

static void hk_busy(void *ctx, int busy)
{
    t2_app_t *a = (t2_app_t *)ctx;
    a->busy = busy;
    if (busy) a->op[0] = 0;
    present(a);
}

static void hk_idle(void *ctx)
{
    t2_app_t *a = (t2_app_t *)ctx;
    if (nm_plat_millis() - a->last_ui_ms >= UI_EVERY_MS) present(a);
}

static uint32_t hk_millis(void *ctx) { (void)ctx; return nm_plat_millis(); }

int t2_gate(t2_app_t *a, char *why, size_t whyn)
{
    if (!a->cfg.drive) {
        snprintf(why, whyn, "this build never talks to the drive (T2_NO_DRIVE): will not write");
        return 0;
    }
    if (!a->fw_checked) {
        /* Asked once (fw_probe_netgd, fwctl.h): 52 00 as the stock record has it, and E0 03 only when 52 00 says NetGD.
           A drive that is not NetGD gets no other packet from us. */
        uint32_t bw = 0, fw = 0;
        int r;
        a->fw_checked = 1;
        r = fw_probe_netgd(&a->fw, &bw, &fw);
        a->gd_fw = fw;
        if (r == FW_PROBE_NOANSWER) snprintf(a->fw_why, sizeof a->fw_why, "the drive did not answer 52 00: will not write");
        else if (r == FW_PROBE_OTHER)
            snprintf(a->fw_why, sizeof a->fw_why, "GDEMU %x.%02x.%x is not NetGD: will not write", (unsigned)(fw >> 24),
                     (unsigned)((fw >> 16) & 0xFF), (unsigned)((fw >> 8) & 0xFF));
        else if (r != FW_PROBE_NETGD) snprintf(a->fw_why, sizeof a->fw_why, "no OGDM VERSION: not NetGD, will not write");
        else if ((a->fw.caps & (FW_CAP_SESSION | FW_CAP_CHECKSUM)) != (FW_CAP_SESSION | FW_CAP_CHECKSUM))
            snprintf(a->fw_why, sizeof a->fw_why, "NetGD without write session / checksum: will not write");
        else { a->fw_ok = 1; a->srv.fw = &a->fw; }
    }
    if (!a->fw_ok) snprintf(why, whyn, "%s", a->fw_why);
    return a->fw_ok;
}

static int hk_may_write(void *ctx, char *why, size_t whyn) { return t2_gate((t2_app_t *)ctx, why, whyn); }

/* ---- the state machine ---- */

void t2_init(t2_app_t *a, const t2_cfg_t *cfg)
{
    ngtp_hooks_t hk;
    memset(a, 0, sizeof *a);
    a->cfg = *cfg;
    if (a->cfg.code > 9999) a->cfg.code %= 10000;
    memset(&hk, 0, sizeof hk);
    hk.ctx = a;
    hk.line = hk_line; hk.progress = hk_progress; hk.stop = hk_stop; hk.confirm = hk_confirm;
    hk.busy = hk_busy; hk.idle = hk_idle; hk.millis = hk_millis; hk.may_write = hk_may_write;
    ngtp_srv_init(&a->srv, &hk, NULL, a->cfg.code);
    ngtp_srv_set_window(&a->srv, a->cfg.want_bytes, a->cfg.depth);
    a->state = T2_S_NETUP;
}

const nm_screen_t *t2_screen(const t2_app_t *a) { return &a->scr; }

static void leave(t2_app_t *a, int result)
{
    a->result = result;
    a->state = T2_S_CLOSE;
}

int t2_step(t2_app_t *a)
{
    uint32_t now, btn, pad;
    int b, ev;
    switch (a->state) {
    case T2_S_NETUP:
        present(a);                                 /* "starting the network" is on the screen during the call */
        if (nm_net_up(a->ip, sizeof a->ip) != 0) { a->result = T2_R_NONET; a->state = T2_S_FAILED; present(a); return 0; }
        a->state = T2_S_LISTEN;
        present(a);
        return 1;
    case T2_S_LISTEN:
        if (nm_net_listen(a->cfg.port) != 0) { a->result = T2_R_NOLISTEN; a->state = T2_S_FAILED; present(a); return 0; }
        a->srv.last_frame_ms = nm_plat_millis();
        a->state = T2_S_SERVE;
        present(a);
        return 1;
    case T2_S_SERVE:
        ev = ngtp_srv_service(&a->srv);
        now = nm_plat_millis();
        pad = nm_plat_pad(&btn);
        b = ng_btnguard_update(&a->bguard, pad, (btn & NM_BTN_B) != 0, now);
        if (ev & NGTP_EV_FINISH) { leave(a, T2_R_FINISH); present(a); return 1; }
        if (ev & NGTP_EV_REQUEST) a->b_was_down = b;        /* a B that stopped an operation does not also leave */
        if (b && !a->b_was_down && !a->busy) { leave(a, T2_R_USER); present(a); return 1; }
        a->b_was_down = b;
        if (a->srv.code_expired) ngtp_srv_new_code(&a->srv, (uint16_t)((a->srv.code * 7919u + 1237u) % 10000u));
        if (a->srv.session && now - a->srv.last_frame_ms >= a->cfg.idle_close_ms) {
            ngtp_srv_end(&a->srv, 1);
            add_log(a, "session closed: idle");
        }
        if (now - a->srv.last_frame_ms >= a->cfg.idle_exit_ms) { leave(a, T2_R_IDLE); present(a); return 1; }
        if (now - a->last_ui_ms >= UI_EVERY_MS || ev) present(a);
        return 1;
    case T2_S_CLOSE:
        a->close_err = ngtp_srv_end(&a->srv, 1);    /* the default image back */
        if (a->srv.connected) ngtp_srv_drop(&a->srv);
        nm_net_stop();
        a->state = T2_S_DONE;
        present(a);
        return 0;
    default:
        return 0;
    }
}
