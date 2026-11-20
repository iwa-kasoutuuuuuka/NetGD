/* Raw block access to the secondary IDE disk (= the SD card), implemented by
   dev_kos.c (KOS g1ata) or test/dev_file.c (an image file).  FatFs does not call
   this directly: diskio.c goes through ngblk.c, which adds the FNV-1a check of
   every write (design 4.5) and the retries (8). */
#ifndef NG_DEV_H
#define NG_DEV_H
#include <stdint.h>

extern int ngdev_use_dma;                   /* set from the firmware's FW_CAP_DMA before the first write */
int ngdev_init(void);                       /* 0 ok */
uint32_t ngdev_sectors(void);               /* capacity in 512-byte sectors */
int ngdev_read(uint32_t lba, uint32_t n, void *buf);
int ngdev_write(uint32_t lba, uint32_t n, const void *buf);
int ngdev_flush(void);                      /* 0 ok */

/* ngblk.c */
#define NGBLK_MAX_RETRY 3
typedef struct {
    uint32_t writes, sectors_written;
    uint32_t io_retries;        /* a command failed and was repeated */
    uint32_t fnv_mismatches;    /* F8 check failed and the LBA was rewritten */
    uint32_t unverified;        /* write commands that had no firmware check */
} ngblk_stats_t;
extern ngblk_stats_t ngblk_stats;
extern int ngblk_check_fnv;     /* 1 when the firmware offers FW_CAP_CHECKSUM */

int ngblk_read(uint32_t lba, uint32_t n, void *buf);
int ngblk_write(uint32_t lba, uint32_t n, const void *buf);     /* 0 ok, -1 EIO, -2 EBUS */

#endif
