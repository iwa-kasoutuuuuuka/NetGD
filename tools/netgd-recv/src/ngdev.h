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
/* KOS build: account a non-g1ata G1 call (the raw ATAPI of fwxfer_kos.c) in the 60 s watchdog */
void ngdev_watch_begin(void);
void ngdev_watch_end(void);

/* ngblk.c */
#define NGBLK_MAX_RETRY 3
#define NGBLK_READ_TRIES 4      /* single-sector reads: attempts to get two identical reads */
typedef struct {
    uint32_t writes, sectors_written;
    uint32_t io_retries;        /* a command failed and was repeated */
    uint32_t fnv_mismatches;    /* F8 check failed and the LBA was rewritten */
    uint32_t unverified;        /* write commands that had no firmware check */
    uint32_t read_mismatches;   /* a double read (one sector, or one ngblk_read_twice chunk) did not agree */
} ngblk_stats_t;
extern ngblk_stats_t ngblk_stats;
extern int ngblk_check_fnv;     /* 1 when the firmware offers FW_CAP_CHECKSUM */

/* n == 1 (FatFs window: FAT, directory, FSInfo, and every sector a commit rewrites): read twice and
   compare, repeat up to NGBLK_READ_TRIES times, -1 if never two identical reads (G1 read bit flips,
   design 8).  n > 1 (bulk data, checked by the readback CRC): one read. */
int ngblk_read(uint32_t lba, uint32_t n, void *buf);
/* n <= 256 sectors of metadata in ONE command, read twice into the same buffer: the CRC-32 of the two
   reads must agree (the mount-time FAT scan, ngfs.c).  Same tries and result as the n == 1 case above.
   CRC-32 instead of a second buffer + memcmp: two reads that differ in one bit, or in one burst of up
   to 32 bits, always have different CRC-32s; any other difference slips through with probability 2^-32. */
int ngblk_read_twice(uint32_t lba, uint32_t n, void *buf);
int ngblk_write(uint32_t lba, uint32_t n, const void *buf);     /* 0 ok, -1 EIO, -2 EBUS */

#endif
