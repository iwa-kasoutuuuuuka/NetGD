/* FAT32 operations through FatFs R0.15b.  Design 5.2 / 5.5 / 8. */
#include <stdio.h>
#include <string.h>
#include "ngfs.h"
#include "ngdev.h"
#include "crc32.h"
#include "diskio.h"

#ifndef FA_MODIFIED
#define FA_MODIFIED 0x40        /* FatFs R0.15b internal flag (ff.c); pinned by the vendored version */
#endif

void (*ng_phase_hook)(const char *phase);
extern int ng_diskio_bus_error;

static FATFS g_fs;
static int g_mounted;
static uint32_t g_ckp_lba;
static uint8_t g_sec[512] __attribute__((aligned(32)));

FATFS *ngfs_volume(void) { return &g_fs; }

/* FatFs R0.15b's get_fileinfo() leaves fname[] alone for an entry without an LFN and at the
   end of the directory, so a caller that does not clear it loops on the old name. */
static FRESULT rd(DIR *d, FILINFO *fno)
{
    fno->fname[0] = 0;
    return f_readdir(d, fno);
}

static void phase(const char *p) { if (ng_phase_hook) ng_phase_hook(p); }

static int ieq(const char *a, const char *b)
{
    for (; *a && *b; a++, b++) {
        char x = *a, y = *b;
        if (x >= 'a' && x <= 'z') x -= 32;
        if (y >= 'a' && y <= 'z') y -= 32;
        if (x != y) return 0;
    }
    return *a == *b;
}

static const char *ext_of(const char *name)
{
    const char *d = strrchr(name, '.');
    return d ? d + 1 : "";
}

ng_err_t ngfs_map(FRESULT r)
{
    switch (r) {
    case FR_OK: return NG_OK;
    case FR_DISK_ERR: return ng_diskio_bus_error ? NG_E_BUS : NG_E_IO;
    case FR_NO_FILE: case FR_NO_PATH: return NG_E_NOENT;
    case FR_EXIST: return NG_E_EXIST;
    case FR_DENIED: return NG_E_NOSPC;
    case FR_INVALID_NAME: return NG_E_MANIFEST;
    case FR_NO_FILESYSTEM: return NG_E_FS;
    default: return NG_E_IO;
    }
}

static uint32_t clst2lba(DWORD clst)
{
    return (uint32_t)(g_fs.database + (LBA_t)(clst - 2) * g_fs.csize);
}

/* ------------------------------------------------------------------ volume */

/* The free-cluster count, from the FAT itself.  FatFs would do this in f_getfree through its
   one-sector window (= one double-read command pair per FAT sector: about 61,000 commands for the
   30,500 FAT sectors of a 128 GB card with 32 KiB clusters).  Here: NGFS_SCAN_SECTORS per command,
   each chunk read twice and compared by CRC-32 (ngblk_read_twice), 4 tries, then EIO.
   Both reads are needed: nothing on the firmware side vouches for the bytes of a READ (the FNV-1a
   of STATUS covers writes only), so a flip on the G1 bus can only be seen by reading again; and one
   flipped bit can turn a used entry into 0 (= one cluster too many reported free) or drop the
   only free cluster of a full card.  The second FAT copy is no substitute: it costs the same read
   and may legitimately differ after a power cut.
   Seeds FatFs exactly as its own f_getfree scan does (free_clst valid, fsi_flag |= 1); FatFs keeps
   the count up to date from there on.  If FSInfo gave no usable "next free" hint, the allocation
   search starts at the first free cluster instead of walking the used front of the FAT a sector at
   a time. */
#define NGFS_SCAN_SECTORS 256

/* The free-run table (see "run finder" below): the NGFS_NRUNS longest runs of free clusters seen by
   the last full scan, kept up to date by the claims.  Filled by the same pass that counts. */
#define NGFS_NRUNS 64
typedef struct { uint32_t start, len; } run_t;
static run_t g_runs[NGFS_NRUNS];
static int g_nruns;
static uint32_t g_runs_evicted;     /* the longest free run that is NOT in the table (0: the table is complete) */
static int g_runs_stale;            /* clusters were freed since the scan: the table may lack runs */
static ngfs_scan_tick g_alloc_tick;
static void *g_alloc_ctx;

static void runs_stale(void);
static void note_overwrite(const char *path);
static void runs_reset(void) { g_nruns = 0; g_runs_evicted = 0; g_runs_stale = 0; }

static void runs_add(uint32_t start, uint32_t len)
{
    int mi = 0;
    if (!len) return;
    if (g_nruns < NGFS_NRUNS) { g_runs[g_nruns].start = start; g_runs[g_nruns].len = len; g_nruns++; return; }
    for (int i = 1; i < g_nruns; i++) if (g_runs[i].len < g_runs[mi].len) mi = i;
    if (len > g_runs[mi].len) {
        if (g_runs[mi].len > g_runs_evicted) g_runs_evicted = g_runs[mi].len;
        g_runs[mi].start = start; g_runs[mi].len = len;
    } else if (len > g_runs_evicted) {
        g_runs_evicted = len;
    }
}

static inline int fat_free(const uint8_t *e) { return !(e[0] | e[1] | e[2] | (e[3] & 0x0F)); }

/* mount: count = 1 seeds FatFs (free_clst, last_clst); a later rescan (the table ran dry) leaves
   FatFs's own, always-current count alone.  Both rebuild the run table. */
