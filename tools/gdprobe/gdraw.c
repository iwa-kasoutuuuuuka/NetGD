/* gdprobe: raw G1 ATAPI access. KOS 2.3.0 APIs used (checked against the fork):
 *   _g1_ata_sem                kernel/arch/dreamcast/hardware/cdrom.c:57, the
 *                              semaphore cdrom.c itself serialises the G1 bus on
 *                              (sem_wait / sem_signal, include/kos/sem.h)
 *   timer_us_gettime64         include/kos/timer.h
 *   thd_sleep                  include/kos/thread.h
 * g1_ata_mutex_lock/unlock are deliberately NOT used: unlock calls
 * g1_ata_select_device(G1_ATA_MASTER) (g1ata.c:216-222), i.e. it writes the
 * DEVICE_SELECT register after every command -- something neither KOS's cdrom
 * driver nor openMenu does to a GDEMU. Taking the semaphore directly keeps the
 * only register writes the ones below: FEATURES, byte count, COMMAND, DATA.
 * Register addresses are the ones g1ata.c and the openMenu gdemu_sdk.c use. */
#include <kos.h>
#include <kos/sem.h>
#include <kos/timer.h>
#include <stdio.h>
#include <string.h>
#include "gdraw.h"

#define GD_ALTSTATUS    0xA05F7018u /* read */
#define GD_DATA         0xA05F7080u
#define GD_ERROR        0xA05F7084u /* read */
#define GD_FEATURES     0xA05F7084u /* write */
#define GD_IRQ_REASON   0xA05F7088u /* read */
/* Read-only here. KOS g1ata.c:90 calls it G1_ATA_LBA_LOW / G1_ATA_CHS_SECTOR (0xA05F708C, Read/Write);
 * on the GD-ROM interface it is the "sector number" register: low nibble = drive status, high nibble =
 * disc format. Reading it has no side effect; gdprobe never writes it. */
#define GD_SECNR        0xA05F708Cu /* read */
#define GD_LBA_MID      0xA05F7090u /* byte count low  */
#define GD_LBA_HIGH     0xA05F7094u /* byte count high */
#define GD_STATUS       0xA05F709Cu /* read; reading clears the interrupt */
#define GD_COMMAND      0xA05F709Cu /* write */
#define GD_EXT_INT_STAT 0xA05F6904u /* bit 0 = GD-ROM interrupt */

#define SR_ERR  0x01
#define SR_DRQ  0x08
#define SR_BSY  0x80

#define ATAPI_PACKET        0xA0
#define ATA_IDENTIFY_PACKET 0xA1

/* PIO byte-count limit written before PACKET: one sector per DRQ block, like
 * a real GD-ROM. The data loop handles any block size the drive really uses. */
#define BYTE_LIMIT 0x0800
#define MAX_TOTAL  (8u * 1024 * 1024)   /* a drive that streams more than this is broken */

/* Defined in cdrom.c; g1ata.c declares it the same way. */
extern semaphore_t _g1_ata_sem;

static int bus_lock(void) { return sem_wait(&_g1_ata_sem); }
static void bus_unlock(void) { sem_signal(&_g1_ata_sem); }

#define OUT8(a, v)  (*(volatile uint8_t *)(a) = (v))
#define IN8(a)      (*(volatile uint8_t *)(a))
#define IN16(a)     (*(volatile uint16_t *)(a))
#define IN32(a)     (*(volatile uint32_t *)(a))
#define OUT16(a, v) (*(volatile uint16_t *)(a) = (v))

static uint64_t now_us(void) { return timer_us_gettime64(); }

#define G1_BUS_PROT        0xA05F74E4u   /* G1_ATA_BUS_PROTECTION | P2 */
#define G1_BUS_PROT_STATUS 0xA05F74ECu   /* G1_ATA_BUS_PROTECTION_STATUS | P2 */
#define G1_BUS_PASSED      3u

