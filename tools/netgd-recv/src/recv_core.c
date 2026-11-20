/* Receive core.  Design 5.2 (layout, checkpoints, atomic finalize), 5.5 (safety), 8 (failures). */
#include <stdio.h>
#include <stdarg.h>
#include <string.h>
#include "recv_core.h"
#include "ngfs.h"
#include "ngdev.h"
#include "crc32.h"

ng_cfg_t ng_cfg;

void ng_cfg_defaults(void)
{
    ng_cfg.ckp_bytes = 8u * 1024 * 1024;
    ng_cfg.read_bytes = 64u * 1024;
    ng_cfg.readback = 1;                 /* design 4.5/11.5: on until the G1 bus is trusted */
    ng_cfg.allow_frag = 0;
    ng_cfg.force = 0;
    ng_cfg.force_slow = 0;
    ng_cfg.headroom = 16u * 1024 * 1024;
    ng_cfg.mount_line_ms = 1000;
}

const char *ng_err_name(ng_err_t e)
{
    static const char *const n[] = { "OK", "EVERSION", "EAUTH", "EBUSY", "EBADCRC", "EMANIFEST", "EEXIST", "ENOENT",
                                     "ENOSPC", "EFRAG", "ECRC", "EIO", "EBUS", "EPROTECTED", "EFW", "ESTATE", "EFS",
                                     "ESTOP", "EJOB", "ESRC" };
    /* order of the enum: ... NG_E_STATE, NG_E_FS, NG_E_STOP, NG_E_JOB, NG_E_SRC */
    return (unsigned)e < sizeof n / sizeof n[0] ? n[e] : "EUNKNOWN";
}

static uint8_t iobuf[65536] __attribute__((aligned(32)));
static ng_manifest_t vm;                 /* manifest of verify / list work */
static ng_fwinfo_t g_fw;
static int g_session;

const ng_fwinfo_t *ng_session_fw(void) { return &g_fw; }

void ng_emitf(const ng_sink_t *sk, int throttled, const char *fmt, ...)
{
    char line[400];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(line, sizeof line, fmt, ap);
    va_end(ap);
    sk->line(sk->ctx, line, throttled);
}

void ng_emit_error(const ng_sink_t *sk, const char *tag, ng_err_t e, int file, uint64_t off, const char *msg)
{
    char pos[64] = "", m[200] = "";
    if (file > 0) snprintf(pos, sizeof pos, " file=%d offset=%llu", file, (unsigned long long)off);
    if (msg && *msg) {
        size_t j = 0;
        for (const char *p = msg; *p && j < sizeof m - 8; p++) m[j++] = (*p == '"' || *p < 0x20) ? '\'' : *p;
        m[j] = 0;
        ng_emitf(sk, 0, "NGD:ERROR %s code=%s%s msg=\"%s\"", tag, ng_err_name(e), pos, m);
    } else {
        ng_emitf(sk, 0, "NGD:ERROR %s code=%s%s", tag, ng_err_name(e), pos);
    }
}

static uint32_t now(const ng_sink_t *sk) { return sk->millis ? sk->millis(sk->ctx) : 0; }

/* ------------------------------------------------------------------ session */

/* Mounting reads the whole FAT (ngfs_mount): seconds on a large card.  One "NGD:MOUNT pct=NN" line
   a second while it runs (droppable chatter), and always one closing line with the size and the time. */
typedef struct {
    const ng_sink_t *sk;
    uint32_t t0, tlast, total;
} mount_prog_t;

static void mount_tick(void *ctx, uint32_t done, uint32_t total)
{
    mount_prog_t *p = (mount_prog_t *)ctx;
    uint32_t t = now(p->sk);
    p->total = total;
    if (done >= total || t - p->tlast < ng_cfg.mount_line_ms) return;
    p->tlast = t;
    ng_emitf(p->sk, 1, "NGD:MOUNT pct=%u", (unsigned)((uint64_t)done * 100 / total));
    if (p->sk->ui) {
        ng_ui_t u = { "mount", "reading the FAT (free space)", 0, 0, done, total, 0 };
        p->sk->ui(p->sk->ctx, &u);
    }
}

