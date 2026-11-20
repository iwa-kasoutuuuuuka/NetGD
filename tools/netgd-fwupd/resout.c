#include "resout.h"
#include <stdio.h>
#include <string.h>

static void *s_open(const char *path, const char *mode) { return fopen(path, mode); }
static size_t s_write(const void *p, size_t n, void *f) { return fwrite(p, 1, n, (FILE *)f); }
static int s_flush(void *f) { return fflush((FILE *)f); }
static int s_close(void *f) { return fclose((FILE *)f); }
static const resout_io g_stdio = { s_open, s_write, s_flush, s_close };

const resout_io *resout_stdio_io(void) { return &g_stdio; }

int resout_open(resout *o, const resout_io *io, const char *path)
{
    memset(o, 0, sizeof(*o));
    o->io = io;
    o->f = io->open(path, "wb");
    if (!o->f) { o->error = 1; return -1; }
    return 0;
}

static int emit(resout *o, size_t n)
{
    if (!o->f || o->error) return -1;
    if (o->io->write(o->buf, n, o->f) != n) { o->error = 1; return -1; }
    o->written += n;
    if (o->io->flush(o->f) != 0) { o->error = 1; return -1; }
    o->fill = 0;
    return 0;
}

int resout_write(resout *o, const char *p, size_t n)
{
    if (!o->f || o->error) return -1;
    while (n) {
        size_t room = RESOUT_CHUNK - o->fill, take = n < room ? n : room;
        memcpy(o->buf + o->fill, p, take);
        o->fill += take; p += take; n -= take;
        if (o->fill == RESOUT_CHUNK && emit(o, RESOUT_CHUNK) != 0) return -1;
    }
    return 0;
}

int resout_sync(resout *o)
{
    if (!o->f || o->error) return -1;
    if (o->fill) return emit(o, o->fill);
    return o->io->flush(o->f);
}

int resout_close(resout *o)
{
    int r = resout_sync(o);
    if (o->f && o->io->close(o->f) != 0) r = -1;
    o->f = NULL;
    return r;
}
