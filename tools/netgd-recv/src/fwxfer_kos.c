/* KOS build: raw ATAPI PACKET to the PRIMARY (GD-ROM) device through the G1 ATA registers,
   under g1_ata_mutex_lock.  The plain ATAPI PIO protocol.  Nothing here has run on hardware.
   Wrapped in the 60 s G1 watchdog accounting of dev_kos.c (ngdev_watch_begin/end). */
#include <stdint.h>
#include <kos.h>
#include <dc/g1ata.h>
#include "fwxfer.h"
#include "ngdev.h"

#define G1_ALTSTATUS    0xA05F7018
#define G1_DATA         0xA05F7080
#define G1_ERROR        0xA05F7084
#define G1_FEATURES     0xA05F7084
#define G1_MID          0xA05F7090
#define G1_HIGH         0xA05F7094
#define G1_STATUS       0xA05F709C
#define G1_COMMAND      0xA05F709C
#define SR_ERR 0x01
#define SR_DRQ 0x08
#define SR_BSY 0x80

#define OUT8(a, v)  (*(volatile uint8_t *)(a) = (v))
#define OUT16(a, v) (*(volatile uint16_t *)(a) = (v))
#define IN8(a)      (*(volatile uint8_t *)(a))
#define IN16(a)     (*(volatile uint16_t *)(a))

uint32_t fw_xfer_limit_ms;
static uint64_t deadline;           /* fw_xfer_limit_ms: the whole command ends by then (0 = no limit) */

/* wait until (status & mask) == want; 0 ok, -1 timeout */
static int wait_for(uint8_t mask, uint8_t want, uint32_t ms)
{
    uint64_t t0 = timer_ms_gettime64();
    while ((IN8(G1_ALTSTATUS) & mask) != want) {
        uint64_t t = timer_ms_gettime64();
        if (t - t0 > ms || (deadline && t > deadline)) return -1;
    }
    return 0;
}

int fw_xfer(const uint8_t pkt[12], void *data, uint32_t alloc, uint32_t *got, uint8_t *err)
{
    int rc = -1;
    uint8_t st;
    if (got) *got = 0;
    if (err) *err = 0;
    deadline = fw_xfer_limit_ms ? timer_ms_gettime64() + fw_xfer_limit_ms : 0;
    if (g1_ata_mutex_lock() != 0) return -1;
    ngdev_watch_begin();
    g1_ata_select_device(G1_ATA_MASTER);
    if (wait_for(SR_BSY | SR_DRQ, 0, 2000)) goto out;
    OUT8(G1_FEATURES, 0);
    OUT8(G1_MID, alloc & 0xFF);
    OUT8(G1_HIGH, (alloc >> 8) & 0xFF);
    OUT8(G1_COMMAND, 0xA0);                                 /* PACKET */
    if (wait_for(SR_BSY | SR_DRQ, SR_DRQ, 2000)) goto out;
    for (int i = 0; i < 6; i++) OUT16(G1_DATA, (uint16_t)(pkt[2 * i] | (pkt[2 * i + 1] << 8)));
    if (wait_for(SR_BSY, 0, 30000)) goto out;               /* a write session START may read LBA 0 from the card */
    st = IN8(G1_STATUS);
    if (st & SR_ERR) { if (err) *err = IN8(G1_ERROR); goto out; }
    if (st & SR_DRQ) {
        uint32_t n = (uint32_t)IN8(G1_MID) | ((uint32_t)IN8(G1_HIGH) << 8);
        uint8_t *p = (uint8_t *)data;
        uint32_t i;
        for (i = 0; i < n; i += 2) {
            uint16_t w = IN16(G1_DATA);
            if (p && i < alloc) p[i] = (uint8_t)w;
            if (p && i + 1 < alloc) p[i + 1] = (uint8_t)(w >> 8);
        }
        if (got) *got = n < alloc ? n : alloc;
        if (wait_for(SR_BSY | SR_DRQ, 0, 5000)) goto out;
        if (IN8(G1_STATUS) & SR_ERR) { if (err) *err = IN8(G1_ERROR); goto out; }
    }
    rc = 0;
out:
    /* No select of the SLAVE here: the firmware may not answer, and the next g1ata call
       (KOS g1_ata_* under the same mutex) selects its own device. */
    ngdev_watch_end();
    g1_ata_mutex_unlock();
    return rc;
}
