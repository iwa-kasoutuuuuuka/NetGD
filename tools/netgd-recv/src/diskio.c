/* FatFs diskio on top of ngblk.c (verified writes) -> ngdev (KOS g1ata or a test image file). */
#include <stddef.h>
#include "ff.h"
#include "diskio.h"
#include "ngdev.h"

int ng_diskio_bus_error;
static int inited;

DSTATUS disk_status(BYTE pdrv) { return (pdrv == 0 && inited) ? 0 : STA_NOINIT; }

DSTATUS disk_initialize(BYTE pdrv)
{
    if (pdrv != 0) return STA_NOINIT;
    if (!inited && ngdev_init() == 0) inited = 1;
    return inited ? 0 : STA_NOINIT;
}

DRESULT disk_read(BYTE pdrv, BYTE *buff, LBA_t sector, UINT count)
{
    if (pdrv != 0 || !inited) return RES_NOTRDY;
    while (count) {
        UINT c = count > 256 ? 256 : count;
        if (ngblk_read((uint32_t)sector, c, buff)) return RES_ERROR;
        sector += c; buff += (size_t)c * 512; count -= c;
    }
    return RES_OK;
}

DRESULT disk_write(BYTE pdrv, const BYTE *buff, LBA_t sector, UINT count)
{
    if (pdrv != 0 || !inited) return RES_NOTRDY;
    while (count) {
        UINT c = count > 256 ? 256 : count;
        int r = ngblk_write((uint32_t)sector, c, buff);
        if (r) { ng_diskio_bus_error = (r == -2); return RES_ERROR; }
        sector += c; buff += (size_t)c * 512; count -= c;
    }
    return RES_OK;
}

DRESULT disk_ioctl(BYTE pdrv, BYTE cmd, void *buff)
{
    if (pdrv != 0 || !inited) return RES_NOTRDY;
    switch (cmd) {
    case CTRL_SYNC: return ngdev_flush() == 0 ? RES_OK : RES_ERROR;
    case GET_SECTOR_COUNT: *(LBA_t *)buff = ngdev_sectors(); return RES_OK;
    case GET_BLOCK_SIZE: *(DWORD *)buff = 128; return RES_OK;
    default: return RES_PARERR;
    }
}
