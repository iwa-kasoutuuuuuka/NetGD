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
static uint32_t nsectors;
static volatile uint64_t busy_since;     /* 0 = idle */
static uint8_t *bounce;                  /* 32-byte aligned, 256 sectors */

static void *watchdog(void *arg)
{
    (void)arg;
    for (;;) {
        thd_sleep(1000);
        if (busy_since && timer_ms_gettime64() - busy_since > 60000) {
            dbglog(DBG_ERROR, "netgd-recv: G1 ATA call stuck for 60 s, aborting\n");
            arch_abort();
        }
    }
    return NULL;
}

int ngdev_init(void)
{
    kos_blockdev_t bd;
    if (inited) return 0;
    if (g1_ata_init() != 0) return -1;
    if (g1_ata_blockdev_for_device(0, &bd) != 0) return -1;
    nsectors = (uint32_t)bd.count_blocks(&bd);
    free(bd.dev_data);
    bounce = (uint8_t *)memalign(32, 256 * 512);
    if (!bounce) return -1;
    thd_create(1, watchdog, NULL);
    inited = 1;
    return 0;
}

uint32_t ngdev_sectors(void) { return nsectors; }

int ngdev_read(uint32_t lba, uint32_t n, void *buf)
{
    int r;
    uint8_t *use = (uint8_t *)buf;
    if (!inited || n > 256) return -1;
    if (ngdev_use_dma && ((uintptr_t)buf & 31)) use = bounce;
    busy_since = timer_ms_gettime64();
    r = ngdev_use_dma ? g1_ata_read_lba_dma(lba, n, use, 1) : g1_ata_read_lba(lba, n, use);
    busy_since = 0;
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
    busy_since = timer_ms_gettime64();
    r = ngdev_use_dma ? g1_ata_write_lba_dma(lba, n, use, 1) : g1_ata_write_lba(lba, n, use);
    busy_since = 0;
    return r == 0 ? 0 : -1;
}

int ngdev_flush(void)
{
    int r;
    if (!inited) return 0;
    busy_since = timer_ms_gettime64();
    r = g1_ata_flush();
    busy_since = 0;
    return r == 0 ? 0 : -1;
}
