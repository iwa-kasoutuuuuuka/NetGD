/* netgd-fwupd: raw G1 ATA access to the secondary device (KOS 2.3.0, fork clone checked):
 *   _g1_ata_sem            kernel/arch/dreamcast/hardware/cdrom.c, the semaphore g1ata.c and the
 *                          cdrom driver serialise the G1 bus on (as tools/gdprobe/gdraw.c does)
 *   register addresses     g1ata.c:83-98 (ALTSTATUS 7018, DATA 7080, ERROR/FEATURES 7084,
 *                          SECTOR_COUNT 7088, LBA_LOW/MID/HIGH 708C/7090/7094, DEVICE_SELECT 7098,
 *                          STATUS/COMMAND 709C, PIO access waits 7490/7494)
 *   device select 0xF0     G1_ATA_SLAVE | G1_ATA_LBA_MODE (g1ata.c:1019 uses 0xF0 for the scan)
 *   LBA48 write order      g1ata.c g1_ata_set_sector_and_count: bits 24-47 first, then bits 0-23.
 *                          The FPGA's LBA registers are a sliding window (ide_device.py:642-663):
 *                          the second write of each pair becomes the low byte, the first the
 *                          "previous" byte the firmware reads back as bits 24-47 (ide.rs lba48()).
 *   timer_us_gettime64     include/kos/timer.h
 *   thd_sleep              include/kos/thread.h
 * g1_ata_mutex_unlock is NOT used: it writes DEVICE_SELECT (g1ata.c:216-222). Here the semaphore is
 * taken directly and DEVICE_SELECT is written explicitly, to the secondary, on every command.
 * g1_ata_init is NOT called: its scan resets nothing itself but also installs DMA handlers we do
 * not want; the two things it does that PIO needs (SET FEATURES "PIO default", PIO wait values)
 * are done in g1dev_prepare(), after the device was recognised as NetGD. */
#include <kos.h>
#include <kos/sem.h>
#include <kos/timer.h>
#include <string.h>
#include "g1dev.h"

#define G1_ALTSTATUS     0xA05F7018u /* read */
#define G1_DATA          0xA05F7080u
#define G1_ERROR         0xA05F7084u /* read */
#define G1_FEATURES      0xA05F7084u /* write */
#define G1_SECTOR_COUNT  0xA05F7088u /* write */
#define G1_LBA_LOW       0xA05F708Cu
#define G1_LBA_MID       0xA05F7090u
#define G1_LBA_HIGH      0xA05F7094u
#define G1_DEVICE_SELECT 0xA05F7098u
#define G1_STATUS        0xA05F709Cu /* read; reading clears the interrupt */
#define G1_COMMAND       0xA05F709Cu /* write */
#define G1_PIO_RWAIT     0xA05F7490u
#define G1_PIO_WWAIT     0xA05F7494u

#define DEV_SECONDARY    0xF0        /* slave, LBA mode */
#define PIO_DEFAULT_WAIT 0x00000222u /* G1_ACCESS_PIO_DEFAULT */

#define SR_ERR 0x01
#define SR_DRQ 0x08
#define SR_BSY 0x80

#define ATA_IDENTIFY     0xEC
#define ATA_SET_FEATURES 0xEF

extern semaphore_t _g1_ata_sem;   /* defined in cdrom.c; g1ata.c declares it the same way */

#define OUT8(a, v)  (*(volatile uint8_t *)(a) = (uint8_t)(v))
#define OUT16(a, v) (*(volatile uint16_t *)(a) = (uint16_t)(v))
#define OUT32(a, v) (*(volatile uint32_t *)(a) = (uint32_t)(v))
#define IN8(a)      (*(volatile uint8_t *)(a))
#define IN16(a)     (*(volatile uint16_t *)(a))

static uint8_t g_orig_dev;
static int g_orig_valid;

static uint64_t now_us(void) { return timer_us_gettime64(); }

static void settle(void)
{
    int i;
    for (i = 0; i < 4; i++) (void)IN8(G1_ALTSTATUS);   /* ~400 ns after a register write */
}

