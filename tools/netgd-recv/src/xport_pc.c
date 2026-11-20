#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <dirent.h>
#include "xport_pc.h"
#include "ng_plat.h"
#include "ngdev.h"

#define MAX_OPS 64

uint32_t ng_pc_poll_ms = 1000;          /* tests set 0 */
void (*ng_pc_cfg_hook)(void);          /* tests: change ng_cfg after the defaults and the *.req flags */

typedef struct {
    char jobdir[160];
    FILE *res;
    uint32_t last_poll;
    int stop;
} pc_t;

static int pc_exists(const char *path)
{
    char buf[256];
    FILE *f = fopen(ng_plat_pcpath(path, buf, sizeof buf), "rb");
    if (!f) return 0;
    fclose(f);
    return 1;
}

/* read a whole small file; returns length or -1.  Caller frees. */
static long pc_slurp(const char *path, char **out, long cap)
{
    char buf[256];
    FILE *f = fopen(ng_plat_pcpath(path, buf, sizeof buf), "rb");
    char *p;
    long got;
    if (!f) return -1;
    p = (char *)malloc((size_t)cap + 1);
    if (!p) { fclose(f); return -1; }
    got = (long)fread(p, 1, (size_t)cap, f);
    fclose(f);
    p[got] = 0;
    *out = p;
    return got;
}

/* ------------------------------------------------------------------ sink */

static void sink_line(void *ctx, const char *line, int throttled)
{
    pc_t *pc = (pc_t *)ctx;
    (void)throttled;
    ng_plat_console(line);
    if (pc->res) {
        /* one line per fwrite, always far below 4 KiB, each followed by fflush (KNOWHOW 4) */
        char tmp[512];
        size_t n = (size_t)snprintf(tmp, sizeof tmp, "%s\n", line);
        if (n >= sizeof tmp) n = sizeof tmp - 1;
        fwrite(tmp, 1, n, pc->res);
        fflush(pc->res);
    }
}

static int sink_poll(void *ctx)
{
    pc_t *pc = (pc_t *)ctx;
    uint32_t t = ng_plat_millis();
    char p[200];
    if (pc->stop) return 1;
    if (ng_plat_user_stop()) { pc->stop = 1; return 1; }
    if (t - pc->last_poll < ng_pc_poll_ms) return 0;     /* /pc/ is a dcload call: once a second */
    pc->last_poll = t;
    snprintf(p, sizeof p, "%s/reload.req", pc->jobdir);
    if (pc_exists(p)) pc->stop = 1;
    snprintf(p, sizeof p, "%s/cancel.req", pc->jobdir);
    if (pc_exists(p)) pc->stop = 1;
    return pc->stop;
}

static uint32_t sink_millis(void *ctx) { (void)ctx; return ng_plat_millis(); }

/* ------------------------------------------------------------------ source */

typedef struct {
    char paths[NG_MAX_FILES][200];
    int n;
    FILE *f;
} src_t;

static int src_open(void *ctx, int idx, uint64_t off)
{
    src_t *s = (src_t *)ctx;
    char buf[256];
    if (idx < 0 || idx >= s->n) return -1;
    if (s->f) fclose(s->f);
    s->f = fopen(ng_plat_pcpath(s->paths[idx], buf, sizeof buf), "rb");
    if (!s->f) return -1;
    if (off && off <= 0x7FFFFFFFull) {
        if (fseek(s->f, (long)off, SEEK_SET) != 0) return -1;
    } else if (off) {                       /* beyond a 32-bit long: read and discard */
        static char skip[8192];
        uint64_t left = off;
        while (left) {
            size_t c = left > sizeof skip ? sizeof skip : (size_t)left;
            if (fread(skip, 1, c, s->f) != c) return -1;
            left -= c;
        }
    }
    return 0;
}

static int32_t src_read(void *ctx, void *b, uint32_t n)
{
    src_t *s = (src_t *)ctx;
    if (!s->f) return -1;
    return (int32_t)fread(b, 1, n, s->f);
}

static void src_close(void *ctx)
{
    src_t *s = (src_t *)ctx;
    if (s->f) fclose(s->f);
    s->f = NULL;
}

static src_t g_src;
static ng_manifest_t g_man;

