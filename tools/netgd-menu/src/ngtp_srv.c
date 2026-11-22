#include <stdio.h>
#include <string.h>
#include "nm.h"
#include "nm_net.h"
#include "ngtp.h"
#include "ngtp_srv.h"
#include "ngfs.h"
#include "manifest.h"

#define FRAME_TIMEOUT_MS 15000      /* the rest of a frame must follow its header within this */
#define DATA_TIMEOUT_MS  30000      /* DATA must follow a WANT within this */
#define MFTEXT_MAX       32768

static uint8_t rx[NGTP_MAX_FRAME];
static uint8_t tx[1024];
static char mftext[MFTEXT_MAX + 1];
static ng_manifest_t man;

static uint32_t now_ms(ngtp_srv_t *s) { return s->hk.millis ? s->hk.millis(s->hk.ctx) : 0; }

/* ---- sending ---- */

static int send_frame(ngtp_srv_t *s, uint8_t type, uint8_t flags, uint32_t seq, const void *p, uint32_t n)
{
    size_t t;
    if (!s->connected || s->broken) return -1;
    t = ngtp_frame_build(tx, sizeof tx, type, flags, seq, p, n);
    if (!t || nm_net_send_all(tx, (uint32_t)t) != 0) { s->broken = 1; return -1; }
    return 0;
}

void ngtp_srv_drop(ngtp_srv_t *s)
{
    if (s->connected) nm_net_close_client();
    s->connected = 0;
    s->hello = 0;
    s->broken = 1;
}

/* ERROR <NAME> <text>, then the connection is closed */
static void send_error(ngtp_srv_t *s, uint32_t seq, const char *name, const char *text)
{
    char b[160];
    int n = snprintf(b, sizeof b, "%s %s", name, text);
    if (n < 0) n = 0;
    send_frame(s, NGTP_ERROR, 0, seq, b, (uint32_t)(n >= (int)sizeof b ? (int)sizeof b - 1 : n));
    ngtp_srv_drop(s);
}

static void send_done(ngtp_srv_t *s, uint32_t seq, ng_err_t e)
{
    uint8_t b[24];
    const char *name = e == NG_OK ? "" : ng_err_name(e);
    size_t l = strlen(name);
    b[0] = e == NG_OK ? 1 : 0;
    memcpy(b + 1, name, l);
    send_frame(s, NGTP_DONE, 0, seq, b, (uint32_t)(1 + l));
}

/* ---- the sink and the source handed to the receive core ---- */

static void sk_line(void *ctx, const char *line, int throttled)
{
    ngtp_srv_t *s = (ngtp_srv_t *)ctx;
    if (s->hk.line) s->hk.line(s->hk.ctx, line, throttled);
    send_frame(s, NGTP_LINE, throttled ? NGTP_F_THROTTLED : 0, s->seq, line, (uint32_t)strlen(line));
}

/* 1 = stop at the next safe point: the user pressed B, the PC asked, or the link is gone */
static int sk_poll(void *ctx)
{
    ngtp_srv_t *s = (ngtp_srv_t *)ctx;
    return s->broken || s->abort_asked || (s->hk.stop && s->hk.stop(s->hk.ctx));
}

static void sk_ui(void *ctx, const ng_ui_t *u)
{
    ngtp_srv_t *s = (ngtp_srv_t *)ctx;
    if (s->hk.progress) s->hk.progress(s->hk.ctx, u);
}

static uint32_t sk_millis(void *ctx) { return now_ms((ngtp_srv_t *)ctx); }

static ng_sink_t sink_of(ngtp_srv_t *s)
{
    ng_sink_t sk;
    sk.ctx = s; sk.line = sk_line; sk.poll = sk_poll; sk.ui = sk_ui; sk.millis = sk_millis;
    return sk;
}

/* The source handed to ng_put: the receive core reads, this side asks the PC (WANT) and takes its answers (DATA).
   WANTs go out in chunks of want_max bytes at fixed offsets from where the core opened the file, never past the
   manifest size, with up to `depth` of them outstanding: with depth 2 the PC sends the next chunk while the core writes
   the last one to the card.  The DATA frames must come back in the order of the WANTs.  Bytes of a DATA that the core
   did not ask for yet wait in `held`.  Before the file changes, and before DONE, the outstanding WANTs are drained (their
   DATA is read and dropped) so that no DATA frame is left in the stream when the next request is read. */