ng_err_t ng_session_open(const ng_sink_t *sk)
{
    char why[96] = "";
    uint64_t fr = 0, tot = 0;
    mount_prog_t mp;
    ng_err_t e;

    if (fw_version(&g_fw) != 0) {
        ng_emit_error(sk, "session", NG_E_FW, 0, 0, "no NetGD firmware answered (this is not a NetGD)");
        return NG_E_FW;
    }
    /* Writing needs the protected area and the write-check value of the firmware: the KOS g1ata PIO
       write does not report errors, so the FNV-1a in STATUS is the only proof that a sector arrived. */
    if ((g_fw.caps & (FW_CAP_SESSION | FW_CAP_CHECKSUM)) != (FW_CAP_SESSION | FW_CAP_CHECKSUM)) {
        ng_emit_error(sk, "session", NG_E_FW, 0, 0,
                      "firmware lacks FW_CAP_SESSION and FW_CAP_CHECKSUM (VERSION caps bit0, bit1): will not write");
        return NG_E_FW;
    }
    if (ngdev_init() != 0) {
        ng_emit_error(sk, "session", NG_E_IO, 0, 0, "no secondary IDE disk");
        return NG_E_IO;
    }
    ngblk_check_fnv = 1;
    ngdev_use_dma = (g_fw.caps & FW_CAP_DMA) != 0;
    if (fw_session_begin(&g_fw) != 0) {
        ng_emit_error(sk, "session", NG_E_FW, 0, 0, "write session refused");
        return NG_E_FW;
    }
    g_session = 1;
    mp.sk = sk; mp.total = 0; mp.t0 = mp.tlast = now(sk);
    e = ngfs_mount(why, sizeof why, mount_tick, &mp);
    if (e) {
        ng_emit_error(sk, "session", e, 0, 0, why);
        ng_session_close(sk, 1, NULL, 1);           /* the session is open: always end it (default image back) */
        return e;
    }
    ng_emitf(sk, 0, "NGD:MOUNT pct=100 fat_sectors=%u ms=%u", (unsigned)mp.total, (unsigned)(now(sk) - mp.t0));
    ngfs_statfs(&fr, &tot, NULL);
    ng_emitf(sk, 0, "NGD:HELLO app=%s fw=OGDM/%d caps=%d free=%llu total=%llu", NG_APP_VERSION, g_fw.proto, g_fw.caps,
             (unsigned long long)fr, (unsigned long long)tot);
    return NG_OK;
}

ng_err_t ng_session_close(const ng_sink_t *sk, int mode, const char *folder, int quiet)
{
    char dir[NG_PATH_BUF];
    int fwmode = mode, rc;
    const char *mounted = NULL;
    if (!g_session) return NG_OK;
    if (mode == 2) {
        char why[96];
        if (!folder || ng_path_ok(folder, why, sizeof why) != 0) {      /* also: at most NG_PATH_MAX = 96, the firmware's MOUNT_NAME capacity */
            fwmode = 1;
        } else {
            ngfs_dirpath(folder, dir, sizeof dir);
            if (!strcmp(ngfs_classify(dir), "complete")) mounted = folder;
            else fwmode = 1;
        }
    }
    ngfs_unmount();
    rc = fw_session_end(&g_fw, fwmode, mounted);
    g_session = 0;
    if (!quiet) {
        if (rc != 0) ng_emitf(sk, 0, "NGD:FINISH fail code=EFW");
        else if (mode == 2 && !mounted) ng_emitf(sk, 0, "NGD:FINISH fail code=ENOENT msg=\"folder name invalid or not a complete image\"");
        else if (mounted) ng_emitf(sk, 0, "NGD:FINISH ok mounted=\"%s\"", mounted);
        else ng_emitf(sk, 0, "NGD:FINISH ok");
    }
    return rc == 0 ? NG_OK : NG_E_FW;
}

/* ------------------------------------------------------------------ put */

