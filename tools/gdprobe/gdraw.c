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
 * only register writes the ones below: FEATURES, byte count, COMMAND, DATA. (COMMAND takes 0xA0 PACKET,
 * 0xA1 IDENTIFY PACKET when identify=on, and 0x08 DEVICE RESET only when swap_devreset=on.)
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

/* seq 25: the G1 DMA registers, as g1ata.c / cdrom.c use them (kernel/arch/dreamcast/hardware/g1ata.c:104-118) */
#define G1_DMA_ADDRESS   0xA05F7404u
#define G1_DMA_LENGTH    0xA05F7408u
#define G1_DMA_DIRECTION 0xA05F740Cu   /* 1 = to memory */
#define G1_DMA_ENABLE    0xA05F7414u
#define G1_DMA_STATUS    0xA05F7418u   /* write 1 = start, 0 = stop; reads 1 while a transfer runs (g1_dma_in_progress) */
#define G1_DMA_PROT      0xA05F74B8u   /* DMA address-range protection (P2 view of 0x005F74B8) */
#define G1_DMA_UNLOCK_SYSMEM 0x8843407Fu /* system RAM only (cdrom.c unlocks ALL memory: not needed here) */
#define ASIC_IRQ_NRM     0xA05F6900u   /* bit 14 = GD-ROM DMA end, ASIC_EVT_GD_DMA = 0x000e */
#define ASIC_IRQ_ERR     0xA05F6908u   /* bit 13 = GD DMA overrun (0x020d), bit 12 = illegal address (0x020c) */
#define ASIC_GD_DMA_END  (1u << 14)
#define ASIC_GD_OVERRUN  (1u << 13)
#define ASIC_GD_ILLADDR  (1u << 12)
#define DMA_GRACE_US     200000u       /* after one side (drive / DMA engine) finished: how long the other may take */

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
#define OUT32(a, v) (*(volatile uint32_t *)(a) = (v))
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

/* Passive register snapshot (R148 SW_REGPOLL). Only reads: ALTSTATUS (unlike STATUS it does not clear the
 * interrupt), ERROR, interrupt reason, SECNR and the byte-count pair. No register is written. */
static void read_regs(gd_regs *g)
{
    g->altst = IN8(GD_ALTSTATUS);
    g->err = IN8(GD_ERROR);
    g->irr = IN8(GD_IRQ_REASON);
    g->secnr = IN8(GD_SECNR);
    g->bcl = IN8(GD_LBA_MID);
    g->bch = IN8(GD_LBA_HIGH);
}

static int gd_regs_op(void *ctx, gd_regs *g)
{
    (void)ctx;
    memset(g, 0, sizeof(*g));
    if (bus_lock())
        return 1;
    read_regs(g);
    bus_unlock();
    return 0;
}

/* ATA DEVICE RESET (0x08), only when the config enabled swap_devreset (probes.c checks the ATA whitelist
 * first). The one register write is COMMAND = 0x08, the same register every PACKET writes; DEVICE_SELECT is
 * still never touched. The command does not raise an interrupt (ATA-4 8.7); we only wait, bounded by
 * timeout_ms, for BSY to clear and then read the registers (byte count 0x14/0xEB = the ATAPI signature,
 * SECNR = what the GD-ROM shows right after a reset). The bus is held for at least RESET_MIN_US (2 ms). */
#define RESET_MIN_US 2000u

static int gd_ata_reset(void *ctx, uint32_t timeout_ms, gd_regs *after, uint32_t *us, int *bsy_seen)
{
    uint64_t t0, deadline;
    int rc = GD_OK;
    (void)ctx;
    memset(after, 0, sizeof(*after));
    *us = 0;
    *bsy_seen = 0;
    if (bus_lock())
        return GD_BUSY;
    t0 = now_us();
    deadline = t0 + (uint64_t)timeout_ms * 1000u;
    OUT8(GD_COMMAND, 0x08);
    settle();
    /* A drive whose firmware (not its bus logic) takes the command may raise BSY a little late: keep
     * watching for at least RESET_MIN_US before trusting a clear BSY. The register poll that follows
     * catches anything later. */
    for (;;) {
        uint8_t s = IN8(GD_ALTSTATUS);
        uint64_t t = now_us();
        if (s & SR_BSY)
            *bsy_seen = 1;
        else if (t - t0 >= RESET_MIN_US)
            break;
        if (t >= deadline) {
            rc = GD_TIMEOUT;
            break;
        }
    }
    *us = (uint32_t)(now_us() - t0);
    read_regs(after);
    bus_unlock();
    return rc;
}