static uint8_t held[NGTP_MAX_DATA];

static int issue_want(ngtp_srv_t *s)
{
    uint8_t w[16];
    uint64_t left = s->src_size - s->issue_off;
    uint32_t n = s->want_max;
    int slot;
    if (left < n) n = (uint32_t)left;
    ngtp_put16(w, (uint32_t)s->src_idx);
    ngtp_put16(w + 2, 0);
    ngtp_put64(w + 4, s->issue_off);
    ngtp_put32(w + 12, n);
    if (send_frame(s, NGTP_WANT, 0, s->seq, w, sizeof w)) return -1;
    slot = (s->qh + s->nq) % NGTP_DEPTH_MAX;
    s->q[slot].off = s->issue_off;
    s->q[slot].len = n;
    s->nq++;
    s->issue_off += n;
    s->wants++;
    return 0;
}

/* The DATA answering the oldest outstanding WANT, into rx.  Its data length, or -1 (the connection is closed). */
static int32_t take_data(ngtp_srv_t *s)
{
    ngtp_hdr_t h;
    uint32_t waited = 0, dlen;
    const uint8_t *p;
    int r;
    for (;;) {                                  /* slices, so the screen stays alive while the PC is slow */
        r = nm_net_recv_all(rx, NGTP_HDR_LEN, 100);
        if (r == 0) break;
        if (r != NM_NET_AGAIN) { ngtp_srv_drop(s); return -1; }
        if (s->hk.idle) s->hk.idle(s->hk.ctx);
        waited += 100;
        if (waited >= DATA_TIMEOUT_MS) { ngtp_srv_drop(s); return -1; }
    }
    if (ngtp_hdr_parse(rx, &h) != NGTP_OK) { send_error(s, s->seq, "EPROTO", "bad frame header"); return -1; }
    if (nm_net_recv_all(rx + NGTP_HDR_LEN, h.len + 4, FRAME_TIMEOUT_MS) != 0) { ngtp_srv_drop(s); return -1; }
    if (ngtp_frame_check(rx, rx + NGTP_HDR_LEN, h.len, rx + NGTP_HDR_LEN + h.len) != NGTP_OK) {
        send_error(s, s->seq, "EBADCRC", "frame CRC mismatch");
        return -1;
    }
    if (h.type != NGTP_DATA || h.seq != s->seq || h.len < 12) {
        send_error(s, s->seq, "ESTATE", "expected the DATA that answers the WANT");
        return -1;
    }
    p = rx + NGTP_HDR_LEN;
    dlen = h.len - 12;
    if (ngtp_get16(p) != (uint32_t)s->src_idx || ngtp_get64(p + 4) != s->q[s->qh].off || dlen > s->q[s->qh].len) {
        send_error(s, s->seq, "ESTATE", "DATA does not match the WANT (file, offset or length)");
        return -1;
    }
    if (h.flags & NGTP_F_ABORT) s->abort_asked = 1;
    if (dlen < s->q[s->qh].len) s->src_eof = 1;     /* the PC's file is shorter than the manifest says */
    s->qh = (s->qh + 1) % NGTP_DEPTH_MAX;
    s->nq--;
    s->data_frames++;
    return (int32_t)dlen;
}

/* read and drop the DATA of every outstanding WANT (quietly: a broken link just ends the connection) */
static void drain_wants(ngtp_srv_t *s)
{
    while (s->nq > 0) {
        if (s->broken || !s->connected || take_data(s) < 0) { s->nq = 0; break; }
    }
    s->held_len = s->held_pos = 0;
}

static int src_open(void *ctx, int idx, uint64_t off)
{
    ngtp_srv_t *s = (ngtp_srv_t *)ctx;
    drain_wants(s);
    s->src_idx = idx;
    s->src_size = (idx >= 0 && idx < man.nfiles) ? man.files[idx].size : 0;
    s->issue_off = off;
    s->src_eof = 0;
    s->qh = s->nq = 0;
    return 0;
}

static void src_close(void *ctx) { drain_wants((ngtp_srv_t *)ctx); }