typedef struct {
    const ng_sink_t *sk;
    const char *tag, *name;
    int fi, fn;
    uint64_t total, base_cum, session_bytes;
    uint32_t t0, tlast;
} prog_t;

static void progress(prog_t *p, uint64_t cum, int force)
{
    uint32_t t = now(p->sk);
    uint32_t el = t - p->t0;
    uint32_t rate = el ? (uint32_t)((cum - p->base_cum) * 1000u / el) : 0;
    if (!force && t - p->tlast < 1000) return;
    p->tlast = t;
    ng_emitf(p->sk, !force, "NGD:PUT %s file=%d/%d bytes=%llu/%llu rate=%u", p->tag, p->fi, p->fn,
             (unsigned long long)cum, (unsigned long long)p->total, (unsigned)rate);
    if (p->sk->ui) {
        ng_ui_t u = { "put", p->name, p->fi, p->fn, cum, p->total, rate };
        p->sk->ui(p->sk->ctx, &u);
    }
}

static void file_path(char *out, size_t n, const char *dir, const ng_manifest_t *m, int i, const char *ngp, int use_ngp)
{
    snprintf(out, n, "%s/%s", dir, (i == m->descriptor && use_ngp) ? ngp : m->files[i].name);
}

typedef struct {
    const ng_sink_t *sk;
    const char *tag, *name;
    int fi, fn;
    uint64_t base, total;
    uint32_t tlast;
} rb_t;

/* called by ngfs_file_crc after every 64 KiB: cancel check, and a progress line (at most 1 per
   second) so that the PC side keeps seeing activity on /pc/ during a long read-back */
static int rb_tick(void *ctx, uint32_t done)
{
    rb_t *r = (rb_t *)ctx;
    uint32_t t = now(r->sk);
    if (r->sk->poll && r->sk->poll(r->sk->ctx)) return 1;
    if (t - r->tlast >= 1000) {
        r->tlast = t;
        ng_emitf(r->sk, 1, "NGD:READBACK %s file=%d/%d bytes=%llu/%llu", r->tag, r->fi, r->fn,
                 (unsigned long long)(r->base + done), (unsigned long long)r->total);
        if (r->sk->ui) {
            ng_ui_t u = { "verify", r->name, r->fi, r->fn, r->base + done, r->total, 0 };
            r->sk->ui(r->sk->ctx, &u);
        }
    }
    return 0;
}

/* read every file back and compare CRC32; returns 0 or an error with *bad = file index.
   NG_E_STOP: reload.req / cancel.req / B was seen (nothing is changed on the card by a read-back). */
static ng_err_t readback_all(const ng_sink_t *sk, const char *dir, const ng_manifest_t *m, const char *ngp,
                             const char *fin, int *bad, const char *tag, int emit_file)
{
    rb_t rb;
    rb.sk = sk; rb.tag = tag; rb.name = ""; rb.fi = 1; rb.fn = m->nfiles; rb.base = 0; rb.total = m->total_bytes;
    rb.tlast = now(sk);
    for (int i = 0; i < m->nfiles; i++) {
        char path[NG_PATH_BUF];
        uint32_t size = 0, crc = 0;
        ng_err_t e;
        if (i == m->descriptor) {
            uint32_t d;
            snprintf(path, sizeof path, "%s/%s", dir, ngp);
            if (ngfs_stat_size(path, &d) != 0) snprintf(path, sizeof path, "%s/%s", dir, fin);
        } else {
            snprintf(path, sizeof path, "%s/%s", dir, m->files[i].name);
        }
        rb.fi = i + 1; rb.name = m->files[i].name;
        e = ngfs_file_crc(path, &size, &crc, iobuf, sizeof iobuf, rb_tick, &rb);
        *bad = i;
        if (e) return e;
        if (size != m->files[i].size || crc != m->files[i].crc) return NG_E_CRC;
        if (emit_file) ng_emitf(sk, 0, "NGD:FILE %s file=%d size=%u crc=%08X", tag, i + 1, (unsigned)size, (unsigned)crc);
        rb.base += m->files[i].size;
    }
    return NG_OK;
}