static int load_src_list(const char *entry_dir, int nfiles, char *err, size_t en)
{
    char path[240], *txt = NULL, *p;
    long len;
    int n = 0;
    snprintf(path, sizeof path, "%s/src.txt", entry_dir);
    len = pc_slurp(path, &txt, 32768);
    if (len < 0) { snprintf(err, en, "cannot read src.txt"); return -1; }
    for (p = txt; *p;) {
        char *nl = strchr(p, '\n');
        size_t l = nl ? (size_t)(nl - p) : strlen(p);
        if (l && p[l - 1] == '\r') l--;
        if (l) {
            if (n >= NG_MAX_FILES || l >= sizeof g_src.paths[0]) { snprintf(err, en, "bad src.txt line"); free(txt); return -1; }
            memcpy(g_src.paths[n], p, l);
            g_src.paths[n][l] = 0;
            n++;
        }
        p = nl ? nl + 1 : p + strlen(p);
    }
    free(txt);
    if (n != nfiles) { snprintf(err, en, "src.txt has %d lines, manifest has %d files", n, nfiles); return -1; }
    g_src.n = n;
    g_src.f = NULL;
    return 0;
}

/* ------------------------------------------------------------------ job */

typedef struct { char kind[10]; char arg[120]; int mode; } op_t;

static int parse_job(const char *text, op_t *ops, int *nops, char *finish, size_t fn)
{
    const char *p = text;
    int first = 1;
    *nops = 0;
    snprintf(finish, fn, "default");
    while (*p) {
        const char *nl = strchr(p, '\n');
        size_t l = nl ? (size_t)(nl - p) : strlen(p);
        char line[200];
        char *sp;
        if (l >= sizeof line) return -1;
        memcpy(line, p, l);
        p = nl ? nl + 1 : p + l;
        if (l && line[l - 1] == 13) l--;        /* tolerate CRLF */
        line[l] = 0;
        if (!l) continue;
        if (first) {
            if (strcmp(line, "NETGD-JOB 1")) return -1;
            first = 0;
            continue;
        }
        sp = strchr(line, ' ');
        if (sp) *sp++ = 0;
        if (!strcmp(line, "finish")) { snprintf(finish, fn, "%s", sp ? sp : "default"); continue; }
        if (*nops >= MAX_OPS) return -1;
        if (!strcmp(line, "statfs") || !strcmp(line, "list") || !strcmp(line, "delete") || !strcmp(line, "verify") ||
            !strcmp(line, "put")) {
            op_t *o = &ops[(*nops)++];
            snprintf(o->kind, sizeof o->kind, "%s", line);
            o->arg[0] = 0;
            o->mode = NG_PUT_NEW;
            if (sp) snprintf(o->arg, sizeof o->arg, "%s", sp);
            if (!strcmp(line, "put")) {
                char *m = strchr(o->arg, ' ');
                if (m) {
                    *m++ = 0;
                    if (!strncmp(m, "resume", 6)) o->mode = NG_PUT_RESUME;
                    else if (!strncmp(m, "overwrite", 9)) o->mode = NG_PUT_OVERWRITE;
                    else if (strncmp(m, "new", 3)) return -1;
                }
                if (strlen(o->arg) != 4) return -1;
            }
        } else {
            return -1;
        }
    }
    return first ? -1 : 0;
}

static int do_put(const ng_sink_t *sk, const char *jobdir, const op_t *o)
{
    char dir[200], path[240], err[96];
    char *mtext = NULL;
    long mlen;
    ng_err_t e;
    snprintf(dir, sizeof dir, "%s/%s", jobdir, o->arg);
    snprintf(path, sizeof path, "%s/manifest.ngm", dir);
    mlen = pc_slurp(path, &mtext, 32768);
    if (mlen < 0) { ng_emit_error(sk, o->arg, NG_E_MANIFEST, 0, 0, "cannot read manifest.ngm"); return NG_E_MANIFEST; }
    if (ng_manifest_parse(mtext, (size_t)mlen, &g_man, err, sizeof err)) {
        ng_emit_error(sk, o->arg, NG_E_MANIFEST, 0, 0, err);
        free(mtext);
        return NG_E_MANIFEST;
    }
    if (load_src_list(dir, g_man.nfiles, err, sizeof err)) {
        ng_emit_error(sk, o->arg, NG_E_MANIFEST, 0, 0, err);
        free(mtext);
        return NG_E_MANIFEST;
    }
    {
        ng_src_t src = { &g_src, src_open, src_read, src_close };
        e = ng_put(sk, o->arg, &g_man, mtext, (size_t)mlen, &src, o->mode);
        src_close(&g_src);
    }
    free(mtext);
    return e;
}

static void (*g_ui)(const ng_ui_t *);
static void sink_ui(void *ctx, const ng_ui_t *u) { (void)ctx; if (g_ui) g_ui(u); }

