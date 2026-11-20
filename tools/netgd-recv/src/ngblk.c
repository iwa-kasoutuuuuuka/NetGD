/* Verified block writes (and double-read single sectors, see ngblk_read): design 4.5 (F8) and 8 ("G1 bit flips", "SD write error").
   After every ATA write command the firmware's FNV-1a of the bytes it received is
   compared to ours; a mismatch rewrites the same LBAs (up to 3 times).  FAT and
   directory sectors go through here too: FatFs reaches the disk only via diskio.c. */
#include <string.h>
#include "ngdev.h"
#include "fwctl.h"
#include "crc32.h"

ngblk_stats_t ngblk_stats;
int ngblk_check_fnv;

int ngblk_read(uint32_t lba, uint32_t n, void *buf)
{
    static uint8_t again[512] __attribute__((aligned(32)));
    if (n == 1) {
        for (int attempt = 0; attempt < NGBLK_READ_TRIES; attempt++) {
            if (ngdev_read(lba, 1, buf) != 0 || ngdev_read(lba, 1, again) != 0) { ngblk_stats.io_retries++; continue; }
            if (!memcmp(buf, again, 512)) return 0;
            ngblk_stats.read_mismatches++;
        }
        return -1;
    }
    for (int attempt = 0; attempt < NGBLK_MAX_RETRY; attempt++) {
        if (ngdev_read(lba, n, buf) == 0) return 0;
        ngblk_stats.io_retries++;
    }
    return -1;
}

int ngblk_read_twice(uint32_t lba, uint32_t n, void *buf)
{
    for (int attempt = 0; attempt < NGBLK_READ_TRIES; attempt++) {
        uint32_t first;
        if (ngdev_read(lba, n, buf) != 0) { ngblk_stats.io_retries++; continue; }
        first = ng_crc32(0, buf, (size_t)n * 512);
        if (ngdev_read(lba, n, buf) != 0) { ngblk_stats.io_retries++; continue; }
        if (ng_crc32(0, buf, (size_t)n * 512) == first) return 0;
        ngblk_stats.read_mismatches++;
    }
    return -1;
}

/* One ATA write command, verified.  Never more than NGBLK_WRITE_MAX sectors: KOS may split a larger PIO
   write (e.g. 255 + 1), and the firmware STATUS only describes the LAST command, so the FNV compare
   would then fail for ever (R89 C-A). */
static int write_cmd(uint32_t lba, uint32_t n, const void *buf)
{
    uint32_t fnv = 0;
    int bus_error = 0;

    if (ngblk_check_fnv) fnv = ng_fnv1a(NG_FNV_INIT, buf, (size_t)n * 512);
    for (int attempt = 0; attempt < NGBLK_MAX_RETRY; attempt++) {
        ng_fwstatus_t st;
        if (ngdev_write(lba, n, buf) != 0) {
            ngblk_stats.io_retries++;
            bus_error = 0;
            continue;
        }
        ngblk_stats.writes++;
        ngblk_stats.sectors_written += n;
        if (!ngblk_check_fnv) {
            ngblk_stats.unverified++;
            return 0;
        }
        if (fw_status(&st) == 0 && (st.flags & FW_ST_LASTOK) && st.last_lba == lba && st.last_count == n &&
            st.last_fnv == fnv)
            return 0;
        ngblk_stats.fnv_mismatches++;
        bus_error = 1;
    }
    return bus_error ? -2 : -1;
}

int ngblk_write(uint32_t lba, uint32_t n, const void *buf)
{
    while (n) {
        uint32_t c = n > NGBLK_WRITE_MAX ? NGBLK_WRITE_MAX : n;
        int r = write_cmd(lba, c, buf);
        if (r) return r;
        lba += c; n -= c; buf = (const uint8_t *)buf + (size_t)c * 512;
    }
    return 0;
}