static int scan_fat(ngfs_scan_tick tick, void *ctx, int mount)
{
    static uint8_t chunk[NGFS_SCAN_SECTORS * 512] __attribute__((aligned(32)));
    uint32_t nent = (uint32_t)g_fs.n_fatent;                  /* entries 0 .. nent-1; 0 and 1 are not clusters */
    uint32_t total = (uint32_t)(((uint64_t)nent * 4 + 511) / 512);
    uint32_t nfree = 0, first_free = 0, ent = 0, rs = 0, rl = 0;
    runs_reset();
    if (!mount && tick) tick(ctx, 0, total);
    for (uint32_t done = 0; done < total;) {
        uint32_t n = total - done > NGFS_SCAN_SECTORS ? NGFS_SCAN_SECTORS : total - done;
        if (ngblk_read_twice((uint32_t)g_fs.fatbase + done, n, chunk)) { runs_reset(); g_runs_stale = 1; return -1; }
        for (const uint8_t *e = chunk, *end = chunk + (size_t)n * 512; e < end && ent < nent; e += 4, ent++) {
            if (ent >= 2 && fat_free(e)) {
                if (!nfree++) first_free = ent;
                if (!rl++) rs = ent;
            } else if (rl) {
                runs_add(rs, rl);
                rl = 0;
            }
        }
        done += n;
        if (tick) tick(ctx, done, total);
    }
    if (rl) runs_add(rs, rl);
    if (mount) {
        g_fs.free_clst = nfree;
        g_fs.fsi_flag |= 1;
        if (first_free && (g_fs.last_clst < 2 || g_fs.last_clst >= nent)) g_fs.last_clst = first_free - 1;
    }
    return 0;
}

/* the checks shared by both mounts: sector 0, the partition type, f_mount, FAT32 at the partition start */
static ng_err_t mount_checked(char *why, size_t whyn)
{
    uint32_t start;
    FRESULT r;
    if (ngblk_read(0, 1, g_sec)) { snprintf(why, whyn, "cannot read sector 0"); return NG_E_IO; }
    if (g_sec[510] != 0x55 || g_sec[511] != 0xAA) { snprintf(why, whyn, "no MBR (partition table) on the card"); return NG_E_FS; }
    if (g_sec[446 + 4] != 0x0B && g_sec[446 + 4] != 0x0C) {
        snprintf(why, whyn, "first partition type 0x%02X is not FAT32", g_sec[446 + 4]);
        return NG_E_FS;
    }
    start = (uint32_t)g_sec[446 + 8] | (uint32_t)g_sec[446 + 9] << 8 | (uint32_t)g_sec[446 + 10] << 16 |
            (uint32_t)g_sec[446 + 11] << 24;
    r = f_mount(&g_fs, "0:", 1);
    if (r != FR_OK) { snprintf(why, whyn, "f_mount failed (%d)", (int)r); return r == FR_NO_FILESYSTEM ? NG_E_FS : ngfs_map(r); }
    if (g_fs.fs_type != FS_FAT32 || (uint32_t)g_fs.volbase != start) {
        snprintf(why, whyn, "volume is not FAT32 in the first partition");
        f_mount(NULL, "0:", 0);
        return NG_E_FS;
    }
    return NG_OK;
}

ng_err_t ngfs_mount(char *why, size_t whyn, ngfs_scan_tick tick, void *ctx)
{
    ng_err_t e;
    ngblk_readonly = 0;
    if ((e = mount_checked(why, whyn)) != NG_OK) return e;
    if (scan_fat(tick, ctx, 1)) {
        snprintf(why, whyn, "cannot read the FAT (no two reads agree)");
        f_mount(NULL, "0:", 0);
        return NG_E_IO;
    }
    g_mounted = 1;
    return NG_OK;
}

ng_err_t ngfs_mount_light(char *why, size_t whyn)
{
    ng_err_t e;
    ngblk_readonly = 1;                 /* before the first FatFs call: nothing below may write */
    if ((e = mount_checked(why, whyn)) != NG_OK) { ngblk_readonly = 0; return e; }
    runs_reset();                       /* no FAT scan: there is no run table */
    g_mounted = 1;
    return NG_OK;
}

void ngfs_unmount(void)
{
    if (g_mounted) f_mount(NULL, "0:", 0);
    g_mounted = 0;
    g_alloc_tick = NULL;
    g_alloc_ctx = NULL;
    if (!ngblk_readonly) ngdev_flush();
    ngblk_readonly = 0;
}

int ngfs_statfs(uint64_t *free_bytes, uint64_t *total_bytes, uint32_t *cluster_bytes)
{
    DWORD nclst;
    FATFS *fsp;
    uint32_t cb = (uint32_t)g_fs.csize * 512;
    if (f_getfree("0:", &nclst, &fsp) != FR_OK) return -1;
    if (free_bytes) *free_bytes = (uint64_t)nclst * cb;
    if (total_bytes) *total_bytes = (uint64_t)(g_fs.n_fatent - 2) * cb;
    if (cluster_bytes) *cluster_bytes = cb;
    return 0;
}

/* ------------------------------------------------------------------ paths */

void ngfs_dirpath(const char *image, char *out, size_t n)
{
    if (!image[0] || !strcmp(image, "/")) snprintf(out, n, "0:/");
    else snprintf(out, n, "0:/%s", image);
}