ng_err_t ng_put(const ng_sink_t *sk, const char *tag, const ng_manifest_t *m, const char *mtext, size_t mlen,
                const ng_src_t *src, int mode)
{
    enum { A_NEW, A_INPLACE, A_CONTINUE, A_COMPLETE } action;
    char dir[NG_PATH_BUF], path[NG_PATH_BUF], ngp[16], fin[16];
    ng_probe_t pr;
    ng_ckp_t ck;
    ng_err_t e;
    uint64_t free_b = 0, tot_b = 0;
    uint32_t cb = 0;
    int destroy = 0, n = m->nfiles, bad = 0, contig;
    prog_t pg;

    ng_manifest_ngp_name(m, ngp, fin);
    ngfs_dirpath(m->image, dir, sizeof dir);
    memset(&ck, 0, sizeof ck);
    ck.mf_crc = m->crc;

    /* 1. what is there? */
    e = ngfs_probe(dir, m, &pr);
    if (e) { ng_emit_error(sk, tag, e, 0, 0, "cannot read the target folder"); return e; }
    if (!pr.exists) {
        action = A_NEW;
    } else if (mode == NG_PUT_NEW) {
        ng_emit_error(sk, tag, NG_E_EXIST, 0, 0, "folder exists (use mode resume or overwrite)");
        return NG_E_EXIST;
    } else if (mode == NG_PUT_OVERWRITE) {
        if (!ngfs_is_image_dir(dir)) {
            ng_emit_error(sk, tag, NG_E_PROTECTED, 0, 0, "folder is not an image folder; will not erase it");
            return NG_E_PROTECTED;
        }
        if (ngfs_protected(m->image) && !ng_cfg.force) {
            ng_emit_error(sk, tag, NG_E_PROTECTED, 0, 0, "protected folder (GDEMU.CFG default or 01); create force.req");
            return NG_E_PROTECTED;
        }
        if (pr.has_subdir) { ng_emit_error(sk, tag, NG_E_EXIST, 0, 0, "folder has sub-folders"); return NG_E_EXIST; }
        action = A_INPLACE;
        destroy = 1;
    } else {    /* resume */
        if (!pr.has_mf) {
            if (pr.nfiles == 0 && !pr.has_subdir) action = A_INPLACE;
            else { ng_emit_error(sk, tag, NG_E_EXIST, 0, 0, "folder holds something that is not a netgd-recv image"); return NG_E_EXIST; }
        } else if (!pr.mf_matches && !pr.mf_valid && pr.ckp == 0 && !pr.has_image_ext) {
            action = A_INPLACE;                 /* NETGD.MF is a torn write of ours (NETGD.CKP is only made after it) */
        } else if (!pr.mf_matches) {
            ng_emit_error(sk, tag, NG_E_EXIST, 0, 0, "a different image (other manifest) is in this folder");
            return NG_E_EXIST;
        } else if (pr.has_subdir) {
            ng_emit_error(sk, tag, NG_E_EXIST, 0, 0, "folder has sub-folders");
            return NG_E_EXIST;
        } else if (pr.ckp == 0) {
            action = pr.has_final ? A_COMPLETE : A_INPLACE;
        } else if (pr.ckp == CKP_TRANSFER || pr.ckp == CKP_COMPLETE) {
            int ok = pr.ck.files_done <= n && (pr.ck.offset % 512) == 0 &&
                     (pr.ck.files_done == n ? pr.ck.offset == 0 : pr.ck.offset < m->files[pr.ck.files_done].size);
            action = A_CONTINUE;
            for (int i = 0; ok && i < n; i++) {
                uint32_t sz;
                int have = 0;
                if (i == m->descriptor) {
                    snprintf(path, sizeof path, "%s/%s", dir, ngp);
                    have = ngfs_stat_size(path, &sz) == 0;
                    if (!have && pr.ckp == CKP_COMPLETE) {
                        snprintf(path, sizeof path, "%s/%s", dir, fin);
                        have = ngfs_stat_size(path, &sz) == 0;
                    }
                } else {
                    snprintf(path, sizeof path, "%s/%s", dir, m->files[i].name);
                    have = ngfs_stat_size(path, &sz) == 0;
                }
                if (!have || sz != m->files[i].size) ok = 0;
            }
            if (!ok) action = A_INPLACE;       /* the checkpoint does not match what is on the card: start over */
            else ck = pr.ck;
        } else {
            action = A_INPLACE;                 /* allocation interrupted, or an unreadable checkpoint */
        }
    }

    ngfs_statfs(&free_b, &tot_b, &cb);
    pg.sk = sk; pg.tag = tag; pg.name = ""; pg.fi = 1; pg.fn = n; pg.total = m->total_bytes;
    pg.base_cum = 0; pg.t0 = pg.tlast = now(sk);

    if (action == A_COMPLETE) {
        e = readback_all(sk, dir, m, ngp, fin, &bad, tag, 1);
        if (e == NG_E_STOP) { ng_emitf(sk, 0, "NGD:STOP %s", tag); return e; }
        if (e) { ng_emit_error(sk, tag, e, bad + 1, 0, "image already on the card does not verify"); return e; }
        ng_emitf(sk, 0, "NGD:DONE %s ok", tag);
        return NG_OK;
    }

    if (action == A_NEW || action == A_INPLACE) {
        uint64_t need = (uint64_t)8 * cb, reclaim = 0;
        for (int i = 0; i < n; i++) need += ((uint64_t)m->files[i].size + cb - 1) / cb * cb;
        if (action == A_INPLACE) reclaim = ngfs_dir_bytes(dir);
        if (free_b + reclaim < need + ng_cfg.headroom) {
            ng_emitf(sk, 0, "NGD:ERROR %s code=ENOSPC need=%llu free=%llu", tag, (unsigned long long)need,
                     (unsigned long long)free_b);
            return NG_E_NOSPC;
        }
        if (action == A_INPLACE) {
            if (!destroy && pr.has_final && ngfs_protected(m->image) && !ng_cfg.force) {
                ng_emit_error(sk, tag, NG_E_PROTECTED, 0, 0, "protected folder");
                return NG_E_PROTECTED;
            }
            e = ngfs_delete_contents(dir);
            if (e) { ng_emit_error(sk, tag, e, 0, 0, "cannot clear the folder"); return e; }
        } else {
            int r = ngfs_mkdir_p(dir);
            if (r != FR_OK) { e = ngfs_map((FRESULT)r); ng_emit_error(sk, tag, e, 0, 0, "mkdir failed"); return e; }
        }
        ng_phase_hook ? ng_phase_hook("dir_ready") : (void)0;
        e = ngfs_write_mf(dir, mtext, mlen);
        if (e) goto fail_new;
        ck.state = CKP_ALLOCATING;
        e = ngfs_ckp_create(dir, &ck);
        if (e) goto fail_new;
        if (ng_phase_hook) ng_phase_hook("ckp_created");
        for (int i = 0; i < n; i++) {
            int frag = 0;
            file_path(path, sizeof path, dir, m, i, ngp, 1);
            e = ngfs_alloc_file(path, m->files[i].size, ng_cfg.allow_frag, &frag);
            if (e) {
                ng_emit_error(sk, tag, e, i + 1, 0, e == NG_E_FRAG ? "no contiguous free area for this file" : "allocation failed");
                goto fail_new;
            }
            if (frag) ck.flags |= CKP_FL_FRAG;
        }
        ck.state = CKP_TRANSFER;
        e = ngfs_ckp_write(&ck);
        if (e) goto fail_new;
        if (ng_phase_hook) ng_phase_hook("alloc_done");
    } else {
        e = ngfs_ckp_open(dir);
        if (e) { ng_emit_error(sk, tag, e, 0, 0, "checkpoint file"); return e; }
        if (ck.files_done < n)
            ng_emitf(sk, 0, "NGD:RESUME %s file=%d offset=%llu crc=%08X", tag, ck.files_done + 1,
                     (unsigned long long)ck.offset, (unsigned)ck.crc);
    }
    contig = !(ck.flags & CKP_FL_FRAG) && !ng_cfg.force_slow;

    /* 2. transfer */
    {
        uint64_t done_before = 0;
        for (int i = 0; i < ck.files_done && i < n; i++) done_before += m->files[i].size;
        pg.base_cum = done_before + (ck.files_done < n ? ck.offset : 0);
        pg.t0 = pg.tlast = now(sk);
    }
    for (int i = ck.files_done; i < n; i++) {
        const ng_mfile_t *f = &m->files[i];
        uint64_t pos = (i == ck.files_done) ? ck.offset : 0;
        uint32_t crc = (i == ck.files_done) ? ck.crc : 0;
        uint64_t cum_before = 0;
        ng_wfile_t w;
        for (int k = 0; k < i; k++) cum_before += m->files[k].size;
        pg.fi = i + 1; pg.name = f->name;
        file_path(path, sizeof path, dir, m, i, ngp, 1);
        e = ngfs_wfile_open(&w, path, f->size, contig);
        if (e) { ng_emit_error(sk, tag, e, i + 1, pos, "cannot open the pre-allocated file"); return e; }
        if (w.note)
            ng_emitf(sk, 0, "NGD:NOTE %s file=%d slow-path reason=\"%s\"", tag, i + 1, w.note);
        if (f->size && src->open(src->ctx, i, pos) != 0) {
            ngfs_wfile_close(&w);
            ng_emit_error(sk, tag, NG_E_SRC, i + 1, pos, "cannot open the source under /pc/");
            return NG_E_SRC;
        }
        while (pos < f->size) {
            uint64_t bound = (pos / ng_cfg.ckp_bytes + 1) * ng_cfg.ckp_bytes;
            uint32_t want = ng_cfg.read_bytes, got = 0;
            if (want > sizeof iobuf) want = sizeof iobuf;
            if (want > f->size - pos) want = (uint32_t)(f->size - pos);
            if (want > bound - pos) want = (uint32_t)(bound - pos);
            while (got < want) {
                int32_t r = src->read(src->ctx, iobuf + got, want - got);
                if (r <= 0) break;
                got += (uint32_t)r;
            }
            if (got != want) {
                src->close(src->ctx);
                ngfs_wfile_close(&w);
                ng_emit_error(sk, tag, NG_E_SRC, i + 1, pos, "short read from the source");
                return NG_E_SRC;
            }
            crc = ng_crc32(crc, iobuf, got);
            e = ngfs_wfile_write(&w, (uint32_t)pos, iobuf, got);
            if (e) {
                src->close(src->ctx);
                ngfs_wfile_close(&w);
                ng_emit_error(sk, tag, e, i + 1, pos & ~(uint64_t)(ng_cfg.ckp_bytes - 1), "write to the card failed");
                return e;
            }
            pos += got;
            if (pos < f->size && pos % ng_cfg.ckp_bytes == 0) {
                ck.state = CKP_TRANSFER; ck.files_done = (uint16_t)i; ck.offset = pos; ck.crc = crc;
                e = ngfs_ckp_write(&ck);
                if (e) { src->close(src->ctx); ngfs_wfile_close(&w); ng_emit_error(sk, tag, e, i + 1, pos, "checkpoint write failed"); return e; }
                if (ng_phase_hook) ng_phase_hook("checkpoint");
            }
            progress(&pg, cum_before + pos, 0);
            if (sk->poll && sk->poll(sk->ctx)) {
                src->close(src->ctx);
                ngfs_wfile_close(&w);
                ng_emitf(sk, 0, "NGD:STOP %s", tag);
                return NG_E_STOP;
            }
        }
        if (f->size) src->close(src->ctx);
        e = ngfs_wfile_close(&w);
        if (e) { ng_emit_error(sk, tag, e, i + 1, 0, "close failed"); return e; }
        if (crc != f->crc) {
            ck.state = CKP_TRANSFER; ck.files_done = (uint16_t)i; ck.offset = 0; ck.crc = 0;
            ngfs_ckp_write(&ck);
            ng_emit_error(sk, tag, NG_E_CRC, i + 1, 0, "CRC32 of the received file does not match the manifest");
            return NG_E_CRC;
        }
        ng_emitf(sk, 0, "NGD:FILE %s file=%d size=%u crc=%08X", tag, i + 1, (unsigned)f->size, (unsigned)crc);
        ck.state = CKP_TRANSFER; ck.files_done = (uint16_t)(i + 1); ck.offset = 0; ck.crc = 0;
        e = ngfs_ckp_write(&ck);
        if (e) { ng_emit_error(sk, tag, e, i + 1, 0, "checkpoint write failed"); return e; }
        if (ng_phase_hook) ng_phase_hook("file_done");
        progress(&pg, cum_before + f->size, 1);
    }

    /* 3. verify (readback is required when the firmware cannot check the bus: 4.5) */
    if (ng_cfg.readback || !ngblk_check_fnv) {
        if (ng_phase_hook) ng_phase_hook("readback");
        e = readback_all(sk, dir, m, ngp, fin, &bad, tag, 0);
        if (e == NG_E_STOP) {           /* the checkpoint says "all files written": a resume goes straight to the read-back */
            ng_emitf(sk, 0, "NGD:STOP %s", tag);
            return e;
        }
        if (e) {
            if (e == NG_E_CRC) {
                ck.state = CKP_TRANSFER; ck.files_done = (uint16_t)bad; ck.offset = 0; ck.crc = 0;
                ngfs_ckp_write(&ck);
            }
            ng_emit_error(sk, tag, e, bad + 1, 0, "read-back verification failed");
            return e;
        }
        if (ng_phase_hook) ng_phase_hook("verified");
    }

    /* 4. commit: CKP "complete", one 3-byte directory-entry change, then drop the CKP */
    ck.state = CKP_COMPLETE; ck.files_done = (uint16_t)n; ck.offset = 0; ck.crc = 0;
    e = ngfs_ckp_write(&ck);
    if (e) { ng_emit_error(sk, tag, e, 0, 0, "checkpoint write failed"); return e; }
    snprintf(path, sizeof path, "%s/%s", dir, ngp);
    {
        uint32_t sz;
        if (ngfs_stat_size(path, &sz) == 0) {
            e = ngfs_commit_ext(path, "NGP", m->is_cdi ? "CDI" : "GDI");
            if (e) { ng_emit_error(sk, tag, e, 0, 0, "finalize failed"); return e; }
        }
    }
    if (ng_phase_hook) ng_phase_hook("committed");
    e = ngfs_ckp_delete(dir);
    if (e) { ng_emit_error(sk, tag, e, 0, 0, "cannot remove NETGD.CKP"); return e; }
    ng_emitf(sk, 0, "NGD:DONE %s ok", tag);
    return NG_OK;

fail_new:
    if (e != NG_E_FRAG) ng_emit_error(sk, tag, e, 0, 0, "could not set up the image folder");
    if (action == A_NEW) ngfs_delete_image(dir, 1);       /* leave nothing half-made behind a folder we created */
    return e;
}