static int32_t src_read(void *ctx, void *buf, uint32_t n)
{
    ngtp_srv_t *s = (ngtp_srv_t *)ctx;
    int32_t dlen;
    if (s->broken || !s->connected) return -1;
    if (s->held_pos < s->held_len) {
        uint32_t k = s->held_len - s->held_pos;
        if (k > n) k = n;
        memcpy(buf, held + s->held_pos, k);
        s->held_pos += k;
        return (int32_t)k;
    }
    while (!s->src_eof && s->nq < s->depth && s->issue_off < s->src_size)
        if (issue_want(s)) return -1;
    if (s->nq == 0) return 0;                   /* end of the file (or the PC ended it early) */
    dlen = take_data(s);
    if (dlen < 0) return -1;
    {
        const uint8_t *d = rx + NGTP_HDR_LEN + 12;
        uint32_t k = (uint32_t)dlen < n ? (uint32_t)dlen : n;
        memcpy(buf, d, k);
        if ((uint32_t)dlen > k) {               /* the rest waits for the next read (rx is reused by the next frame) */
            memcpy(held, d + k, (uint32_t)dlen - k);
            s->held_len = (uint32_t)dlen - k;
            s->held_pos = 0;
        }
        return (int32_t)k;
    }
}

/* ---- requests ---- */

static ng_err_t ensure_session(ngtp_srv_t *s, const ng_sink_t *sk)
{
    ng_err_t e;
    if (s->session) return NG_OK;
    if (s->hk.may_write) {
        char why[96] = "writing is not allowed";
        if (!s->hk.may_write(s->hk.ctx, why, sizeof why)) {
            ng_emit_error(sk, "session", NG_E_FW, 0, 0, why);
            return NG_E_FW;
        }
    }
    if (s->hk.before_session) s->hk.before_session(s->hk.ctx);
    e = ng_session_open(sk);
    if (e == NG_OK) s->session = 1;
    return e;
}

/* the folder argument of a request: the payload bytes from `off`, as a C string */
static int folder_arg(const uint8_t *p, uint32_t n, uint32_t off, char *out, size_t cap)
{
    uint32_t l;
    if (n < off) return -1;
    l = n - off;
    if (l >= cap) return -1;
    memcpy(out, p + off, l);
    out[l] = 0;
    return strlen(out) == l ? 0 : -1;
}

static void do_hello(ngtp_srv_t *s, uint32_t seq, const uint8_t *p, uint32_t n)
{
    uint8_t b[64];
    uint32_t l;
    if (s->hello) { send_error(s, seq, "ESTATE", "HELLO twice"); return; }
    if (n < 4) { send_error(s, seq, "EPROTO", "short HELLO"); return; }
    if (ngtp_get16(p) != NGTP_VERSION) { send_error(s, seq, "EVERSION", "NGTP version 1 only"); return; }
    if (ngtp_get16(p + 2) != s->code) {
        if (++s->auth_fail >= 3) s->code_expired = 1;
        send_error(s, seq, "EAUTH", "wrong code");
        return;
    }
    l = n - 4 < sizeof s->client - 1 ? n - 4 : (uint32_t)sizeof s->client - 1;
    memcpy(s->client, p + 4, l);
    s->client[l] = 0;
    for (uint32_t i = 0; i < l; i++) if ((uint8_t)s->client[i] < 0x20 || (uint8_t)s->client[i] > 0x7E) s->client[i] = '?';
    s->hello = 1;
    s->auth_fail = 0;
    memset(b, 0, sizeof b);
    ngtp_put16(b, NGTP_VERSION);
    ngtp_put16(b + 2, s->session ? 1 : 0);
    ngtp_put32(b + 4, NGTP_MAX_DATA);
    b[8] = s->fw ? s->fw->proto : 0;
    b[9] = s->fw ? s->fw->caps : 0;
    if (s->fw) memcpy(b + 10, s->fw->fwstr, 24);
    memcpy(b + 34, NM_VERSION, sizeof NM_VERSION - 1);
    send_frame(s, NGTP_HELLO_OK, 0, seq, b, 34 + (uint32_t)(sizeof NM_VERSION - 1));
}