/* seq 25: CD_READ (and nothing else) with the DMA feature bit, data to RAM through the G1 DMA engine (the way the BIOS
 * and cdrom.c do it, but with raw registers so the drive's own behaviour can be observed). Same bus lock and the same
 * register writes as gd_packet, plus the DMA registers:
 *   FEATURES = 1 (DMA); the DMA address / length / direction (to memory) / enable are programmed before PACKET and the
 *   transfer is started (G1_DMA_STATUS = 1) right after the CDB was written (g1ata.c order: registers, command, start).
 * Every wait is bounded by timeout_ms (and a 200 ms grace after one side finished). On every exit the DMA engine is
 * stopped (STATUS = 0, ENABLE = 0), the data cache over the buffer is invalidated, the ASIC DMA bits are acknowledged
 * and a DRQ left pending by the drive (it fell back to PIO) is drained, so the drive is idle again. The buffer must
 * be 32-byte aligned cached RAM; the length is whatever the runner asks for (its `exact` mode is deliberately not a
 * multiple of 32 when the sector bytes are not). */
static int gd_packet_dma(void *ctx, const uint8_t cdb[12], uint8_t *buf, uint32_t buflen, uint32_t dma_len,
                         uint32_t timeout_ms, gd_result *r, gd_dma *d)
{
    uint64_t t0, deadline, t_dma = 0, t_idle = 0, t_irq = 0;
    uint8_t st = 0;
    uint32_t total_drain = 0;
    int i, gd_irq = 0, dma_idle = 0, finished = 0;
    (void)ctx;
    memset(r, 0, sizeof(*r));
    memset(d, 0, sizeof(*d));
    d->len_left = 0xFFFFFFFFu;
    d->req_len = dma_len;
    /* defence in depth: only CD_READ, only into an aligned cached-RAM buffer that holds the length */
    if (cdb[0] != 0x30 || ((uintptr_t)buf & 31u) || (buflen & 31u) || dma_len == 0 || dma_len > buflen ||
        ((uintptr_t)buf & 0xE0000000u) == 0xA0000000u) {
        r->rc = GD_REFUSED;
        return r->rc;
    }
    if (bus_lock()) {
        r->rc = GD_BUSY;
        return r->rc;
    }
    t0 = now_us();
    deadline = t0 + (uint64_t)timeout_ms * 1000u;

    if (!wait_clear(SR_BSY | SR_DRQ, deadline) || (IN32(G1_DMA_STATUS) & 1u)) {
        r->rc = GD_BUSY;
        r->status = IN8(GD_ALTSTATUS);
        goto done;
    }
    (void)IN32(GD_STATUS);          /* clear a stale interrupt */
    OUT32(ASIC_IRQ_NRM, ASIC_GD_DMA_END);                   /* clear a stale DMA-end bit (write 1 = clear) */
    OUT32(ASIC_IRQ_ERR, ASIC_GD_OVERRUN | ASIC_GD_ILLADDR);
    OUT32(G1_DMA_PROT, G1_DMA_UNLOCK_SYSMEM);
    d->prot_ok = (IN32(G1_DMA_PROT) == G1_DMA_UNLOCK_SYSMEM);
    /* the runner filled the buffer with its pattern through the cache: write it back, then drop the lines so the
     * CPU re-reads what the DMA wrote */
    dcache_flush_range((uintptr_t)buf, buflen);
    dcache_inval_range((uintptr_t)buf, buflen);
    OUT32(G1_DMA_ADDRESS, (uint32_t)((uintptr_t)buf & 0x1FFFFFFFu));
    OUT32(G1_DMA_LENGTH, dma_len);
    OUT32(G1_DMA_DIRECTION, 1);
    OUT32(G1_DMA_ENABLE, 1);

    OUT8(GD_FEATURES, 1);           /* DMA */
    OUT8(GD_LBA_MID, BYTE_LIMIT & 0xFF);
    OUT8(GD_LBA_HIGH, BYTE_LIMIT >> 8);
    OUT8(GD_COMMAND, ATAPI_PACKET);
    settle();
    if (!wait_pkt_drq(deadline, &r->cdbst)) {
        r->rc = GD_TIMEOUT;
        r->status = IN8(GD_ALTSTATUS);
        goto done;
    }
    r->has_cdbst = 1;
    (void)IN32(GD_STATUS);
    for (i = 0; i < 6; i++)
        OUT16(GD_DATA, (uint16_t)(cdb[2 * i] | (cdb[2 * i + 1] << 8)));
    t_dma = now_us();
    OUT32(G1_DMA_STATUS, 1);        /* start */

    /* wait for the DMA engine to go idle and for the drive to complete (interrupt, BSY clear) */
    for (;;) {
        uint64_t t = now_us();
        if (!dma_idle && !(IN32(G1_DMA_STATUS) & 1u)) {
            dma_idle = 1;
            t_idle = t;
            d->done = 1;
            d->us_dma = (uint32_t)(t - t_dma);
            if (!d->first) d->first = 'd';
        }
        if (!gd_irq && (IN32(GD_EXT_INT_STAT) & 1u)) {
            gd_irq = 1;
            t_irq = t;
            if (!d->first) d->first = 'g';
        }
        if (dma_idle && gd_irq && !(IN8(GD_ALTSTATUS) & SR_BSY)) { finished = 1; break; }
        if (dma_idle && t - t_idle >= DMA_GRACE_US && !(IN8(GD_ALTSTATUS) & SR_BSY)) { finished = 1; break; } /* no irq */
        if (gd_irq && t - t_irq >= DMA_GRACE_US && !(IN8(GD_ALTSTATUS) & SR_BSY)) { finished = 1; break; }   /* DMA never idle */
        if (t >= deadline)
            break;
    }
    if (!dma_idle)
        d->aborted = 1;             /* the engine is still waiting for data (or never got any): stopped below */
done:
    /* always: stop the engine, then look at what it left */
    OUT32(G1_DMA_STATUS, 0);
    d->dma_st_end = (uint8_t)IN32(G1_DMA_STATUS);
    d->len_left = IN32(G1_DMA_LENGTH);
    d->addr_after = IN32(G1_DMA_ADDRESS);
    OUT32(G1_DMA_ENABLE, 0);
    {
        uint32_t nrm = IN32(ASIC_IRQ_NRM), er = IN32(ASIC_IRQ_ERR);
        d->asic_nrm = (uint8_t)((nrm & ASIC_GD_DMA_END) ? 1 : 0);
        d->asic_err = (uint8_t)(((er & ASIC_GD_OVERRUN) ? 1 : 0) | ((er & ASIC_GD_ILLADDR) ? 2 : 0));
        OUT32(ASIC_IRQ_NRM, ASIC_GD_DMA_END);
        OUT32(ASIC_IRQ_ERR, ASIC_GD_OVERRUN | ASIC_GD_ILLADDR);
    }
    dcache_inval_range((uintptr_t)buf, buflen);
    if (r->rc == GD_OK && finished) {
        st = IN8(GD_STATUS);        /* also clears the interrupt */
        r->status = st;
        if (st & SR_ERR) {
            r->error = IN8(GD_ERROR);
            r->rc = GD_CHECK;
        } else if (st & SR_DRQ) {
            /* the drive wants PIO after all: drain it (bounded) so that it is idle again */
            uint32_t n = IN8(GD_LBA_MID) | ((uint32_t)IN8(GD_LBA_HIGH) << 8), w;
            d->drq = 1;
            d->drq_bc = (uint16_t)n;
            while ((st & SR_DRQ) && n && total_drain < MAX_TOTAL) {
                for (w = 0; w < (n + 1) / 2; w++) {
                    uint16_t v = IN16(GD_DATA);
                    uint32_t off = total_drain + 2 * w;
                    if (off + 1 < sizeof(d->tail)) {
                        d->tail[off] = (uint8_t)v;
                        d->tail[off + 1] = (uint8_t)(v >> 8);
                        d->tail_n = (uint8_t)(off + 2);
                    }
                }
                total_drain += n;
                if (!wait_irq(now_us() + 200000u) || !wait_clear(SR_BSY, now_us() + 200000u))
                    break;
                st = IN8(GD_STATUS);
                n = IN8(GD_LBA_MID) | ((uint32_t)IN8(GD_LBA_HIGH) << 8);
            }
            d->drain = total_drain;
            d->st_after = IN8(GD_ALTSTATUS);
            r->status = st;
            /* the DMA itself finished: the bytes left in the drive are an observation, not a failure */
            r->rc = dma_idle ? GD_OK : GD_PROTO;
        } else if (!dma_idle) {
            r->rc = GD_PROTO;       /* the drive completed without error but the DMA never ran / finished */
        }
    } else if (r->rc == GD_OK) {
        /* no completion before the deadline: a last bounded chance for the drive to go idle, nothing else */
        r->rc = GD_TIMEOUT;
        r->status = IN8(GD_ALTSTATUS);
        (void)wait_clear(SR_BSY, now_us() + 200000u);
        (void)IN32(GD_STATUS);
    }
    d->bcl = IN8(GD_LBA_MID);
    d->bch = IN8(GD_LBA_HIGH);
    r->irr = IN8(GD_IRQ_REASON);
    r->secnr = IN8(GD_SECNR);
    r->has_secnr = 1;
    r->us = now_us() - t0;
    OUT8(GD_FEATURES, 0);           /* leave the feature register in PIO mode */
    bus_unlock();
    return r->rc;
}

/* Busy-wait on the CPU timer only: no bus access. */
static void op_spin(void *ctx, uint32_t us)
{
    uint64_t end = now_us() + us;
    (void)ctx;
    while (now_us() < end)
        ;
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
    ops->regs = gd_regs_op;
    ops->ata_reset = gd_ata_reset;
    ops->spin_us = op_spin;
    ops->packet_dma = gd_packet_dma;
}