unsigned gdraw_bus_prepare(char *buf, unsigned len)
{
    volatile uint32_t *bios = (volatile uint32_t *)0xA0000000u;
    unsigned before, after, alt_before, alt_after, size = 0, p;
    uint64_t t0, t1;

    bus_lock();
    before = IN32(G1_BUS_PROT_STATUS) & 0xFFu;
    alt_before = IN8(GD_ALTSTATUS);
    t0 = now_us();
    if (before != G1_BUS_PASSED) {
        /* Same as cdrom_init: a BIOS starting with 0xe6ff passes with 1 KB. */
        size = (*(volatile uint16_t *)0xA0000000u == 0xe6ffu) ? 0x400u : 0x200000u;
        *(volatile uint32_t *)G1_BUS_PROT = (size == 0x400u) ? 0x3ffu : 0x1fffffu;
        for (p = 0; p < size / 4u; p++)
            (void)bios[p];
    }
    t1 = now_us();
    after = IN32(G1_BUS_PROT_STATUS) & 0xFFu;
    alt_after = IN8(GD_ALTSTATUS);
    bus_unlock();

    snprintf(buf, len, "g1bus prot_before=%u prot_after=%u bios_read=%u us=%lu altstatus_before=0x%02x altstatus_after=0x%02x",
             before, after, size, (unsigned long)(t1 - t0), alt_before, alt_after);
    return after;
}

/* Wait until (ALTSTATUS & mask) == 0. 1 = ok, 0 = timed out. */
static int wait_clear(uint8_t mask, uint64_t deadline)
{
    for (;;) {
        if (!(IN8(GD_ALTSTATUS) & mask))
            return 1;
        if (now_us() >= deadline)
            return 0;
    }
}

/* Wait for DRQ (1), or ERR (-1), or timeout (0). */
static int wait_drq(uint64_t deadline)
{
    for (;;) {
        uint8_t s = IN8(GD_ALTSTATUS);
        if (!(s & SR_BSY)) {
            if (s & SR_DRQ) return 1;
            if (s & SR_ERR) return -1;
        }
        if (now_us() >= deadline)
            return 0;
    }
}

/* After writing PACKET: wait for BSY=0 && DRQ=1 (1), or timeout (0). ERR is
 * ignored on purpose: until the device raises BSY, the status still shows the
 * previous command's CHECK bit, and PACKET itself never ends in ERR before the
 * CDB phase. Treating that stale ERR as a failure would leave the device
 * waiting for a CDB that never comes. openMenu also waits for DRQ only. */
static int wait_pkt_drq(uint64_t deadline, uint8_t *seen)
{
    for (;;) {
        uint8_t s = IN8(GD_ALTSTATUS);
        if (!(s & SR_BSY) && (s & SR_DRQ)) {
            *seen = s;      /* the very read that saw DRQ: no extra bus access (R137 a) */
            return 1;
        }
        if (now_us() >= deadline)
            return 0;
    }
}

static int wait_irq(uint64_t deadline)
{
    for (;;) {
        if (IN32(GD_EXT_INT_STAT) & 1)
            return 1;
        if (now_us() >= deadline)
            return 0;
    }
}

static void settle(void)
{
    /* ATA: wait ~400 ns after a command write before trusting the status. */
    int i;
    for (i = 0; i < 4; i++)
        (void)IN8(GD_ALTSTATUS);
}

