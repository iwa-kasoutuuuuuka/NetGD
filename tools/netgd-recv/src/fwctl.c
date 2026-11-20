/* The firmware command layer (host-testable: the transport is fw_xfer, see fwxfer.h).
   ATAPI PACKET commands to the PRIMARY (GD-ROM) device.

   Packet layouts (all packet[0] = 0xE0 IMAGE_CTL), from mcu/netgd/src/ide.rs and
   common/discimg/src/wsession.rs:
     3 VERSION   [3..=4] = allocation length (big endian)      -> 32 bytes
     5 MOUNT_NAME[2] = chunk index, [3..12] = up to 9 name bytes (chunk 0 starts a name)
     2 MOUNT     [2] = 0xFF: the GDEMU.CFG default; else the staged name
     6 WSESSION_BEGIN [2..6] = "NGWS"
     7 WSESSION_END   [2] = 0 leave empty / 1 load_media / 2 mount the staged name
     8 STATUS    [3..=4] = allocation length                   -> 48 bytes (see fwctl.h)
   Nothing here has run on hardware. */
#include <string.h>
#include <stdint.h>
#include "fwctl.h"
#include "fwxfer.h"

/* One command.  A PACKET that ends in CHECK CONDITION with sense key 6 (UNIT ATTENTION --
   the firmware raises one after a load / mount / END, and at power-up) was NOT executed:
   send it once more.  Any other error is the caller's. */
static int atapi(const uint8_t pkt[12], void *data, uint32_t alloc, uint32_t *got)
{
    uint8_t err = 0;
    int rc = fw_xfer(pkt, data, alloc, got, &err);
    if (rc != 0 && (err >> 4) == 6) rc = fw_xfer(pkt, data, alloc, got, &err);
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


int fw_session_begin(const ng_fwinfo_t *fw)
{
    uint8_t p[12] = { 0xE0, 6, 'N', 'G', 'W', 'S' };
    (void)fw;           /* callers (ng_session_open) refuse firmware without FW_CAP_SESSION|FW_CAP_CHECKSUM */
    return atapi(p, NULL, 0, NULL);
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
    (void)fw;
    if (mode == 2 && (!name || stage_name(name) != 0)) mode = 1;
    rc = ctl(7, (uint8_t)mode, 0, 0, NULL, 0, NULL);
    if (rc != 0 && mode == 2) {
        /* the firmware ended the session even though the mount failed (ILLEGAL REQUEST):
           bring the default image back so the console is not left without a disc.
           (The END may have raised a UNIT ATTENTION: atapi() retries over it.) */
        ctl(2, 0xFF, 0, 0, NULL, 0, NULL);
    }
    return rc;
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