/* Wait until (ALTSTATUS & mask) == 0. */
static int wait_clear(uint8_t mask, uint32_t ms)
{
    uint64_t dl = now_us() + (uint64_t)ms * 1000u;
    for (;;) {
        if (!(IN8(G1_ALTSTATUS) & mask)) return 1;
        if (now_us() >= dl) return 0;
    }
}

/* After a command write or a block: 1 = DRQ up and BSY down, 2 = BSY and DRQ both down (the
 * command ended, with or without ERR), 0 = timeout. */
static int wait_drq_or_end(uint32_t ms)
{
    uint64_t dl = now_us() + (uint64_t)ms * 1000u;
    for (;;) {
        uint8_t s = IN8(G1_ALTSTATUS);
        if (!(s & SR_BSY)) return (s & SR_DRQ) ? 1 : 2;
        if (now_us() >= dl) return 0;
    }
}

static int bus_lock(void) { return sem_wait(&_g1_ata_sem); }
static void bus_unlock(void) { sem_signal(&_g1_ata_sem); }

/* Select the secondary explicitly. ATA says not to touch DEVICE_SELECT while BSY/DRQ is up. */
static int select_secondary(uint32_t ms)
{
    if (!wait_clear(SR_BSY | SR_DRQ, ms)) return 0;
    if (!g_orig_valid) {
        g_orig_dev = IN8(G1_DEVICE_SELECT);
        g_orig_valid = 1;
    }
    OUT8(G1_DEVICE_SELECT, DEV_SECONDARY);
    settle();
    return wait_clear(SR_BSY | SR_DRQ, ms);
}

void g1dev_deselect(void)
{
    if (!g_orig_valid) return;
    if (bus_lock()) return;
    if (wait_clear(SR_BSY | SR_DRQ, 200)) OUT8(G1_DEVICE_SELECT, g_orig_dev);
    bus_unlock();
}

static void write_key48(uint64_t key)
{
    /* bits 24..47 first, then bits 0..23 (see the header comment) */
    OUT8(G1_LBA_LOW, key >> 24);
    OUT8(G1_LBA_MID, key >> 32);
    OUT8(G1_LBA_HIGH, key >> 40);
    OUT8(G1_LBA_LOW, key);
    OUT8(G1_LBA_MID, key >> 8);
    OUT8(G1_LBA_HIGH, key >> 16);
}

static int g1_f9(void *ctx, uint8_t feat, uint8_t count, int use_key, uint64_t key,
                 const uint8_t *data, uint32_t nblocks, uint32_t timeout_ms, fw_reply *r)
{
    uint32_t b;
    int w, ph;
    (void)ctx;
    memset(r, 0, sizeof(*r));
    if (bus_lock()) { r->rc = FWIO_BUSY; return r->rc; }

    if (!select_secondary(timeout_ms)) { r->rc = FWIO_BUSY; r->status = IN8(G1_ALTSTATUS); goto done; }
    (void)IN8(G1_STATUS);                        /* a stale interrupt */
    if (use_key) write_key48(key);
    OUT8(G1_FEATURES, feat);
    OUT8(G1_SECTOR_COUNT, count);
    OUT8(G1_COMMAND, FW_CMD);
    settle();                                    /* the FPGA sets BSY and clears ERR/DRQ on the write */

    for (b = 0; b < nblocks; b++) {
        ph = wait_drq_or_end(timeout_ms);
        if (ph == 0) { r->rc = FWIO_TIMEOUT; r->status = IN8(G1_ALTSTATUS); goto done; }
        if (ph == 2) break;                      /* ended early: the reply below says why */
        for (w = 0; w < 256; w++) {
            const uint8_t *p = data + (size_t)b * FW_BLOCK + 2 * (size_t)w;
            OUT16(G1_DATA, (uint16_t)(p[0] | ((uint16_t)p[1] << 8)));
        }
        r->blocks++;
    }
    /* completion: BSY and DRQ both down (the updater posts IDLE, or IDLE|ERR). Straight after
     * the last block DRQ may still read high for a moment: that is "not done yet". */
    if (!wait_clear(SR_BSY | SR_DRQ, timeout_ms)) {
        r->rc = FWIO_TIMEOUT;
        r->status = IN8(G1_ALTSTATUS);
        goto done;
    }
    r->status = IN8(G1_STATUS);                  /* also clears INTRQ */
    r->error = IN8(G1_ERROR);
    r->rc = FWIO_OK;
    if (r->blocks < nblocks && !(r->status & SR_ERR))
        r->rc = FWIO_PROTO;                      /* finished without taking all the data */
done:
    bus_unlock();
    return r->rc;
}

