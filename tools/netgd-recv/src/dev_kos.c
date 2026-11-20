/* KOS build: ngdev on top of KOS g1ata (the secondary IDE disk = the SD card).
   KOS g1ata has no timeouts (design 2.3), so a watchdog thread aborts after 60 s without
   progress (design 8, "firmware does not answer"). */
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <kos.h>
#include <dc/g1ata.h>
#include "ngdev.h"

int ngdev_use_dma;
static int inited;
static int dma_proven;                   /* a DMA command has succeeded: never fall back after that */
static uint32_t nsectors;
static volatile uint64_t busy_since;     /* 0 = idle; a 64-bit value: written and read with IRQs off */
static uint8_t *bounce;                  /* 32-byte aligned, 256 sectors */
static int watch_started;

static void set_busy(uint64_t v)
{
    irq_mask_t m = irq_disable();
    busy_since = v;
    irq_restore(m);
}

static void *watchdog(void *arg)
{
    (void)arg;
    for (;;) {
        uint64_t b;
        irq_mask_t m;
        thd_sleep(1000);
        m = irq_disable();               /* the SH-4 has no atomic 64-bit load: take a consistent copy */
        b = busy_since;
        irq_restore(m);
        if (b && timer_ms_gettime64() - b > 60000) {
            dbglog(DBG_ERROR, "netgd-recv: G1 ATA call stuck for 60 s, aborting\n");
            /* Ending the write session from here is not possible: the stuck call holds the G1 mutex
               and the bus.  The session (and the ejected disc) is left to the next run / a reset. */
            arch_abort();
        }
    }
    return NULL;
}

static void start_watch(void)
{
    if (watch_started) return;
    watch_started = 1;
    thd_create(1, watchdog, NULL);
}

void ngdev_watch_begin(void) { start_watch(); set_busy(timer_ms_gettime64()); }
void ngdev_watch_end(void) { set_busy(0); }

int ngdev_init(void)
{
    kos_blockdev_t bd;
    uint64_t cnt;
    if (inited) return 0;
    if (g1_ata_init() != 0) return -1;
    if (g1_ata_blockdev_for_device(0, &bd) != 0) return -1;
    cnt = bd.count_blocks(&bd);
    free(bd.dev_data);
    /* KOS g1ata: block_count = max_lba + 1, but IDENTIFY's "max_lba" (words 60..61 / 100..103) is already
       the NUMBER of user sectors, and g1ata itself refuses sector + count > max_lba: one sector too many. */
    nsectors = cnt ? (uint32_t)(cnt - 1) : 0;
    bounce = (uint8_t *)memalign(32, 256 * 512);
    if (!bounce) return -1;
    start_watch();
    inited = 1;
    return 0;
}

uint32_t ngdev_sectors(void) { return nsectors; }

/* One transfer.  DMA that fails before any DMA command has ever worked (the first reads at mount
   time) is repeated once as PIO; if that works, DMA is switched off for the rest of the run. */
int ngdev_read(uint32_t lba, uint32_t n, void *buf)
{
    int r;
    uint8_t *use = (uint8_t *)buf;
    if (!inited || n > 256) return -1;
    if (ngdev_use_dma && ((uintptr_t)buf & 31)) use = bounce;
    set_busy(timer_ms_gettime64());
    r = ngdev_use_dma ? g1_ata_read_lba_dma(lba, n, use, 1) : g1_ata_read_lba(lba, n, use);
    set_busy(0);
    if (r != 0 && ngdev_use_dma && !dma_proven) {
        set_busy(timer_ms_gettime64());
        r = g1_ata_read_lba(lba, n, use);
        set_busy(0);
        if (r == 0) {
            ngdev_use_dma = 0;
            dbglog(DBG_WARNING, "netgd-recv: DMA read failed at mount time, PIO from now on\n");
        }
    } else if (r == 0 && ngdev_use_dma) {
        dma_proven = 1;
    }
    if (r != 0) return -1;
    if (use != buf) memcpy(buf, use, (size_t)n * 512);
    return 0;
}

int ngdev_write(uint32_t lba, uint32_t n, const void *buf)
{
    int r;
    const uint8_t *use = (const uint8_t *)buf;
    if (!inited || n > 256) return -1;
    if (ngdev_use_dma && ((uintptr_t)buf & 31)) { memcpy(bounce, buf, (size_t)n * 512); use = bounce; }
    set_busy(timer_ms_gettime64());
    r = ngdev_use_dma ? g1_ata_write_lba_dma(lba, n, use, 1) : g1_ata_write_lba(lba, n, use);
    set_busy(0);
    if (r != 0 && ngdev_use_dma && !dma_proven) {
        set_busy(timer_ms_gettime64());
        r = g1_ata_write_lba(lba, n, use);
        set_busy(0);
        if (r == 0) {
            ngdev_use_dma = 0;
            dbglog(DBG_WARNING, "netgd-recv: DMA write failed before any DMA worked, PIO from now on\n");
        }
    } else if (r == 0 && ngdev_use_dma) {
        dma_proven = 1;
    }
    return r == 0 ? 0 : -1;
}

int ngdev_flush(void)
{
    int r;
    if (!inited) return 0;
    set_busy(timer_ms_gettime64());
    r = g1_ata_flush();
    set_busy(0);
    return r == 0 ? 0 : -1;
}
