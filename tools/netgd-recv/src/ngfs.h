/* FAT32 operations on the SD card through FatFs (design 3.2 choice A, 5.2).
   Everything that touches FatFs internals (FIL.dir_sect, FATFS.database, the window)
   is in ngfs.c: that is the part that pins the FatFs version (R0.15b). */
#ifndef NG_FS_H
#define NG_FS_H
#include <stdint.h>
#include "ff.h"
#include "ng.h"
#include "manifest.h"

#define NG_PATH_BUF 320

/* NETGD.CKP (design 5.2): 512 bytes, little endian */
#define CKP_ALLOCATING 1
#define CKP_TRANSFER   2
#define CKP_COMPLETE   3
#define CKP_FL_FRAG    0x0001   /* at least one file is not contiguous */

typedef struct {
    uint16_t state;
    uint32_t mf_crc;
    uint16_t files_done;
    uint16_t flags;
    uint64_t offset;
    uint32_t crc;
} ng_ckp_t;

/* what is in an image folder (from ngfs_probe) */
typedef struct {
    int exists;             /* folder exists */
    int has_subdir;
    int nfiles;             /* plain files in it */
    int has_mf, mf_matches; /* NETGD.MF present / equals the manifest */
    int mf_valid;           /* NETGD.MF parses as a complete manifest (a torn write of ours does not) */
    int ckp;                /* 0 absent, -1 present but invalid/foreign, else state 1..3 */
    ng_ckp_t ck;            /* valid when ckp > 0 */
    int has_ngp, has_final; /* DESCRIPTOR.NGP / DESCRIPTOR.GDI|CDI (any file with the real ext counts) */
    int has_image_ext;      /* any .GDI/.CDI/.NGP (an image folder, ours or not) */
} ng_probe_t;

/* ---- volume ---- */
/* First partition, FAT32 only.  Mounting counts the free clusters itself: the whole FAT is read in
   256-sector commands, each one twice (ngblk_read_twice), and the count is handed to FatFs, so that
   neither f_getfree nor FSInfo is ever the source of the free space (FF_FS_NOFSINFO stays 1).
   tick (may be NULL) is called after every command pair with the FAT sectors done / to do. */
typedef void (*ngfs_scan_tick)(void *ctx, uint32_t done, uint32_t total);
/* The same kind of callback for the search for a contiguous free run (ngfs_alloc_file): called only when
   the run table from the mount scan cannot answer and the FAT is scanned again; (0, total) first, then
   after every command pair, the last call has done == total.  NULL = silent. */
void ngfs_set_scan_progress(ngfs_scan_tick tick, void *ctx);
ng_err_t ngfs_mount(char *why, size_t whyn, ngfs_scan_tick tick, void *ctx);
/* Read-only mount for the menu (netgd-menu): the same checks as ngfs_mount, but the FAT is NOT scanned
   (no free-space count, no run table) and ngblk_readonly is set, so nothing is ever written.  Only
   f_opendir/f_readdir/f_closedir, f_open(FA_READ)/f_read/f_lseek/f_close/f_stat, ngfs_default_image
   and ngfs_classify may be used while mounted this way.  ngfs_unmount ends it. */
ng_err_t ngfs_mount_light(char *why, size_t whyn);
void ngfs_unmount(void);
int ngfs_statfs(uint64_t *free_bytes, uint64_t *total_bytes, uint32_t *cluster_bytes);
FATFS *ngfs_volume(void);
ng_err_t ngfs_map(FRESULT r);                  /* FatFs result -> NG error (uses the last ngblk failure kind) */

/* ---- paths ---- */
void ngfs_dirpath(const char *image, char *out, size_t n);     /* "0:/" + image */
int ngfs_mkdir_p(const char *dir);                              /* all levels; FRESULT */
/* canonical folder path: no leading/trailing "/", "//" collapsed, trailing space/dot of components dropped */
void ngfs_norm_path(const char *in, char *out, size_t n);
int ngfs_protected(const char *image);                          /* GDEMU.CFG default, "01" (5.5) */
int ngfs_default_image(char *out, size_t n);                    /* GDEMU.CFG image=, 0 if none */

/* ---- image folder ---- */
ng_err_t ngfs_probe(const char *dir, const ng_manifest_t *m, ng_probe_t *p);
uint64_t ngfs_dir_bytes(const char *dir);                       /* cluster-rounded size of the plain files in dir */
ng_err_t ngfs_delete_image(const char *dir, int remove_dir);    /* descriptor first, then the rest, then the folder */
ng_err_t ngfs_delete_contents(const char *dir);                 /* the same, keeping the folder */
int ngfs_is_image_dir(const char *dir);                         /* NETGD.MF or .GDI/.CDI/.NGP present */

ng_err_t ngfs_write_mf(const char *dir, const void *text, size_t n);
ng_err_t ngfs_ckp_create(const char *dir, const ng_ckp_t *c);   /* creates the file and remembers its sector */
ng_err_t ngfs_ckp_open(const char *dir);                        /* remember the sector of an existing one */
ng_err_t ngfs_ckp_write(const ng_ckp_t *c);                     /* ONE sector write */
ng_err_t ngfs_ckp_delete(const char *dir);
/* returns 0 ok, 1 absent, -1 invalid */
int ngfs_ckp_read(const char *dir, uint32_t mf_crc, ng_ckp_t *c);

/* ---- allocation and data ---- */
ng_err_t ngfs_alloc_file(const char *path, uint32_t size, int allow_frag, int *fragmented);

typedef struct {
    char path[NG_PATH_BUF];
    uint32_t size;
    int contig;             /* direct sector writes (allocation is one run) */
    uint32_t base_lba;
    FIL fp;
    int open;
    const char *note;       /* set when the direct path was asked for but refused (why) */
} ng_wfile_t;

ng_err_t ngfs_wfile_open(ng_wfile_t *w, const char *path, uint32_t size, int contig);
/* Write n bytes at off.  off % 512 == 0.  n is a multiple of 512 unless it ends the file. */
ng_err_t ngfs_wfile_write(ng_wfile_t *w, uint32_t off, const void *buf, uint32_t n);
ng_err_t ngfs_wfile_close(ng_wfile_t *w);

int ngfs_stat_size(const char *path, uint32_t *size);          /* 0 if it is a file */

/* read a file back through FatFs: CRC32 + size check.  tick (may be NULL) is called after every
   bufn-byte read with the bytes done so far; non-zero stops the read (returns NG_E_STOP). */
ng_err_t ngfs_file_crc(const char *path, uint32_t *size, uint32_t *crc, void *buf, uint32_t bufn,
                       int (*tick)(void *ctx, uint32_t done), void *ctx);

/* ---- commit (5.2 step 5) ---- */
/* Rewrite the 3 extension bytes of the 8.3 entry of `path` (DESC.NGP) to `newext`
   ("GDI"/"CDI") in ONE sector write.  Verifies the entry first. */
ng_err_t ngfs_commit_ext(const char *path, const char *oldext, const char *newext);

/* ---- listing (for `list`) ---- */
/* "complete" (real .GDI/.CDI), "partial" (.NGP or NETGD.CKP), "other" */
const char *ngfs_classify(const char *dir);
typedef void (*ng_list_cb)(void *ctx, const char *state, const char *path);
ng_err_t ngfs_list(const char *folder, ng_list_cb cb, void *ctx);

/* hook for the host tests: called at points where a power cut is interesting */
extern void (*ng_phase_hook)(const char *phase);

#endif
