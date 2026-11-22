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

/* seq 25: what the G1 DMA engine did during one CD_READ done with the DMA feature bit (R-DMA probes).
 * The runner pre-fills the buffer with a known pattern; the transport programs the G1 DMA registers, runs the
 * command and reports the registers it saw. The number of bytes the DMA really wrote is measured by the runner
 * from the buffer (gd_result.total stays 0 here). */
typedef struct {
    uint32_t req_len;     /* bytes programmed into G1_ATA_DMA_LENGTH */
    uint32_t len_left;    /* G1_ATA_DMA_LENGTH read back after the command (0xFFFFFFFF = not read) */
    uint32_t addr_after;  /* G1_ATA_DMA_ADDRESS read back after the command */
    uint32_t drain;       /* bytes drained by PIO because the drive raised DRQ instead of doing DMA */
    uint32_t us_dma;      /* microseconds from "DMA started" until G1_ATA_DMA_STATUS read idle (0 = never) */
    uint8_t done;         /* 1 = the DMA engine reported idle before the deadline */
    uint8_t aborted;      /* 1 = the transport had to stop the DMA (timeout or drive finished without it) */
    uint8_t dma_st_end;   /* G1_ATA_DMA_STATUS (low byte) when the wait ended */
    uint8_t asic_nrm;     /* ASIC normal status bits seen: 1 = GD DMA end (bit 14), 2 = GD command (bit 0 ext) */
    uint8_t asic_err;     /* ASIC error bits: 1 = DMA overrun (bit 13), 2 = illegal address (bit 12) */
    uint8_t first;        /* which came first: 'd' DMA idle, 'g' GD-ROM interrupt, '-' neither */
    uint8_t bcl, bch;     /* byte-count registers at the end */
    uint8_t prot_ok;      /* G1_ATA_DMA_PROTECTION read back as the system-memory unlock value */
    uint8_t drq;          /* 1 = the drive still had DRQ up when the command completed (bytes left in the drive) */
    uint16_t drq_bc;      /* byte-count registers when that DRQ was first seen */
    uint8_t st_after;     /* ALTSTATUS after the drain */
    uint8_t tail_n;       /* first bytes of what was drained (tail[0..tail_n)) */
    uint8_t tail[64];
} gd_dma;

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
    /* --- seq 25; may be NULL (an older transport: the DMA probes are skipped). A CD_READ (0x30) PACKET with the
     * DMA feature bit set: `buf` is 32-byte aligned and `buflen` (a multiple of 32) bytes long, and the transport
     * programs the G1 DMA registers for `dma_len` (<= buflen) bytes into it. Refuses (GD_REFUSED, nothing sent)
     * any other opcode. Always leaves the DMA engine stopped. Fills *r and *d. */
    int (*packet_dma)(void *ctx, const uint8_t cdb[12], uint8_t *buf, uint32_t buflen, uint32_t dma_len,
                      uint32_t timeout_ms, gd_result *r, gd_dma *d);
} gd_ops;

#endif
