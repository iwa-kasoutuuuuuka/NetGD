/* KOS build: the firmware command layer.  Raw ATAPI PACKET commands to the PRIMARY
   (GD-ROM) device through the G1 ATA registers, under g1_ata_mutex_lock.

   Packet layouts (all packet[0] = 0xE0 IMAGE_CTL), from mcu/opengdemu/src/ide.rs and
   common/discimg/src/wsession.rs:
     3 VERSION   [3..=4] = allocation length (big endian)      -> 32 bytes
     5 MOUNT_NAME[2] = chunk index, [3..12] = up to 9 name bytes (chunk 0 starts a name)
     2 MOUNT     [2] = 0xFF: the GDEMU.CFG default; else the staged name
     6 WSESSION_BEGIN [2..6] = "NGWS"
     7 WSESSION_END   [2] = 0 leave empty / 1 load_media / 2 mount the staged name
     8 STATUS    [3..=4] = allocation length                   -> 48 bytes (see fwctl.h)
   Nothing here has run on hardware.  The register protocol below is the plain ATAPI PIO one. */
#include <string.h>
#include <stdint.h>
#include <kos.h>
#include <dc/g1ata.h>
#include "fwctl.h"

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

/* wait until (status & mask) == want; 0 ok, -1 timeout */
static int wait_for(uint8_t mask, uint8_t want, uint32_t ms)
{
    uint64_t t0 = timer_ms_gettime64();
    while ((IN8(G1_ALTSTATUS) & mask) != want)
        if (timer_ms_gettime64() - t0 > ms) return -1;
    return 0;
}

/* One ATAPI command.  data/alloc: data-in buffer (alloc 0 = none). 0 ok, -1 error/timeout. */
static int atapi(const uint8_t pkt[12], void *data, uint32_t alloc, uint32_t *got)
{
    int rc = -1;
    uint8_t st;
    if (got) *got = 0;
    if (g1_ata_mutex_lock() != 0) return -1;
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
    if (st & SR_ERR) goto out;
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
        if (IN8(G1_STATUS) & SR_ERR) goto out;
    }
    rc = 0;
out:
    g1_ata_select_device(G1_ATA_SLAVE);                     /* the secondary disk is what the rest of the app uses */
    g1_ata_mutex_unlock();
    return rc;
}

static int ctl(uint8_t sub, uint8_t b2, uint8_t b3, uint8_t b4, void *data, uint32_t alloc, uint32_t *got)
{
    uint8_t p[12] = { 0xE0, sub, b2, b3, b4 };
    return atapi(p, data, alloc, got);
}

int fw_version(ng_fwinfo_t *out)
{
    uint8_t b[32];
    uint32_t got = 0;
    memset(b, 0, sizeof b);
    if (ctl(3, 0, 0, 32, b, 32, &got) != 0 || got < 8 || memcmp(b, "OGDM", 4)) return -1;
    out->proto = b[4];
    out->caps = b[5];
    memcpy(out->fwstr, b + 8, 24);
    out->fwstr[24] = 0;
    return 0;
}

static int eject(void) { uint8_t p[12] = { 0x1B, 0, 0, 0, 0x02 }; return atapi(p, NULL, 0, NULL); }   /* START STOP UNIT LoEj=1 Start=0 */
static int load(void) { uint8_t p[12] = { 0x1B, 0, 0, 0, 0x03 }; return atapi(p, NULL, 0, NULL); }    /* LoEj=1 Start=1 */

int fw_session_begin(const ng_fwinfo_t *fw)
{
    uint8_t p[12] = { 0xE0, 6, 'N', 'G', 'W', 'S' };
    if (fw->caps & FW_CAP_SESSION) return atapi(p, NULL, 0, NULL);
    return eject();     /* old firmware: no protected area, no check value, buttons not locked */
}

static int stage_name(const char *name)
{
    size_t n = strlen(name);
    for (uint8_t chunk = 0; (size_t)chunk * 9 < n + 1; chunk++) {
        uint8_t p[12] = { 0xE0, 5, chunk };
        size_t off = (size_t)chunk * 9, l = n > off ? n - off : 0;
        if (l > 9) l = 9;
        memcpy(p + 3, name + off, l);
        if (atapi(p, NULL, 0, NULL) != 0) return -1;
    }
    return 0;
}

int fw_session_end(const ng_fwinfo_t *fw, int mode, const char *name)
{
    int rc;
    if (mode == 2 && (!name || stage_name(name) != 0)) mode = 1;
    if (fw->caps & FW_CAP_SESSION) {
        rc = ctl(7, (uint8_t)mode, 0, 0, NULL, 0, NULL);
        if (rc != 0 && mode == 2) {
            /* the firmware ended the session even though the mount failed (ILLEGAL REQUEST):
               bring the default image back so the console is not left without a disc */
            ctl(2, 0xFF, 0, 0, NULL, 0, NULL);
        }
        return rc;
    }
    if (mode == 0) return 0;
    if (mode == 2) return ctl(2, 0, 0, 0, NULL, 0, NULL);
    return load();
}

int fw_status(ng_fwstatus_t *out)
{
    uint8_t b[48];
    uint32_t got = 0;
    if (ctl(8, 0, 0, 48, b, 48, &got) != 0 || got < 32 || memcmp(b, "NGST", 4)) return -1;
#define LE32(o) ((uint32_t)b[o] | (uint32_t)b[(o) + 1] << 8 | (uint32_t)b[(o) + 2] << 16 | (uint32_t)b[(o) + 3] << 24)
    out->flags = b[4];
    out->sectors_written = LE32(8);
    out->failed_cmds = LE32(12);
    out->last_lba = LE32(16);
    out->last_count = LE32(20);
    out->last_fnv = LE32(24);
    out->part_lba = LE32(28);
    return 0;
}
