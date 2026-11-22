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

/* A passive snapshot of the drive's task-file registers (R148 probes). Every field is a plain register READ:
 * ALTSTATUS (does not clear the interrupt), ERROR, interrupt reason (sector count), SECNR (sector number:
 * low nibble = drive status, high nibble = disc format) and the byte-count pair (LBA mid / high; after a
 * reset an ATAPI device puts its signature 0x14 / 0xEB there). */
typedef struct {
    uint8_t altst;
    uint8_t err;
    uint8_t irr;
    uint8_t secnr;
    uint8_t bcl;
    uint8_t bch;
} gd_regs;

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
    /* --- R148 additions; each may be NULL (an older transport: the probe that needs it is skipped) --- */
    /* Read the registers above, nothing else, no command. Returns 0 (or non-zero if the bus is unavailable). */
    int (*regs)(void *ctx, gd_regs *out);
    /* ATA DEVICE RESET (0x08): write the command register once, then wait (bounded by timeout_ms) for BSY to
     * clear and snapshot the registers. *us = time from the write until BSY was clear (or the timeout);
     * *bsy_seen = 1 if BSY was seen set after the write. Returns GD_OK or GD_TIMEOUT (or GD_BUSY if the bus
     * lock failed). The caller has already checked the ATA whitelist. */
    int (*ata_reset)(void *ctx, uint32_t timeout_ms, gd_regs *after, uint32_t *us, int *bsy_seen);
    /* Busy-wait about `us` microseconds without touching the drive (the register poll needs 2-5 ms steps,
     * finer than a thread sleep may give). May be NULL: sleep_ms is used instead. */
    void (*spin_us)(void *ctx, uint32_t us);
} gd_ops;

#endif