int ngfs_mkdir_p(const char *dir)
{
    char tmp[NG_PATH_BUF];
    size_t n = strlen(dir);
    FRESULT r = FR_OK;
    if (n >= sizeof tmp) return FR_INVALID_NAME;
    memcpy(tmp, dir, n + 1);
    for (size_t i = 3; i <= n; i++) {
        if (tmp[i] == '/' || tmp[i] == 0) {
            char c = tmp[i];
            tmp[i] = 0;
            r = f_mkdir(tmp);
            if (r != FR_OK && r != FR_EXIST) return r;
            tmp[i] = c;
            if (!c) break;
        }
    }
    return FR_OK;
}

/* One GDEMU.CFG line (modified in place).  -1 not an image= line, 0 image= with no value, 1 value copied. */
static int cfg_line(char *line, char *out, size_t n)
{
    char *eq, *k, *v, *e;
    while (*line == ' ' || *line == '\t') line++;
    if (*line == '#' || !*line) return -1;
    eq = strchr(line, '=');
    if (!eq) return -1;
    *eq = 0;
    k = line;
    v = eq + 1;
    for (e = k + strlen(k); e > k && (e[-1] == ' ' || e[-1] == '\t' || e[-1] == '\r'); e--) e[-1] = 0;
    if (!ieq(k, "image")) return -1;
    while (*v == ' ' || *v == '\t') v++;
    for (e = v + strlen(v); e > v && (e[-1] == ' ' || e[-1] == '\t' || e[-1] == '\r'); e--) e[-1] = 0;
    if (!*v) return 0;
    snprintf(out, n, "%s", v);
    return 1;
}

/* The whole file is read (bounded: 64 KiB), line by line across 512-byte reads, so an image= line after
   a long comment still counts (R89 C11).  A line longer than the line buffer is cut (only a comment or
   garbage is that long; image= values are at most NG_PATH_MAX). */
int ngfs_default_image(char *out, size_t n)
{
    FIL fp;
    char buf[512], line[NG_PATH_BUF + 32];
    UINT got = 0;
    size_t ll = 0;
    uint32_t total = 0;
    int r;
    if (f_open(&fp, "0:/GDEMU.CFG", FA_READ) != FR_OK) return 0;
    while (total < 65536 && f_read(&fp, buf, sizeof buf, &got) == FR_OK && got) {
        total += got;
        for (UINT i = 0; i < got; i++) {
            if (buf[i] != '\n') {
                if (ll + 1 < sizeof line) line[ll++] = buf[i];
                continue;
            }
            line[ll] = 0;
            ll = 0;
            if ((r = cfg_line(line, out, n)) >= 0) { f_close(&fp); return r; }
        }
    }
    f_close(&fp);
    if (ll) {
        line[ll] = 0;
        if ((r = cfg_line(line, out, n)) >= 0) return r;
    }
    return 0;
}

/* Canonical form of a folder path for comparisons: components separated by single '/', no leading
   or trailing '/', "//" collapsed, "." dropped, and trailing spaces/dots of each component removed
   (FatFs ignores them, so "01." and "01 " name folder 01).  Case is folded by the compare. */
void ngfs_norm_path(const char *in, char *out, size_t n)
{
    size_t o = 0;
    const char *p = in;
    if (!n) return;
    while (*p) {
        const char *s, *e;
        size_t l;
        while (*p == '/' || *p == '\\') p++;
        s = p;
        while (*p && *p != '/' && *p != '\\') p++;
        e = p;
        while (e > s && (e[-1] == ' ' || e[-1] == '.')) e--;
        l = (size_t)(e - s);
        if (!l) continue;
        if (o && o + 1 < n) out[o++] = '/';
        for (size_t i = 0; i < l && o + 1 < n; i++) out[o++] = s[i];
    }
    out[o] = 0;
}

static int under(const char *image, const char *prot)       /* both normalized, prot non-empty */
{
    size_t n = strlen(prot);
    if (strlen(image) < n) return 0;
    for (size_t i = 0; i < n; i++) {
        char a = image[i], b = prot[i];
        if (a >= 'a' && a <= 'z') a -= 32;
        if (b >= 'a' && b <= 'z') b -= 32;
        if (a != b) return 0;
    }
    return image[n] == 0 || image[n] == '/';
}

int ngfs_protected(const char *image)
{
    char def[NG_PATH_BUF], nd[NG_PATH_BUF], ni[NG_PATH_BUF];
    ngfs_norm_path(image, ni, sizeof ni);
    if (!ni[0]) return 1;                                    /* the root: never an image folder */
    if (under(ni, "01")) return 1;
    if (ngfs_default_image(def, sizeof def)) {
        ngfs_norm_path(def, nd, sizeof nd);
        if (nd[0] && under(ni, nd)) return 1;
    }
    return 0;
}

/* ------------------------------------------------------------------ NETGD.CKP */

static void put16(uint8_t *p, uint32_t v) { p[0] = (uint8_t)v; p[1] = (uint8_t)(v >> 8); }
static void put32(uint8_t *p, uint32_t v) { put16(p, v); put16(p + 2, v >> 16); }
static uint32_t get16(const uint8_t *p) { return (uint32_t)p[0] | (uint32_t)p[1] << 8; }
static uint32_t get32(const uint8_t *p) { return get16(p) | get16(p + 2) << 16; }