/* a request that needs the session; returns the final error to report in DONE */
static ng_err_t do_put(ngtp_srv_t *s, const ng_sink_t *sk, const uint8_t *p, uint32_t n)
{
    char tag[5], err[96];
    ng_src_t src;
    ng_err_t e;
    int mode = p[0], forceflag = p[1] & 1;
    if (n < 8 + 16 || n - 8 > MFTEXT_MAX || mode > 2) {
        ng_emit_error(sk, "put", NG_E_MANIFEST, 0, 0, "bad PUT request");
        return NG_E_MANIFEST;
    }
    for (int i = 0; i < 4; i++) {
        if (p[4 + i] < '0' || p[4 + i] > '9') {
            ng_emit_error(sk, "put", NG_E_MANIFEST, 0, 0, "the tag is not 4 digits");
            return NG_E_MANIFEST;
        }
        tag[i] = (char)p[4 + i];
    }
    tag[4] = 0;
    memcpy(mftext, p + 8, n - 8);          /* DATA frames reuse the receive buffer: keep our own copy */
    mftext[n - 8] = 0;
    if (ng_manifest_parse(mftext, n - 8, &man, err, sizeof err)) {
        ng_emit_error(sk, tag, NG_E_MANIFEST, 0, 0, err);
        return NG_E_MANIFEST;
    }
    ng_cfg.allow_frag = (p[1] & 2) != 0;
    ng_cfg.force = 0;
    if (forceflag && ngfs_protected(man.image))
        ng_cfg.force = s->hk.confirm ? s->hk.confirm(s->hk.ctx, "overwrite", man.image) : 0;
    src.ctx = s; src.open = src_open; src.read = src_read; src.close = src_close;
    s->nq = s->qh = 0;
    s->held_len = s->held_pos = 0;
    s->abort_asked = 0;                     /* a stop the PC asked for ends that PUT only, not the next one on the link */
    e = ng_put(sk, tag, &man, mftext, n - 8, &src, mode);
    drain_wants(s);                         /* no DATA may follow the DONE */
    ng_cfg.force = 0;
    ng_cfg.allow_frag = 0;
    return e;
}

static int serve_request(ngtp_srv_t *s, const ngtp_hdr_t *h, const uint8_t *p, uint32_t n)
{
    ng_sink_t sk = sink_of(s);
    char folder[NG_PATH_BUF];
    ng_err_t e = NG_OK;
    int finish = 0;

    s->seq = h->seq;
    if (h->type == NGTP_HELLO) { do_hello(s, h->seq, p, n); return 0; }
    if (!s->hello) { send_error(s, h->seq, "ESTATE", "HELLO first"); return 0; }
    if (h->type == NGTP_DATA) { send_error(s, h->seq, "ESTATE", "DATA without a WANT"); return 0; }
    if (h->type == NGTP_PING) { send_frame(s, NGTP_PONG, 0, h->seq, NULL, 0); return 0; }

    if (h->type == NGTP_FINISH) {
        int mode = n >= 1 ? p[0] : 1;
        if (n < 1 || mode > 2 || folder_arg(p, n, 1, folder, sizeof folder)) { send_error(s, h->seq, "EPROTO", "bad FINISH"); return 0; }
        if (s->hk.busy) s->hk.busy(s->hk.ctx, 1);
        e = ng_session_close(&sk, mode, folder, 0);
        s->session = 0;
        if (s->hk.busy) s->hk.busy(s->hk.ctx, 0);
        send_done(s, h->seq, e);
        ngtp_srv_drop(s);                  /* answer first, then leave the transfer screen */
        return 1;
    }

    /* the operations that need the write session */
    if (h->type == NGTP_LIST || h->type == NGTP_VERIFY) {
        if (folder_arg(p, n, 0, folder, sizeof folder)) { send_error(s, h->seq, "EPROTO", "bad folder"); return 0; }
    } else if (h->type == NGTP_DELETE) {
        if (n < 1 || folder_arg(p, n, 1, folder, sizeof folder)) { send_error(s, h->seq, "EPROTO", "bad DELETE"); return 0; }
    } else if (h->type == NGTP_PUT) {
        if (n < 8) { send_error(s, h->seq, "EPROTO", "short PUT"); return 0; }
    } else if (h->type != NGTP_STATFS) {
        send_error(s, h->seq, "EPROTO", "unknown request");
        return 0;
    }
    if (s->hk.busy) s->hk.busy(s->hk.ctx, 1);
    e = ensure_session(s, &sk);
    if (e == NG_OK) {
        switch (h->type) {
        case NGTP_STATFS: e = ng_statfs(&sk); break;
        case NGTP_LIST: e = ng_list(&sk, folder); break;
        case NGTP_VERIFY: e = ng_verify(&sk, folder); break;
        case NGTP_DELETE:
            ng_cfg.force = 0;
            if ((p[0] & 1) && ngfs_protected(folder))
                ng_cfg.force = s->hk.confirm ? s->hk.confirm(s->hk.ctx, "delete", folder) : 0;
            e = ng_delete(&sk, folder);
            ng_cfg.force = 0;
            break;
        default: e = do_put(s, &sk, p, n); break;
        }
    }
    if (s->hk.busy) s->hk.busy(s->hk.ctx, 0);
    send_done(s, h->seq, e);
    return finish;
}