/* ------------------------------------------------------------------ other operations */

/* every folder argument of the job goes through the validation of a put's image path */
static ng_err_t check_folder_arg(const ng_sink_t *sk, const char *tag, const char *folder, int allow_root)
{
    char why[96];
    if (allow_root && (!folder[0] || !strcmp(folder, "/"))) return NG_OK;
    if (ng_path_ok(folder, why, sizeof why) != 0) {
        ng_emit_error(sk, tag, NG_E_MANIFEST, 0, 0, why);
        return NG_E_MANIFEST;
    }
    return NG_OK;
}

ng_err_t ng_verify(const ng_sink_t *sk, const char *folder)
{
    char dir[NG_PATH_BUF], path[NG_PATH_BUF], ngp[16], fin[16];
    static uint8_t mf[32768];
    FIL fp;
    UINT got = 0;
    char err[96];
    int bad = 0;
    ng_err_t e;
    e = check_folder_arg(sk, "verify", folder, 0);
    if (e) return e;
    ngfs_dirpath(folder, dir, sizeof dir);
    snprintf(path, sizeof path, "%s/NETGD.MF", dir);
    if (f_open(&fp, path, FA_READ) != FR_OK) {
        ng_emit_error(sk, "verify", NG_E_NOENT, 0, 0, "no NETGD.MF in that folder");
        return NG_E_NOENT;
    }
    if (f_size(&fp) > sizeof mf || f_read(&fp, mf, (UINT)f_size(&fp), &got) != FR_OK) { f_close(&fp); return NG_E_IO; }
    f_close(&fp);
    if (ng_manifest_parse((const char *)mf, got, &vm, err, sizeof err)) {
        ng_emit_error(sk, "verify", NG_E_MANIFEST, 0, 0, err);
        return NG_E_MANIFEST;
    }
    ng_manifest_ngp_name(&vm, ngp, fin);
    e = readback_all(sk, dir, &vm, ngp, fin, &bad, "verify", 0);
    if (e == NG_E_STOP) { ng_emitf(sk, 0, "NGD:STOP verify"); return e; }
    if (e) {
        ng_emitf(sk, 0, "NGD:VERIFY \"%s\" fail code=%s file=%d", folder, ng_err_name(e), bad + 1);
        return e;
    }
    ng_emitf(sk, 0, "NGD:VERIFY \"%s\" ok", folder);
    return NG_OK;
}

