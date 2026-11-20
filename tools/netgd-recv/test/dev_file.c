/* ngdev on an image file.  Power loss: after sim_cut_after sector writes, every further
   I/O fails and the remaining sectors of the interrupted command are NOT written
   (a sector is atomic, as an SD block is). */
#include <stdio.h>
#include <string.h>
#include <time.h>
#include "ngdev.h"
#include "crc32.h"
#include "simdisk.h"

int ngdev_use_dma;
int64_t sim_cut_after = -1;
int sim_dead;
uint64_t sim_sector_writes, sim_cmd_writes;
int64_t sim_flip_cmd = -1;
int64_t sim_rflip_cmd = -1;
int sim_rflip_every;
static uint64_t rd1_count;
uint64_t sim_cmd_reads, sim_sector_reads;
int64_t sim_rflipn_cmd = -1;
int sim_rflipn_every;
static uint64_t rdn_count;
uint32_t sim_part_off, sim_protect_lba;
uint64_t sim_violations;
uint32_t sim_last_lba, sim_last_n, sim_last_fnv;
int sim_last_ok;
int sim_unsessioned_writes;
extern int sim_session;

static FILE *img;
static uint32_t nsec;

/* create != 0: make a new, empty image (mkimage).  Otherwise the file must exist: an image that
   cannot be opened is an error, never silently replaced by an empty one.  A few short retries,
   because on Windows a virus scanner may still hold a file that was copied a moment ago. */
int sim_open_ex(const char *path, uint32_t sectors, int create)
{
    for (int i = 0; i < 40 && !img; i++) {
        img = fopen(path, create ? "w+b" : "r+b");
        if (!img) {
            clock_t t = clock();
            while ((double)(clock() - t) / CLOCKS_PER_SEC < 0.05) { }
        }
    }
    nsec = sectors;
    return img ? 0 : -1;
}

int sim_open(const char *path, uint32_t sectors) { return sim_open_ex(path, sectors, 0); }

void sim_close(void)
{
    if (img) fclose(img);
    img = NULL;
}

int ngdev_init(void) { return img ? 0 : -1; }

uint32_t ngdev_sectors(void) { return nsec - sim_part_off; }

int ngdev_read(uint32_t lba, uint32_t n, void *buf)
{
    size_t got;
    if (sim_dead || !img || n > 256 || (uint64_t)lba + sim_part_off + n > nsec) return -1;
    fseek(img, (long)(((uint64_t)lba + sim_part_off) * 512), SEEK_SET);
    got = fread(buf, 1, (size_t)n * 512, img);
    if (got < (size_t)n * 512) memset((uint8_t *)buf + got, 0, (size_t)n * 512 - got);   /* beyond EOF = zeros */
    sim_cmd_reads++;
    sim_sector_reads += n;
    if (n > 1) {                                                    /* the same for multi-sector reads */
        size_t len = (size_t)n * 512;
        rdn_count++;
        if (sim_rflipn_cmd > 0 && (int64_t)rdn_count == sim_rflipn_cmd) ((uint8_t *)buf)[len / 2 + 5] ^= 0x01;
        if (sim_rflipn_every) ((uint8_t *)buf)[(rdn_count * 4099) % len] ^= (uint8_t)(1u << (rdn_count % 8));
    }
    if (n == 1) {                                                   /* G1 read bit flips (only in what the app sees) */
        rd1_count++;
        if (sim_rflip_cmd > 0 && (int64_t)rd1_count == sim_rflip_cmd) ((uint8_t *)buf)[100] ^= 0x04;
        if (sim_rflip_every) ((uint8_t *)buf)[(rd1_count * 7) % 512] ^= (uint8_t)(1u << (rd1_count % 8));
    }
    return 0;
}

int ngdev_write(uint32_t lba, uint32_t n, const void *buf)
{
    static uint8_t tmp[256 * 512];
    uint32_t i;
    if (sim_dead || !img || n > 256 || (uint64_t)lba + sim_part_off + n > nsec) return -1;
    if (!sim_session && !sim_part_off) sim_unsessioned_writes++;
    if (sim_session && sim_protect_lba && lba < sim_protect_lba) {       /* firmware F7 */
        sim_violations++;
        sim_last_ok = 0;
        return -1;
    }
    sim_cmd_writes++;
    memcpy(tmp, buf, (size_t)n * 512);
    if (sim_flip_cmd > 0 && (int64_t)sim_cmd_writes == sim_flip_cmd) tmp[(size_t)n * 512 / 2] ^= 0x10;   /* G1 bit flip */
    sim_last_lba = lba;
    sim_last_n = n;
    sim_last_fnv = ng_fnv1a(NG_FNV_INIT, tmp, (size_t)n * 512);          /* what the "firmware" received */
    sim_last_ok = 1;
    for (i = 0; i < n; i++) {
        if (sim_cut_after >= 0 && (int64_t)sim_sector_writes >= sim_cut_after) {
            sim_dead = 1;
            sim_last_ok = 0;
            return -1;
        }
        fseek(img, (long)(((uint64_t)lba + sim_part_off + i) * 512), SEEK_SET);
        if (fwrite(tmp + (size_t)i * 512, 1, 512, img) != 512) return -1;
        sim_sector_writes++;
    }
    fflush(img);
    return 0;
}

int ngdev_flush(void) { return sim_dead ? -1 : 0; }