static int gd_packet(void *ctx, const uint8_t cdb[12], uint8_t *buf, uint32_t cap,
                     uint32_t timeout_ms, gd_result *r)
{
    uint64_t t0, deadline;
    uint8_t st = 0;
    uint32_t total = 0;
    int i;
    (void)ctx;
    memset(r, 0, sizeof(*r));
    if (bus_lock()) {
        r->rc = GD_BUSY;
        return r->rc;
    }
    t0 = now_us();
    deadline = t0 + (uint64_t)timeout_ms * 1000u;

    if (!wait_clear(SR_BSY | SR_DRQ, deadline)) {
        r->rc = GD_BUSY;
        r->status = IN8(GD_ALTSTATUS);
        goto done;
    }
    (void)IN32(GD_STATUS);          /* clear a stale interrupt */
    OUT8(GD_FEATURES, 0);           /* PIO */
    OUT8(GD_LBA_MID, BYTE_LIMIT & 0xFF);
    OUT8(GD_LBA_HIGH, BYTE_LIMIT >> 8);
    OUT8(GD_COMMAND, ATAPI_PACKET);
    settle();

    if (!wait_pkt_drq(deadline, &r->cdbst)) { r->rc = GD_TIMEOUT; r->status = IN8(GD_ALTSTATUS); goto done; }

    r->has_cdbst = 1;
    (void)IN32(GD_STATUS);
    for (i = 0; i < 6; i++)
        OUT16(GD_DATA, (uint16_t)(cdb[2 * i] | (cdb[2 * i + 1] << 8)));

    for (;;) {
        if (!wait_irq(deadline) || !wait_clear(SR_BSY, deadline)) {
            r->rc = GD_TIMEOUT;
            r->status = IN8(GD_ALTSTATUS);
            goto done;
        }
        st = IN8(GD_STATUS);        /* also clears the interrupt */
        if (st & SR_ERR) {
            r->error = IN8(GD_ERROR);
            r->rc = GD_CHECK;
            break;
        }
        if (st & SR_DRQ) {
            uint32_t n = IN8(GD_LBA_MID) | ((uint32_t)IN8(GD_LBA_HIGH) << 8);
            uint32_t words = (n + 1) / 2, w;
            if (n == 0 || total + n > MAX_TOTAL) {
                r->rc = GD_PROTO;
                break;
            }
            for (w = 0; w < words; w++) {
                uint16_t v = IN16(GD_DATA);
                uint32_t off = total + 2 * w;
                if (off < cap) buf[off] = (uint8_t)v;
                if (off + 1 < cap) buf[off + 1] = (uint8_t)(v >> 8);
            }
            total += n;
            continue;
        }
        break;                      /* no DRQ, no ERR: command complete */
    }
    r->status = st;
done:
    r->irr = IN8(GD_IRQ_REASON);
    r->secnr = IN8(GD_SECNR);       /* passive read (R137 b) */
    r->has_secnr = 1;
    r->total = total;
    r->stored = total < cap ? total : cap;
    r->us = now_us() - t0;
    bus_unlock();
    return r->rc;
}

/* ATA IDENTIFY PACKET DEVICE (0xA1). The GD-ROM answers 80 bytes (SPI spec
 * table 3.5, 00h-4Fh), not ATA's 512: read words only while DRQ is up.
 * Read-only; still opt-in (config identify=on) because it is not something
 * the BIOS is known to send to a GDEMU. */
static int gd_identify(void *ctx, uint8_t *buf, uint32_t timeout_ms, gd_result *r)
{
    uint64_t t0, deadline;
    int d, w;
    (void)ctx;
    memset(r, 0, sizeof(*r));
    if (bus_lock()) {
        r->rc = GD_BUSY;
        return r->rc;
    }
    t0 = now_us();
    deadline = t0 + (uint64_t)timeout_ms * 1000u;
    if (!wait_clear(SR_BSY | SR_DRQ, deadline)) {
        r->rc = GD_BUSY;
        goto done;
    }
    (void)IN32(GD_STATUS);
    OUT8(GD_COMMAND, ATA_IDENTIFY_PACKET);
    settle();
    d = wait_drq(deadline);
    /* A stale CHECK bit from the previous command can show before the device
     * raises BSY: only believe ERR once the command has had 1 ms. */
    while (d < 0 && now_us() - t0 < 1000u)
        d = wait_drq(deadline);
    if (d == 0) {
        r->rc = GD_TIMEOUT;
    } else if (d < 0) {
        r->error = IN8(GD_ERROR);
        r->status = IN8(GD_STATUS);
        r->rc = GD_CHECK;
    } else {
        for (w = 0; w < 256 && (IN8(GD_ALTSTATUS) & SR_DRQ); w++) {
            uint16_t v = IN16(GD_DATA);
            buf[2 * w] = (uint8_t)v;
            buf[2 * w + 1] = (uint8_t)(v >> 8);
        }
        r->total = r->stored = 2u * (uint32_t)w;
        r->status = IN8(GD_STATUS);
        r->rc = GD_OK;
    }
done:
    r->irr = IN8(GD_IRQ_REASON);
    r->secnr = IN8(GD_SECNR);       /* passive read (R137 b) */
    r->has_secnr = 1;
    r->us = now_us() - t0;
    bus_unlock();
    return r->rc;
}

static uint64_t op_now(void *ctx) { (void)ctx; return now_us(); }
static void op_sleep(void *ctx, uint32_t ms) { (void)ctx; thd_sleep((int)ms); }
static void op_progress(void *ctx, const char *m) { (void)ctx; printf("[gdprobe] %s\n", m); }

void gdraw_get_ops(gd_ops *ops)
{
    ops->ctx = NULL;
    ops->packet = gd_packet;
    ops->identify = gd_identify;
    ops->now_us = op_now;
    ops->sleep_ms = op_sleep;
    ops->progress = op_progress;
}