static void ckp_encode(const ng_ckp_t *c, uint8_t *b)
{
    memset(b, 0, 512);
    memcpy(b, "NGCK", 4);
    put16(b + 4, 1);
    put16(b + 6, c->state);
    put32(b + 8, c->mf_crc);
    put16(b + 12, c->files_done);
    put16(b + 14, c->flags);
    put32(b + 16, (uint32_t)c->offset);
    put32(b + 20, (uint32_t)(c->offset >> 32));
    put32(b + 24, c->crc);
    put32(b + 508, ng_crc32(0, b, 508));
}

static int ckp_decode(const uint8_t *b, uint32_t mf_crc, ng_ckp_t *c)
{
    if (memcmp(b, "NGCK", 4) || get16(b + 4) != 1 || get32(b + 508) != ng_crc32(0, b, 508)) return -1;
    c->state = (uint16_t)get16(b + 6);
    c->mf_crc = get32(b + 8);
    c->files_done = (uint16_t)get16(b + 12);
    c->flags = (uint16_t)get16(b + 14);
    c->offset = (uint64_t)get32(b + 16) | (uint64_t)get32(b + 20) << 32;
    c->crc = get32(b + 24);
    if (c->state < 1 || c->state > 3 || c->mf_crc != mf_crc) return -1;
    return 0;
}

int ngfs_ckp_read(const char *dir, uint32_t mf_crc, ng_ckp_t *c)
{
    char path[NG_PATH_BUF];
    FIL fp;
    UINT got = 0;
    uint8_t b[512];
    FRESULT r;
    snprintf(path, sizeof path, "%s/NETGD.CKP", dir);
    r = f_open(&fp, path, FA_READ);
    if (r != FR_OK) return 1;
    r = f_read(&fp, b, 512, &got);
    f_close(&fp);
    if (r != FR_OK || got != 512) return -1;
    return ckp_decode(b, mf_crc, c);
}

ng_err_t ngfs_ckp_open(const char *dir)
{
    char path[NG_PATH_BUF];
    FIL fp;
    FRESULT r;
    snprintf(path, sizeof path, "%s/NETGD.CKP", dir);
    r = f_open(&fp, path, FA_READ);
    if (r != FR_OK) return ngfs_map(r);
    g_ckp_lba = clst2lba(fp.obj.sclust);
    f_close(&fp);
    return NG_OK;
}

ng_err_t ngfs_ckp_create(const char *dir, const ng_ckp_t *c)
{
    char path[NG_PATH_BUF];
    FIL fp;
    UINT bw = 0;
    FRESULT r;
    uint8_t b[512];
    snprintf(path, sizeof path, "%s/NETGD.CKP", dir);
    ckp_encode(c, b);
    note_overwrite(path);
    r = f_open(&fp, path, FA_CREATE_ALWAYS | FA_WRITE);
    if (r == FR_OK) r = f_write(&fp, b, 512, &bw);
    if (r == FR_OK && bw != 512) r = FR_DISK_ERR;
    if (r == FR_OK) r = f_sync(&fp);
    f_close(&fp);
    if (r != FR_OK) return ngfs_map(r);
    return ngfs_ckp_open(dir);
}

ng_err_t ngfs_ckp_write(const ng_ckp_t *c)
{
    int r;
    ckp_encode(c, g_sec);
    r = ngblk_write(g_ckp_lba, 1, g_sec);
    return r == 0 ? NG_OK : (r == -2 ? NG_E_BUS : NG_E_IO);
}

ng_err_t ngfs_ckp_delete(const char *dir)
{
    char path[NG_PATH_BUF];
    FRESULT r;
    snprintf(path, sizeof path, "%s/NETGD.CKP", dir);
    note_overwrite(path);
    r = f_unlink(path);
    return (r == FR_OK || r == FR_NO_FILE) ? NG_OK : ngfs_map(r);
}

ng_err_t ngfs_write_mf(const char *dir, const void *text, size_t n)
{
    char path[NG_PATH_BUF];
    FIL fp;
    UINT bw = 0;
    FRESULT r;
    snprintf(path, sizeof path, "%s/NETGD.MF", dir);
    note_overwrite(path);
    r = f_open(&fp, path, FA_CREATE_ALWAYS | FA_WRITE);
    if (r == FR_OK) r = f_write(&fp, text, (UINT)n, &bw);
    if (r == FR_OK && bw != n) r = FR_DENIED;
    if (r == FR_OK) r = f_sync(&fp);
    f_close(&fp);
    return ngfs_map(r);
}

/* ------------------------------------------------------------------ probe / delete / list */

static int is_image_ext(const char *name)
{
    const char *e = ext_of(name);
    return ieq(e, "GDI") || ieq(e, "CDI") || ieq(e, "NGP");
}

int ngfs_is_image_dir(const char *dir)
{
    DIR d;
    FILINFO fno;
    int found = 0;
    if (f_opendir(&d, dir) != FR_OK) return 0;
    while (rd(&d, &fno) == FR_OK && fno.fname[0]) {
        if (fno.fattrib & AM_DIR) continue;
        if (ieq(fno.fname, "NETGD.MF") || is_image_ext(fno.fname)) { found = 1; break; }
    }
    f_closedir(&d);
    return found;
}