int ng_pc_run(const char *jobdir, void (*ui)(const ng_ui_t *))
{
    static pc_t pc;
    ng_sink_t sk;
    op_t ops[MAX_OPS];
    int nops = 0, failed = 0, stopped = 0, halted = 0;
    char path[240], finish[130], *jtext = NULL;
    long jl;
    ng_err_t e;

    memset(&pc, 0, sizeof pc);
    snprintf(pc.jobdir, sizeof pc.jobdir, "%s", jobdir);
    pc.last_poll = ng_plat_millis();
    sk.ctx = &pc; sk.line = sink_line; sk.poll = sink_poll; sk.millis = sink_millis;
    g_ui = ui;
    sk.ui = sink_ui;

    snprintf(path, sizeof path, "%s/result.ngr", jobdir);
    {
        char buf[256];
        pc.res = fopen(ng_plat_pcpath(path, buf, sizeof buf), "wb");
    }
    ng_cfg_defaults();
    snprintf(path, sizeof path, "%s/force.req", jobdir);
    if (pc_exists(path)) ng_cfg.force = 1;
    snprintf(path, sizeof path, "%s/fragok.req", jobdir);
    if (pc_exists(path)) ng_cfg.allow_frag = 1;
    if (ng_pc_cfg_hook) ng_pc_cfg_hook();

    snprintf(path, sizeof path, "%s/job.ngj", jobdir);
    jl = pc_slurp(path, &jtext, 16384);
    if (jl < 0 || parse_job(jtext, ops, &nops, finish, sizeof finish)) {
        ng_emit_error(&sk, "job", NG_E_JOB, 0, 0, "job.ngj missing or not NETGD-JOB 1");
        ng_emitf(&sk, 0, "NGD:END ok=0");
        if (jtext) free(jtext);
        if (pc.res) fclose(pc.res);
        return 1;
    }
    free(jtext);

    e = ng_session_open(&sk);
    if (e) {
        ng_emitf(&sk, 0, "NGD:END ok=0");
        if (pc.res) fclose(pc.res);
        return 1;
    }

    for (int i = 0; i < nops && !stopped && !halted; i++) {
        const op_t *o = &ops[i];
        if (!strcmp(o->kind, "put")) e = (ng_err_t)do_put(&sk, jobdir, o);
        else if (!strcmp(o->kind, "statfs")) e = ng_statfs(&sk);
        else if (!strcmp(o->kind, "list")) e = ng_list(&sk, o->arg);
        else if (!strcmp(o->kind, "delete")) e = ng_delete(&sk, o->arg);
        else e = ng_verify(&sk, o->arg);
        if (e == NG_E_STOP) stopped = 1;
        else if (e) {
            failed = 1;
            if (e == NG_E_IO || e == NG_E_BUS || e == NG_E_FW) halted = 1;   /* hardware trouble: do not go on writing */
        }
    }

    if (stopped || halted) {
        ng_session_close(&sk, 1, NULL, 1);                     /* back to the default image, say nothing more */
        ng_emitf(&sk, 0, "NGD:END ok=0");
    } else {
        int mode = 1;
        const char *folder = NULL;
        if (!strcmp(finish, "none")) mode = 0;
        else if (!strncmp(finish, "mount ", 6)) { mode = 2; folder = finish + 6; }
        if (ng_session_close(&sk, mode, folder, 0) != NG_OK) failed = 1;
        ng_emitf(&sk, 0, "NGD:END ok=%d", failed ? 0 : 1);
    }
    if (pc.res) fclose(pc.res);
    return stopped ? 2 : (failed ? 1 : 0);
}

const char *ng_pc_find_jobdir(char *buf, size_t n)
{
    char path[240], root[] = "/pc/netgd", mapped[256];
    DIR *d;
    struct dirent *de;
    int found = 0;
    snprintf(path, sizeof path, "%s/job.ngj", NG_JOBDIR_DEFAULT);
    if (pc_exists(path)) { snprintf(buf, n, "%s", NG_JOBDIR_DEFAULT); return buf; }
    d = opendir(ng_plat_pcpath(root, mapped, sizeof mapped));
    if (!d) return NULL;
    while ((de = readdir(d)) != NULL) {
        if (de->d_name[0] == '.') continue;
        snprintf(path, sizeof path, "%s/%s/job.ngj", root, de->d_name);
        if (pc_exists(path)) {
            if (++found > 1) break;
            snprintf(buf, n, "%s/%s", root, de->d_name);
        }
    }
    closedir(d);
    return found == 1 ? buf : NULL;
}
