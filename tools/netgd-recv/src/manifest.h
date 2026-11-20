/* Manifest (design 5.1): parse and validate.  Checks are the same as netgd-send's
   (manifest.py), plus the descriptor must be 8.3-safe (5.2, ngfs_commit_ext). */
#ifndef NG_MANIFEST_H
#define NG_MANIFEST_H
#include <stdint.h>
#include <stddef.h>

#define NG_MAX_FILES 128
#define NG_NAME_MAX 127
#define NG_PATH_MAX 96          /* gamecfg::MAX_NEXT */
#define NG_PATH_PARTS 3         /* firmware MAX_PATH_PARTS */

typedef struct {
    uint32_t size;
    uint32_t crc;
    char name[NG_NAME_MAX + 1];
} ng_mfile_t;

typedef struct {
    char image[NG_PATH_MAX + 1];
    int is_cdi;
    int nfiles;
    int descriptor;             /* index into files[] */
    uint32_t crc;               /* CRC32 of the manifest bytes exactly as read (NETGD.CKP offset 8) */
    uint64_t total_bytes;
    ng_mfile_t files[NG_MAX_FILES];
} ng_manifest_t;

/* An image folder path as put accepts it: 1..NG_PATH_MAX chars, at most NG_PATH_PARTS '/'-separated
   components, each printable ASCII, FAT-safe, non-empty, no leading/trailing '.' or ' ' and not "..".
   (So no leading/trailing '/', no "//".)  Used for every folder argument of the job.  0 ok, -1 + err. */
int ng_path_ok(const char *path, char *err, size_t errn);

/* returns 0, or -1 with a message in err */
int ng_manifest_parse(const char *text, size_t len, ng_manifest_t *m, char *err, size_t errn);

/* 8.3 name of the .NGP placeholder of the descriptor ("DISC.NGP"), and the final
   name with the real extension ("DISC.GDI"). Both buffers: at least 13 bytes. */
void ng_manifest_ngp_name(const ng_manifest_t *m, char *ngp, char *final);

#endif