ng_err_t ngfs_probe(const char *dir, const ng_manifest_t *m, ng_probe_t *p)
{
    DIR d;
    FILINFO fno;
    char ngp[16], fin[16];
    FRESULT r;
    memset(p, 0, sizeof *p);
    ng_manifest_ngp_name(m, ngp, fin);
    r = f_opendir(&d, dir);
    if (r == FR_NO_PATH || r == FR_NO_FILE) return NG_OK;
    if (r != FR_OK) return ngfs_map(r);
    p->exists = 1;
    while ((r = rd(&d, &fno)) == FR_OK && fno.fname[0]) {
        if (fno.fattrib & AM_DIR) { p->has_subdir = 1; continue; }
        p->nfiles++;
        if (ieq(fno.fname, "NETGD.MF")) p->has_mf = 1;
        else if (ieq(fno.fname, "NETGD.CKP")) p->ckp = -1;
        if (ieq(fno.fname, ngp)) p->has_ngp = 1;
        if (ieq(fno.fname, fin)) p->has_final = 1;
        if (is_image_ext(fno.fname)) p->has_image_ext = 1;
    }
    f_closedir(&d);
    if (r != FR_OK) return ngfs_map(r);
    if (p->has_mf) {
        char path[NG_PATH_BUF];
        FIL fp;
        static uint8_t mf[32768];
        UINT got = 0;
        uint32_t crc;
        snprintf(path, sizeof path, "%s/NETGD.MF", dir);
        if (f_open(&fp, path, FA_READ) == FR_OK) {
            FSIZE_t sz = f_size(&fp);
            if (sz <= sizeof mf && f_read(&fp, mf, (UINT)sz, &got) == FR_OK && got == sz) {
                crc = ng_crc32(0, mf, got);
                p->mf_matches = (crc == m->crc);
                {
                    static ng_manifest_t other;
                    char why[32];
                    p->mf_valid = ng_manifest_parse((const char *)mf, got, &other, why, sizeof why) == 0;
                }
            }
            f_close(&fp);
        }
    }
    if (p->ckp) {
        int rc = ngfs_ckp_read(dir, m->crc, &p->ck);
        p->ckp = (rc == 0) ? (int)p->ck.state : -1;
    }
    return NG_OK;
}

/* find one directory entry matching `which`; 0 none, 1 found, -1 error */
enum { F_DESC = 1, F_OTHER, F_CKP, F_MF, F_DIR };

static int find_entry(const char *dir, int which, char *name, size_t n)
{
    DIR d;
    FILINFO fno;
    int found = 0;
    FRESULT r;
    r = f_opendir(&d, dir);
    if (r != FR_OK) return -1;
    while (rd(&d, &fno) == FR_OK && fno.fname[0]) {
        int isdir = (fno.fattrib & AM_DIR) != 0;
        int hit;
        if (which == F_DIR) hit = isdir;
        else if (isdir) hit = 0;
        else if (which == F_DESC) hit = is_image_ext(fno.fname);
        else if (which == F_CKP) hit = ieq(fno.fname, "NETGD.CKP");
        else if (which == F_MF) hit = ieq(fno.fname, "NETGD.MF");
        else hit = !ieq(fno.fname, "NETGD.CKP") && !ieq(fno.fname, "NETGD.MF");
        if (hit) { snprintf(name, n, "%s", fno.fname); found = 1; break; }
    }
    f_closedir(&d);
    return found;
}

ng_err_t ngfs_delete_contents(const char *dir)
{
    static const int order[] = { F_DESC, F_OTHER, F_CKP, F_MF };
    char name[NG_FNAME_BUF], path[NG_PATH_BUF];
    int rc;
    if (find_entry(dir, F_DIR, name, sizeof name) == 1) return NG_E_EXIST;   /* design 5.3: sub-folders -> fail */
    for (unsigned i = 0; i < sizeof order / sizeof order[0]; i++) {
        while ((rc = find_entry(dir, order[i], name, sizeof name)) == 1) {
            FRESULT r;
            if (snprintf(path, sizeof path, "%s/%s", dir, name) >= (int)sizeof path) return NG_E_IO;   /* never a cut-off path */
            note_overwrite(path);
            r = f_unlink(path);
            if (r != FR_OK) return ngfs_map(r);
        }
        if (rc < 0) return NG_E_IO;
    }
    return NG_OK;
}

ng_err_t ngfs_delete_image(const char *dir, int remove_dir)
{
    ng_err_t e = ngfs_delete_contents(dir);
    if (e) return e;
    if (remove_dir) {
        FRESULT r;
        note_overwrite(dir);
        r = f_unlink(dir);
        if (r != FR_OK) return ngfs_map(r);
    }
    return NG_OK;
}

uint64_t ngfs_dir_bytes(const char *dir)
{
    DIR d;
    FILINFO fno;
    uint64_t sum = 0;
    uint32_t cb = (uint32_t)g_fs.csize * 512;
    if (f_opendir(&d, dir) != FR_OK) return 0;
    while (rd(&d, &fno) == FR_OK && fno.fname[0])
        if (!(fno.fattrib & AM_DIR)) sum += ((uint64_t)fno.fsize + cb - 1) / cb * cb;
    f_closedir(&d);
    return sum;
}

int ngfs_stat_size(const char *path, uint32_t *size)
{
    FILINFO fno;
    if (f_stat(path, &fno) != FR_OK || (fno.fattrib & AM_DIR)) return -1;
    *size = (uint32_t)fno.fsize;
    return 0;
}

const char *ngfs_classify(const char *dir)
{
    DIR d;
    FILINFO fno;
    int fin = 0, ngp = 0, ckp = 0;
    if (f_opendir(&d, dir) != FR_OK) return "other";
    while (rd(&d, &fno) == FR_OK && fno.fname[0]) {
        if (fno.fattrib & AM_DIR) continue;
        if (ieq(fno.fname, "NETGD.CKP")) ckp = 1;
        else if (ieq(ext_of(fno.fname), "NGP")) ngp = 1;
        else if (ieq(ext_of(fno.fname), "GDI") || ieq(ext_of(fno.fname), "CDI")) fin = 1;
    }
    f_closedir(&d);
    if (ngp || ckp) return "partial";
    return fin ? "complete" : "other";
}