void ngtp_srv_init(ngtp_srv_t *s, const ngtp_hooks_t *hk, const ng_fwinfo_t *fw, uint16_t code)
{
    memset(s, 0, sizeof *s);
    s->hk = *hk;
    s->fw = fw;
    s->code = code;
    s->last_frame_ms = now_ms(s);
    s->want_max = NGTP_MAX_DATA;
    s->depth = 1;
    ng_cfg_defaults();
}

void ngtp_srv_set_window(ngtp_srv_t *s, uint32_t want_max, int depth)
{
    s->want_max = want_max < 1 ? 1 : want_max > NGTP_MAX_DATA ? NGTP_MAX_DATA : want_max;
    s->depth = depth < 1 ? 1 : depth > NGTP_DEPTH_MAX ? NGTP_DEPTH_MAX : depth;
}

void ngtp_srv_new_code(ngtp_srv_t *s, uint16_t code)
{
    s->code = code;
    s->auth_fail = 0;
    s->code_expired = 0;
}

int ngtp_srv_service(ngtp_srv_t *s)
{
    ngtp_hdr_t h;
    int ev = 0, r;
    if (!s->connected) {
        if (nm_net_accept(0) != 1) return 0;
        s->connected = 1;
        s->hello = 0;
        s->broken = 0;
        s->abort_asked = 0;
        s->client[0] = 0;
        s->last_frame_ms = now_ms(s);
    }
    r = nm_net_recv_all(rx, NGTP_HDR_LEN, 0);
    if (r == NM_NET_AGAIN) {
        /* Idle, and nothing from the client.  A new connection means the old one is gone or dead (a PC that lost power or
           a cable that was pulled sends no FIN): the network layer closes the old socket and hands over the new one, and the
           new client starts again with HELLO (R104 A).  The write session stays as it is: the same PC resumes in it. */
        if (nm_net_accept(0) == 1) {
            s->hello = 0;
            s->broken = 0;
            s->abort_asked = 0;
            s->client[0] = 0;
            s->last_frame_ms = now_ms(s);
        }
        return 0;
    }
    if (r != 0) { ngtp_srv_drop(s); return NGTP_EV_CLOSED; }
    if (ngtp_hdr_parse(rx, &h) != NGTP_OK) { send_error(s, 0, "EPROTO", "bad frame header"); return NGTP_EV_CLOSED; }
    if (nm_net_recv_all(rx + NGTP_HDR_LEN, h.len + 4, FRAME_TIMEOUT_MS) != 0) { ngtp_srv_drop(s); return NGTP_EV_CLOSED; }
    if (ngtp_frame_check(rx, rx + NGTP_HDR_LEN, h.len, rx + NGTP_HDR_LEN + h.len) != NGTP_OK) {
        send_error(s, h.seq, "EBADCRC", "frame CRC mismatch");
        return NGTP_EV_CLOSED;
    }
    if (!ngtp_is_request(h.type)) { send_error(s, h.seq, "EPROTO", "unknown frame type"); return NGTP_EV_CLOSED; }
    s->last_frame_ms = now_ms(s);
    if (serve_request(s, &h, rx + NGTP_HDR_LEN, h.len)) ev |= NGTP_EV_FINISH;
    s->last_frame_ms = now_ms(s);
    ev |= NGTP_EV_REQUEST;
    if (s->connected && s->broken) ngtp_srv_drop(s);        /* the link failed during the operation */
    if (!s->connected) ev |= NGTP_EV_CLOSED;
    return ev;
}

ng_err_t ngtp_srv_end(ngtp_srv_t *s, int mode)
{
    ng_sink_t sk = sink_of(s);
    ng_err_t e;
    if (!s->session) return NG_OK;
    e = ng_session_close(&sk, mode, NULL, 1);
    s->session = 0;
    return e;
}
