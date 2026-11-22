/* gdprobe: types shared between the raw G1 access layer (gdraw.c, KOS only)
 * and the probe runner (probes.c, pure C). */
#ifndef GDPROBE_GDTYPES_H
#define GDPROBE_GDTYPES_H
#include <stddef.h>
#include <stdint.h>

/* Result codes of one command. Their names are written to the record by
 * rec_rc_name() and must stay stable (compare.py reads them). */
enum {
    GD_OK = 0,      /* completed, ERR bit clear */
    GD_CHECK = 1,   /* completed with the ERR bit set (CHECK CONDITION) */
    GD_TIMEOUT = 2, /* a wait ran out; the drive may still be busy */
    GD_BUSY = 3,    /* BSY/DRQ never cleared before we could send anything */
    GD_PROTO = 4,   /* unexpected phase (e.g. DRQ without data, endless data) */
    GD_REFUSED = 5  /* the whitelist refused the command: nothing was sent */
};

typedef struct {
    int rc;            /* GD_* */
    uint8_t status;    /* ATA status register at completion */
    uint8_t error;     /* ATA error register (sense key in the high nibble) */
    uint8_t irr;       /* interrupt reason (sector count) register */
    uint32_t total;    /* bytes the drive sent in the data-in phase */
    uint32_t stored;   /* bytes that fit in the caller's buffer */
    uint64_t us;       /* time from PACKET command to completion, microseconds */
    /* Passive observations (R137 a, b); the has_* flag says whether the transport filled the value
     * (the host fake drive may leave them out, then the record omits the field). */
    uint8_t cdbst;     /* ALTSTATUS at the moment DRQ was seen for the CDB (PACKET only) */
    uint8_t has_cdbst;
    uint8_t secnr;     /* GD-ROM "sector number" status register 0xA05F708C at completion:
                          low nibble = drive status, high nibble = disc format */
    uint8_t has_secnr;
} gd_result;

/* The transport the runner uses. gdraw.c implements it on real hardware; the
 * host tests implement it with a scripted fake drive. */
typedef struct gd_ops {
    void *ctx;
    /* Send a 12-byte PACKET command and collect up to `cap` bytes of data-in.
     * Must always fill *r. Returns r->rc. */
    int (*packet)(void *ctx, const uint8_t cdb[12], uint8_t *buf, uint32_t cap,
                  uint32_t timeout_ms, gd_result *r);
    /* ATA IDENTIFY PACKET DEVICE (0xA1), 512 bytes. May be NULL. */
    int (*identify)(void *ctx, uint8_t *buf512, uint32_t timeout_ms, gd_result *r);
    uint64_t (*now_us)(void *ctx);
    void (*sleep_ms)(void *ctx, uint32_t ms);
    /* Progress text for the console. May be NULL. */
    void (*progress)(void *ctx, const char *msg);
} gd_ops;

#endif
