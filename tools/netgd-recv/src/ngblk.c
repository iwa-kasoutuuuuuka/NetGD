/* Verified block writes: design 4.5 (F8) and 8 ("G1 bit flips", "SD write error").
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
    for (int attempt = 0; attempt < NGBLK_MAX_RETRY; attempt++) {
        if (ngdev_read(lba, n, buf) == 0) return 0;
        ngblk_stats.io_retries++;
    }
    return -1;
}

int ngblk_write(uint32_t lba, uint32_t n, const void *buf)
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
