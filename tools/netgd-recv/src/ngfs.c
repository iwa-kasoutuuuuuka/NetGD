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

ng_err_t ngfs_mount(char *why, size_t whyn)
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
    g_mounted = 1;
    return NG_OK;
}

void ngfs_unmount(void)
{
    if (g_mounted) f_mount(NULL, "0:", 0);
    g_mounted = 0;
    ngdev_flush();
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

int ngfs_default_image(char *out, size_t n)
{
    FIL fp;
    char buf[512];
    UINT got = 0;
    char *line, *nl;
    if (f_open(&fp, "0:/GDEMU.CFG", FA_READ) != FR_OK) return 0;
    f_read(&fp, buf, sizeof buf - 1, &got);
    f_close(&fp);
    buf[got] = 0;
    for (line = buf; line && *line; line = nl ? nl + 1 : NULL) {
        char *eq, *k, *v, *e;
        nl = strchr(line, '\n');
        if (nl) *nl = 0;
        while (*line == ' ' || *line == '\t') line++;
        if (*line == '#' || !*line) continue;
        eq = strchr(line, '=');
        if (!eq) continue;
        *eq = 0;
        k = line;
        v = eq + 1;
        for (e = k + strlen(k); e > k && (e[-1] == ' ' || e[-1] == '\t' || e[-1] == '\r'); e--) e[-1] = 0;
        if (!ieq(k, "image")) continue;
        while (*v == ' ' || *v == '\t') v++;
        for (e = v + strlen(v); e > v && (e[-1] == ' ' || e[-1] == '\t' || e[-1] == '\r'); e--) e[-1] = 0;
        if (!*v) return 0;
        snprintf(out, n, "%s", v);
        return 1;
    }
    return 0;
}

static int under(const char *image, const char *prot)
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
    char def[NG_PATH_MAX + 1];
    if (under(image, "01")) return 1;
    if (ngfs_default_image(def, sizeof def) && under(image, def)) return 1;
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
    char name[NG_NAME_MAX + 1], path[NG_PATH_BUF];
    int rc;
    if (find_entry(dir, F_DIR, name, sizeof name) == 1) return NG_E_EXIST;   /* design 5.3: sub-folders -> fail */
    for (unsigned i = 0; i < sizeof order / sizeof order[0]; i++) {
        while ((rc = find_entry(dir, order[i], name, sizeof name)) == 1) {
            FRESULT r;
            snprintf(path, sizeof path, "%s/%s", dir, name);
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
        FRESULT r = f_unlink(dir);
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

static ng_err_t list_rec(const char *rel, int depth, ng_list_cb cb, void *ctx)
{
    char dir[NG_PATH_BUF], child[NG_PATH_BUF];
    DIR d;
    FILINFO fno;
    ngfs_dirpath(rel, dir, sizeof dir);
    if (f_opendir(&d, dir) != FR_OK) return NG_E_NOENT;
    /* collect this level first: recursion would reuse the FatFs window anyway */
    {
        char list[32][NG_NAME_MAX + 1];
        int n = 0;
        while (n < 32 && rd(&d, &fno) == FR_OK && fno.fname[0])
            if (fno.fattrib & AM_DIR) snprintf(list[n++], sizeof list[0], "%s", fno.fname);
        f_closedir(&d);
        for (int i = 0; i < n; i++) {
            const char *st;
            if (rel[0] && strcmp(rel, "/")) snprintf(child, sizeof child, "%s/%s", rel, list[i]);
            else snprintf(child, sizeof child, "%s", list[i]);
            ngfs_dirpath(child, dir, sizeof dir);
            st = ngfs_classify(dir);
            cb(ctx, st, child);
            if (!strcmp(st, "other") && depth + 1 < NG_PATH_PARTS) list_rec(child, depth + 1, cb, ctx);
        }
    }
    return NG_OK;
}

ng_err_t ngfs_list(const char *folder, ng_list_cb cb, void *ctx)
{
    return list_rec(folder, 0, cb, ctx);
}

/* ------------------------------------------------------------------ allocation + data */

ng_err_t ngfs_alloc_file(const char *path, uint32_t size, int allow_frag, int *fragmented)
{
    FIL fp;
    FRESULT r = f_open(&fp, path, FA_CREATE_ALWAYS | FA_WRITE);
    if (r != FR_OK) return ngfs_map(r);
    if (size) {
        r = f_expand(&fp, size, 1);
        if (r == FR_DENIED) {
            if (!allow_frag) { f_close(&fp); return NG_E_FRAG; }
            r = f_lseek(&fp, size);                         /* chain by chain: fragmented */
            if (r == FR_OK && f_tell(&fp) != size) { f_close(&fp); return NG_E_NOSPC; }
            if (r == FR_OK) r = f_lseek(&fp, 0);
            *fragmented = 1;
        }
    }
    if (r == FR_OK) r = f_sync(&fp);
    f_close(&fp);
    return ngfs_map(r);
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
    if (contig) {
        if (size) {
            w->base_lba = clst2lba(w->fp.obj.sclust);
            if ((uint64_t)w->base_lba + (size + 511) / 512 > ngdev_sectors()) { f_close(&w->fp); return NG_E_IO; }
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

ng_err_t ngfs_file_crc(const char *path, uint32_t *size, uint32_t *crc, void *buf, uint32_t bufn)
{
    FIL fp;
    FRESULT r = f_open(&fp, path, FA_READ);
    uint32_t c = 0;
    if (r != FR_OK) return ngfs_map(r);
    *size = (uint32_t)f_size(&fp);
    for (;;) {
        UINT got = 0;
        r = f_read(&fp, buf, bufn, &got);
        if (r != FR_OK) break;
        if (!got) break;
        c = ng_crc32(c, buf, got);
    }
    f_close(&fp);
    *crc = c;
    return ngfs_map(r);
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