#define LIST_BATCH 16
static ng_err_t list_rec(const char *rel, int depth, ng_list_cb cb, void *ctx)
{
    char dir[NG_PATH_BUF], child[NG_PATH_BUF];
    ngfs_dirpath(rel, dir, sizeof dir);
    /* Sub-folders are taken LIST_BATCH at a time (the FatFs window and the stack are small; recursion
       would reuse the window anyway): re-open the directory and skip what was already reported. */
    for (int skip = 0;; skip += LIST_BATCH) {
        char list[LIST_BATCH][NG_FNAME_BUF];
        DIR d;
        FILINFO fno;
        int n = 0, seen = 0;
        if (f_opendir(&d, dir) != FR_OK) return skip ? NG_E_IO : NG_E_NOENT;
        while (n < LIST_BATCH && rd(&d, &fno) == FR_OK && fno.fname[0])
            if ((fno.fattrib & AM_DIR) && seen++ >= skip) snprintf(list[n++], sizeof list[0], "%s", fno.fname);
        f_closedir(&d);
        for (int i = 0; i < n; i++) {
            const char *st;
            char cdir[NG_PATH_BUF];
            int need = (rel[0] && strcmp(rel, "/")) ? snprintf(child, sizeof child, "%s/%s", rel, list[i])
                                                    : snprintf(child, sizeof child, "%s", list[i]);
            if (need < 0 || (size_t)need >= sizeof child) return NG_E_IO;   /* never report a cut-off name */
            ngfs_dirpath(child, cdir, sizeof cdir);
            st = ngfs_classify(cdir);
            cb(ctx, st, child);
            if (!strcmp(st, "other") && depth + 1 < NG_PATH_PARTS) list_rec(child, depth + 1, cb, ctx);
        }
        if (n < LIST_BATCH) break;
    }
    return NG_OK;
}

ng_err_t ngfs_list(const char *folder, ng_list_cb cb, void *ctx)
{
    return list_rec(folder, 0, cb, ctx);
}

/* ------------------------------------------------------------------ allocation + data */

/* ---- run finder -------------------------------------------------------------------------------
   f_expand (FatFs's search for one contiguous run) walks the FAT from last_clst through get_fat and
   FatFs's one-sector window: two read commands per FAT sector, up to ~61,000 on a large fragmented
   or nearly full card, and no output.  So the search is done here, in bulk, and FatFs is only asked
   to allocate at a place we know is free:
     1. the mount scan leaves a table of the NGFS_NRUNS longest free runs (scan_fat);
     2. a candidate run (best fit: the shortest one that is long enough) is CLAIMED: its FAT sectors
        are read again, 256 per command, each read twice and CRC-compared (ngblk_read_twice), and the
        first `need` consecutive free clusters in it are located (FatFs may have taken a cluster of it
        since the scan: a directory, NETGD.MF ...); what is left of the run goes back into the table;
     3. last_clst is set to that cluster and f_expand is called: it finds the run at once (it reads the
        same entries once more through its own window: about two commands per 128 clusters of the file),
        marks the chain, and keeps free_clst / last_clst / the FAT copies the way FatFs always does.
        FatFs does the allocation, so nothing here writes the FAT;
     4. no candidate: if nothing was freed since the scan (g_runs_stale) and every run the table does not
        hold is shorter than `need`, there is no run: NG_E_FRAG without touching FatFs.  Otherwise the
        table is rebuilt by a full bulk scan (progress lines through ngfs_set_scan_progress) and 2. is
        tried once more; a table that still holds no run is final.
   The table is only a hint: a wrong entry costs a bulk read, never a wrong allocation (claims read the
   FAT again; f_expand reads it again).  Entries are exact or too optimistic, never pessimistic, until a
   cluster is freed: unlink / truncation set g_runs_stale. */

void ngfs_set_scan_progress(ngfs_scan_tick tick, void *ctx) { g_alloc_tick = tick; g_alloc_ctx = ctx; }

static void runs_stale(void) { g_runs_stale = 1; }

/* The first `need` consecutive free clusters inside the table entry `idx`.  The entry leaves the table;
   the free pieces of it that were looked at and are not the answer, and the unread rest, go back.
   1 found (*at), 0 none in it, -1 read error. */
static int claim_in_run(int idx, uint32_t need, uint32_t *at)
{
    static uint8_t chunk[NGFS_SCAN_SECTORS * 512] __attribute__((aligned(32)));
    uint32_t cl = g_runs[idx].start, end = g_runs[idx].start + g_runs[idx].len;
    uint32_t cs = 0, cn = 0;                       /* the free piece in progress */
    int found = 0;
    g_runs[idx] = g_runs[--g_nruns];
    while (cl < end && !found) {
        uint32_t s0 = cl / 128, s1 = (end - 1) / 128;
        uint32_t n = s1 - s0 + 1 > NGFS_SCAN_SECTORS ? NGFS_SCAN_SECTORS : s1 - s0 + 1;
        uint32_t lim = (s0 + n) * 128 < end ? (s0 + n) * 128 : end;
        if (ngblk_read_twice((uint32_t)g_fs.fatbase + s0, n, chunk)) { runs_stale(); return -1; }
        for (; cl < lim; cl++) {
            if (fat_free(chunk + (size_t)(cl - s0 * 128) * 4)) {
                if (!cn++) cs = cl;
                if (cn == need) { found = 1; cl++; break; }
            } else if (cn) {
                runs_add(cs, cn);
                cn = 0;
            }
        }
    }
    if (found) { *at = cs; runs_add(cl, end - cl); }
    else if (cn) runs_add(cs, cn);
    return found;
}

