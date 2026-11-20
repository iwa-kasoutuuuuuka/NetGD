/* netgd-fwupd: buffered writer for /pc/ (dcload-ip). Pure C; the file functions
 * are injected so the host test can check the chunking.
 *
 * KNOWHOW (Dreamcast-Dev-Tools-for-BBA): a big single fwrite to /pc/ can hang
 * dcload-ip. So: never one write over 4096 bytes, and fflush after every one.
 */
#ifndef NETGD_FWUPD_RESOUT_H
#define NETGD_FWUPD_RESOUT_H
#include <stddef.h>

#define RESOUT_CHUNK 4096

typedef struct resout_io {
    void *(*open)(const char *path, const char *mode);
    size_t (*write)(const void *p, size_t n, void *f);
    int (*flush)(void *f);
    int (*close)(void *f);
} resout_io;

typedef struct {
    const resout_io *io;
    void *f;
    char buf[RESOUT_CHUNK];
    size_t fill;
    size_t written;   /* bytes handed to io->write */
    int error;
} resout;

const resout_io *resout_stdio_io(void);                             /* fopen/fwrite/fflush/fclose */
int resout_open(resout *o, const resout_io *io, const char *path);   /* 0 = ok */
int resout_write(resout *o, const char *p, size_t n);            /* buffered; writes full 4096-byte chunks */
int resout_sync(resout *o);                                      /* writes the partial chunk (<4096) + flush */
int resout_close(resout *o);                                     /* sync + close */

#endif