static int g1_identify(void *ctx, uint8_t *buf, uint32_t timeout_ms, fw_reply *r)
{
    int w, ph;
    (void)ctx;
    memset(r, 0, sizeof(*r));
    if (bus_lock()) { r->rc = FWIO_BUSY; return r->rc; }
    if (!select_secondary(timeout_ms)) { r->rc = FWIO_BUSY; goto done; }
    (void)IN8(G1_STATUS);
    OUT8(G1_SECTOR_COUNT, 0);
    OUT8(G1_LBA_LOW, 0);
    OUT8(G1_LBA_MID, 0);
    OUT8(G1_LBA_HIGH, 0);
    OUT8(G1_COMMAND, ATA_IDENTIFY);
    settle();
    ph = wait_drq_or_end(timeout_ms);
    if (ph == 0) { r->rc = FWIO_TIMEOUT; goto done; }
    if (ph == 1) {
        for (w = 0; w < 256; w++) {
            uint16_t v = IN16(G1_DATA);
            buf[2 * w] = (uint8_t)v;
            buf[2 * w + 1] = (uint8_t)(v >> 8);
        }
        r->blocks = 1;
        if (!wait_clear(SR_BSY | SR_DRQ, timeout_ms)) { r->rc = FWIO_TIMEOUT; goto done; }
    }
    r->status = IN8(G1_STATUS);
    r->error = IN8(G1_ERROR);
    r->rc = FWIO_OK;
    if (ph != 1) r->status |= SR_ERR;            /* no data: not a disk we can read */
done:
    bus_unlock();
    return r->rc;
}

/* What g1_ata_scan does after IDENTIFY (g1ata.c:1062-1065): SET FEATURES "PIO default mode",
 * then the PIO access wait values. Non-fatal if the device refuses. */
static int g1_prepare(void *ctx)
{
    int ok = 0;
    (void)ctx;
    if (bus_lock()) return -1;
    if (select_secondary(1000)) {
        (void)IN8(G1_STATUS);
        OUT8(G1_FEATURES, 0x03);                 /* set transfer mode */
        OUT8(G1_SECTOR_COUNT, 0x00);             /* PIO default */
        OUT8(G1_LBA_LOW, 0);
        OUT8(G1_LBA_MID, 0);
        OUT8(G1_LBA_HIGH, 0);
        OUT8(G1_COMMAND, ATA_SET_FEATURES);
        settle();
        if (wait_clear(SR_BSY, 1000) && !(IN8(G1_STATUS) & SR_ERR)) {
            OUT32(G1_PIO_RWAIT, PIO_DEFAULT_WAIT);
            OUT32(G1_PIO_WWAIT, PIO_DEFAULT_WAIT);
            ok = 1;
        }
    }
    bus_unlock();
    return ok ? 0 : -1;
}

static uint32_t g1_now_ms(void *ctx) { (void)ctx; return (uint32_t)(timer_us_gettime64() / 1000u); }
static void g1_sleep_ms(void *ctx, uint32_t ms) { (void)ctx; thd_sleep((int)ms); }

void g1dev_fill_env(fw_env *env)
{
    env->f9 = g1_f9;
    env->identify = g1_identify;
    env->prepare = g1_prepare;
    env->now_ms = g1_now_ms;
    env->sleep_ms = g1_sleep_ms;
}