/* the table entry to try: the shortest one with len >= need; -1 none */
static int best_run(uint32_t need)
{
    int bi = -1;
    for (int i = 0; i < g_nruns; i++)
        if (g_runs[i].len >= need && (bi < 0 || g_runs[i].len < g_runs[bi].len)) bi = i;
    return bi;
}

/* Where a run of `need` clusters is free: 1 *at, 0 there is none, -1 error, -2 unknown (use FatFs). */
static int find_run(uint32_t need, uint32_t *at)
{
    int rescanned = 0;
    for (;;) {
        int i = best_run(need);
        if (i >= 0) {
            int r = claim_in_run(i, need, at);
            if (r) return r;
            continue;
        }
        if (!g_runs_stale && g_runs_evicted < need) return 0;
        if (rescanned) return g_runs_evicted >= need ? -2 : 0;
        if (scan_fat(g_alloc_tick, g_alloc_ctx, 0)) return -1;
        rescanned = 1;
    }
}

/* a file or folder that exists is about to lose its clusters (overwrite / truncate / delete) */
static void note_overwrite(const char *path)
{
    FILINFO fno;
    if (f_stat(path, &fno) == FR_OK && (fno.fsize || (fno.fattrib & AM_DIR))) runs_stale();
}

ng_err_t ngfs_alloc_file(const char *path, uint32_t size, int allow_frag, int *fragmented)
{
    FIL fp;
    FRESULT r;
    note_overwrite(path);
    r = f_open(&fp, path, FA_CREATE_ALWAYS | FA_WRITE);
    if (r != FR_OK) return ngfs_map(r);
    if (size) {
        uint32_t cb = (uint32_t)g_fs.csize * 512;
        uint32_t need = (uint32_t)(((uint64_t)size + cb - 1) / cb), at = 0;
        DWORD hint = g_fs.last_clst;
        int f = find_run(need, &at);
        if (f < 0 && f != -2) { f_close(&fp); return ng_diskio_bus_error ? NG_E_BUS : NG_E_IO; }
        if (f == 1) {
            g_fs.last_clst = at;                           /* f_expand starts its search here */
            r = f_expand(&fp, size, 1);
            if (r != FR_OK) {
                g_fs.last_clst = hint;
                runs_add(at, need);                        /* the claim did not happen: give the run back (R89 C10) */
            }
        } else {
            /* f == 0: no run on the card.  f == -2: the table gave out.  FatFs's own unhinted f_expand search
               is never used (it walks the whole FAT for minutes, and R0.15b can return a wrong run count after
               a wrap, R89 C13): both are "no contiguous area". */
            r = FR_DENIED;
        }
        if (r == FR_DENIED) {
            if (!allow_frag) { f_close(&fp); return NG_E_FRAG; }
            r = f_lseek(&fp, size);                         /* chain by chain: fragmented */
            runs_stale();                                   /* it took clusters the table may still list */
            if (r == FR_OK && f_tell(&fp) != size) { f_close(&fp); return NG_E_NOSPC; }
            if (r == FR_OK) r = f_lseek(&fp, 0);
            *fragmented = 1;
        }
    }
    if (r == FR_OK) r = f_sync(&fp);
    f_close(&fp);
    return ngfs_map(r);
}

/* The direct-write path trusts that the file is ONE run of clusters.  Check it on the card: the
   start cluster is a data cluster, the run lies inside the volume, and the FAT chain really is
   sclust, sclust+1, ... ending in an end-of-chain mark.  0 ok, 1 not usable (*why says why), -1 read error. */
static int check_contiguous(DWORD sclust, uint32_t size, const char **why)
{
    uint32_t cb = (uint32_t)g_fs.csize * 512;
    uint64_t ncl = ((uint64_t)size + cb - 1) / cb;
    static uint8_t fatsec[512] __attribute__((aligned(32)));
    LBA_t cur = (LBA_t)0 - 1;
    if (sclust < 2) { *why = "start cluster below 2"; return 1; }
    if ((uint64_t)sclust + ncl > g_fs.n_fatent) { *why = "run extends beyond the volume"; return 1; }
    for (uint64_t k = 0; k < ncl; k++) {
        DWORD c = sclust + (DWORD)k, v;
        LBA_t sec = g_fs.fatbase + c / 128;
        const uint8_t *e;
        if (sec != cur) {
            if (ngblk_read((uint32_t)sec, 1, fatsec)) return -1;
            cur = sec;
        }
        e = fatsec + (c % 128) * 4;
        v = ((DWORD)e[0] | (DWORD)e[1] << 8 | (DWORD)e[2] << 16 | (DWORD)e[3] << 24) & 0x0FFFFFFF;
        if (k + 1 < ncl) { if (v != c + 1) { *why = "FAT chain is not contiguous"; return 1; } }
        else if (v < 0x0FFFFFF8) { *why = "FAT chain does not end after the last cluster"; return 1; }
    }
    return 0;
}

