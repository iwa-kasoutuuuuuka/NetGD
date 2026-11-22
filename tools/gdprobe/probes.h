/* gdprobe: the probe battery and the image-switch loop. Pure C: it talks to
 * the drive only through gd_ops, so the host tests run it against a fake drive.
 *
 * Every command goes through whitelist.c first (rc=refused, nothing sent, if
 * it is not in the table).
 */
#ifndef GDPROBE_PROBES_H
#define GDPROBE_PROBES_H
#include <stddef.h>
#include <stdint.h>
#include "cfg.h"
#include "gdtypes.h"

#define GDPROBE_VERSION "2"

typedef struct gp_sink {
    void *ctx;
    /* One finished record line (with its '\n'). is_hash = 1 for the many
     * per-sector HASH lines (the console skips those). */
    void (*line)(void *ctx, const char *s, size_t len, int is_hash);
    /* Called at section ends: flush what is buffered to /pc/. May be NULL. */
    void (*sync)(void *ctx);
} gp_sink;

/* Run everything the config asks for. Returns 0, or 1 if the run was
 * abandoned because the drive stopped answering (3 timeouts in a row; the
 * record says so). */
int gp_run(const gd_ops *ops, const gp_config *cfg, const gp_sink *sink);

#endif