ng_err_t ng_delete(const ng_sink_t *sk, const char *folder)
{
    char dir[NG_PATH_BUF];
    ng_err_t e;
    if (!folder[0] || !strcmp(folder, "/")) {
        ng_emit_error(sk, "delete", NG_E_PROTECTED, 0, 0, "the root cannot be deleted");
        return NG_E_PROTECTED;
    }
    e = check_folder_arg(sk, "delete", folder, 0);
    if (e) return e;
    ngfs_dirpath(folder, dir, sizeof dir);
    if (!ngfs_is_image_dir(dir)) {
        ng_emit_error(sk, "delete", NG_E_PROTECTED, 0, 0, "not an image folder (no NETGD.MF/.GDI/.CDI/.NGP)");
        return NG_E_PROTECTED;
    }
    if (ngfs_protected(folder) && !ng_cfg.force) {
        ng_emit_error(sk, "delete", NG_E_PROTECTED, 0, 0, "protected folder; create force.req");
        return NG_E_PROTECTED;
    }
    e = ngfs_delete_image(dir, 1);
    if (e) { ng_emit_error(sk, "delete", e, 0, 0, "delete failed"); return e; }
    ng_emitf(sk, 0, "NGD:DELETE \"%s\" ok", folder);
    return NG_OK;
}

static void list_cb(void *ctx, const char *state, const char *path)
{
    ng_emitf((const ng_sink_t *)ctx, 0, "NGD:LIST %s %s", state, path);
}

ng_err_t ng_list(const ng_sink_t *sk, const char *folder)
{
    ng_err_t e = check_folder_arg(sk, "list", folder, 1);
    if (e) return e;
    e = ngfs_list(folder, list_cb, (void *)sk);
    if (e) ng_emit_error(sk, "list", e, 0, 0, NULL);
    return e;
}

ng_err_t ng_statfs(const ng_sink_t *sk)
{
    uint64_t fr = 0, tot = 0;
    uint32_t cb = 0;
    if (ngfs_statfs(&fr, &tot, &cb)) { ng_emit_error(sk, "statfs", NG_E_IO, 0, 0, NULL); return NG_E_IO; }
    ng_emitf(sk, 0, "NGD:STATFS free=%llu total=%llu cluster=%u", (unsigned long long)fr, (unsigned long long)tot,
             (unsigned)cb);
    return NG_OK;
}