ng_err_t ngfs_wfile_open(ng_wfile_t *w, const char *path, uint32_t size, int contig)
{
    FRESULT r;
    memset(w, 0, sizeof *w);
    snprintf(w->path, sizeof w->path, "%s", path);
    w->size = size;
    w->contig = contig;
    r = f_open(&w->fp, path, contig ? FA_READ : (FA_READ | FA_WRITE));
    if (r != FR_OK) return ngfs_map(r);
    if (f_size(&w->fp) != size) { f_close(&w->fp); return NG_E_STATE; }
    if (contig && size) {
        const char *why = "";
        int c = check_contiguous(w->fp.obj.sclust, size, &why);
        if (c < 0) { f_close(&w->fp); return NG_E_IO; }
        if (c > 0) {                    /* the checkpoint claimed more than the card shows: take the slow, safe path */
            f_close(&w->fp);
            w->note = why;
            w->contig = contig = 0;
            r = f_open(&w->fp, path, FA_READ | FA_WRITE);
            if (r != FR_OK) return ngfs_map(r);
        }
    }
    if (contig) {
        if (size) {
            w->base_lba = clst2lba(w->fp.obj.sclust);
            if ((uint64_t)w->base_lba + ((uint64_t)size + 511) / 512 > ngdev_sectors()) { f_close(&w->fp); return NG_E_IO; }
        }
        f_close(&w->fp);
    } else {
        w->open = 1;
    }
    return NG_OK;
}

ng_err_t ngfs_wfile_write(ng_wfile_t *w, uint32_t off, const void *buf, uint32_t n)
{
    if ((uint64_t)off + n > w->size || (off & 511)) return NG_E_STATE;
    if (!w->contig) {
        UINT bw = 0;
        FRESULT r = FR_OK;
        if (f_tell(&w->fp) != off) r = f_lseek(&w->fp, off);
        if (r == FR_OK) r = f_write(&w->fp, buf, n, &bw);
        if (r == FR_OK && bw != n) r = FR_DENIED;
        return ngfs_map(r);
    }
    {
        const uint8_t *p = (const uint8_t *)buf;
        uint32_t lba = w->base_lba + off / 512, full = n / 512;
        while (full) {
            uint32_t c = full > 256 ? 256 : full;
            int r = ngblk_write(lba, c, p);
            if (r) return r == -2 ? NG_E_BUS : NG_E_IO;
            lba += c; p += (size_t)c * 512; full -= c;
        }
        if (n & 511) {                                     /* last, partial sector: zero padded */
            int r;
            memset(g_sec, 0, 512);
            memcpy(g_sec, p, n & 511);
            r = ngblk_write(lba, 1, g_sec);
            if (r) return r == -2 ? NG_E_BUS : NG_E_IO;
        }
    }
    return NG_OK;
}

ng_err_t ngfs_wfile_close(ng_wfile_t *w)
{
    if (w->open) {
        FRESULT r = f_close(&w->fp);
        w->open = 0;
        return ngfs_map(r);
    }
    return NG_OK;
}

ng_err_t ngfs_file_crc(const char *path, uint32_t *size, uint32_t *crc, void *buf, uint32_t bufn,
                       int (*tick)(void *ctx, uint32_t done), void *ctx)
{
    FIL fp;
    FRESULT r = f_open(&fp, path, FA_READ);
    uint32_t c = 0, done = 0;
    int stop = 0;
    if (r != FR_OK) return ngfs_map(r);
    *size = (uint32_t)f_size(&fp);
    for (;;) {
        UINT got = 0;
        r = f_read(&fp, buf, bufn, &got);
        if (r != FR_OK) break;
        if (!got) break;
        c = ng_crc32(c, buf, got);
        done += got;
        if (tick && tick(ctx, done)) { stop = 1; break; }
    }
    f_close(&fp);
    *crc = c;
    return stop ? NG_E_STOP : ngfs_map(r);
}

/* ------------------------------------------------------------------ commit */

ng_err_t ngfs_commit_ext(const char *path, const char *oldext, const char *newext)
{
    FIL fp;
    FRESULT r;
    LBA_t sect;
    uint32_t off;
    uint8_t *e;

    r = f_open(&fp, path, FA_READ | FA_WRITE | FA_OPEN_EXISTING);
    if (r != FR_OK) return ngfs_map(r);
    sect = fp.dir_sect;
    off = (uint32_t)(fp.dir_ptr - g_fs.win);
    if (off >= 512 || (off & 31) || sect == 0) { f_close(&fp); return NG_E_IO; }
    fp.flag |= FA_MODIFIED;
    r = f_close(&fp);                    /* f_sync -> sync_fs: the window is clean afterwards */
    if (r != FR_OK) return ngfs_map(r);
    if (g_fs.wflag) return NG_E_IO;

    if (ngblk_read((uint32_t)sect, 1, g_sec)) return NG_E_IO;
    e = g_sec + off;
    if ((e[11] & 0x1F) == 0x0F || (e[11] & AM_DIR) || memcmp(e + 8, oldext, 3)) return NG_E_STATE;
    phase("commit");
    memcpy(e + 8, newext, 3);
    {
        int w = ngblk_write((uint32_t)sect, 1, g_sec);       /* the one sector write */
        g_fs.winsect = (LBA_t)0 - 1;                         /* drop FatFs's copy of it */
        if (w) return w == -2 ? NG_E_BUS : NG_E_IO;
    }
    return NG_OK;
}
