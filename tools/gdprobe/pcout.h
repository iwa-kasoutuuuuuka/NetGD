/* gdprobe: buffered writer for /pc/ (dcload-ip). Pure C; the file functions
 * are injected so the host test can check the chunking.
 *
 * KNOWHOW (Dreamcast-Dev-Tools-for-BBA): a big single fwrite to /pc/ can hang
 * dcload-ip. So: never one write over 4096 bytes, and fflush after every one.
 */
#ifndef GDPROBE_PCOUT_H
#define GDPROBE_PCOUT_H
#include <stddef.h>

#define PCOUT_CHUNK 4096

typedef struct pc_io {
    void *(*open)(const char *path, const char *mode);
    size_t (*write)(const void *p, size_t n, void *f);
    int (*flush)(void *f);
    int (*close)(void *f);
} pc_io;

typedef struct {
    const pc_io *io;
    void *f;
    char buf[PCOUT_CHUNK];
    size_t fill;
    size_t written;   /* bytes handed to io->write */
    int error;
} pcout;

const pc_io *pcout_stdio_io(void);                             /* fopen/fwrite/fflush/fclose */
int pcout_open(pcout *o, const pc_io *io, const char *path);   /* 0 = ok */
int pcout_write(pcout *o, const char *p, size_t n);            /* buffered; writes full 4096-byte chunks */
int pcout_sync(pcout *o);                                      /* writes the partial chunk (<4096) + flush */
int pcout_close(pcout *o);                                     /* sync + close */

#endif
