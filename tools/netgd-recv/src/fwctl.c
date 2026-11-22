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
     9 STACK     [3..=4] = allocation length                   -> 20 bytes "NGSK" (common/stackmark)
   Nothing here has run on hardware. */
#include <string.h>
#include <stdint.h>
#include "fwctl.h"
#include "fwxfer.h"

/* One command.  A PACKET that ends in CHECK CONDITION with sense key 6 (UNIT ATTENTION --
   the firmware raises one after a load / mount / END, and at power-up) was NOT executed:
   send it once more.  Any other error is the caller's. */
/* sense: the sense key of the last attempt (err >> 4; 0 for a transport failure), may be NULL.
   retry_ua = 0: a UNIT ATTENTION is reported to the caller instead of repeated (fw_test_unit_ready). */
static int atapi_s(const uint8_t pkt[12], void *data, uint32_t alloc, uint32_t *got, uint8_t *sense, int retry_ua)
{
    uint8_t err = 0;
    int rc = fw_xfer(pkt, data, alloc, got, &err);
    if (rc != 0 && retry_ua && (err >> 4) == 6) rc = fw_xfer(pkt, data, alloc, got, &err);
    if (sense) *sense = rc ? (uint8_t)(err >> 4) : 0;
    return rc;
}

static int atapi(const uint8_t pkt[12], void *data, uint32_t alloc, uint32_t *got)
{
    return atapi_s(pkt, data, alloc, got, NULL, 1);
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

int fw_stack(ng_fwstack_t *out)
{
    uint8_t b[20];
    uint32_t got = 0;
    memset(b, 0, sizeof b);
    if (ctl(9, 0, 0, 20, b, 20, &got) != 0 || got < 16 || memcmp(b, "NGSK", 4) || b[4] < 1) return -1;
    out->format = b[4];
    out->reserved = LE32(8);
    out->used = LE32(12);
    out->unpainted = got >= 20 ? LE32(16) : 0;
    return 0;
}

/* ---- the menu's commands: GDEMU 0x52 (mount by number, step, version), MOUNT_NAME + MOUNT, TEST UNIT READY ---- */

int fw_gdemu_mount(uint16_t n, uint8_t *sense)
{
    uint8_t p[12] = { 0x52, 0x82, (uint8_t)n, (uint8_t)(n >> 8) };
    return atapi_s(p, NULL, 0, NULL, sense, 1);
}

int fw_gdemu_step(int prev, uint8_t *sense)
{
    uint8_t p[12] = { 0x52, 0x81, (uint8_t)(prev ? 0x44 : 0x55) };
    return atapi_s(p, NULL, 0, NULL, sense, 1);
}

int fw_gdemu_version(uint32_t *boot, uint32_t *fw)
{
    uint8_t p[12] = { 0x52, 0x00, 0x00, 0x00, 0x08 };
    uint8_t b[8] = { 0 };
    uint32_t got = 0;
    if (atapi(p, b, 8, &got) != 0 || got < 8) return -1;
    if (boot) *boot = (uint32_t)b[0] | (uint32_t)b[1] << 8 | (uint32_t)b[2] << 16 | (uint32_t)b[3] << 24;
    if (fw) *fw = (uint32_t)b[4] | (uint32_t)b[5] << 8 | (uint32_t)b[6] << 16 | (uint32_t)b[7] << 24;
    return 0;
}

int fw_probe_netgd(ng_fwinfo_t *out, uint32_t *boot, uint32_t *fwv)
{
    static const uint8_t ver52[12] = { 0x52, 0x00 };                /* byte for byte the CDB of the stock record */
    static const uint8_t ver_e0[12] = { 0xE0, 3, 0, 0, 32 };
    uint8_t b[32];
    uint32_t got = 0, bw, fw;
    uint32_t keep = fw_xfer_limit_ms;
    int rc;
    fw_xfer_limit_ms = FW_PROBE_LIMIT_MS;
    memset(b, 0, sizeof b);
    rc = atapi_s(ver52, b, 8, &got, NULL, 1);
    if (rc != 0 || got < 8) { fw_xfer_limit_ms = keep; return FW_PROBE_NOANSWER; }
    bw = (uint32_t)b[0] | (uint32_t)b[1] << 8 | (uint32_t)b[2] << 16 | (uint32_t)b[3] << 24;
    fw = (uint32_t)b[4] | (uint32_t)b[5] << 8 | (uint32_t)b[6] << 16 | (uint32_t)b[7] << 24;
    if (boot) *boot = bw;
    if (fwv) *fwv = fw;
    if (fw != FW_NETGD_GDEMU_WORD) { fw_xfer_limit_ms = keep; return FW_PROBE_OTHER; }
    memset(b, 0, sizeof b);
    got = 0;
    rc = atapi_s(ver_e0, b, 32, &got, NULL, 0);
    fw_xfer_limit_ms = keep;
    if (rc != 0 || got < 8 || memcmp(b, "OGDM", 4)) return FW_PROBE_NOTOGDM;
    out->proto = b[4];
    out->caps = b[5];
    memcpy(out->fwstr, b + 8, 24);
    out->fwstr[24] = 0;
    return FW_PROBE_NETGD;
}

int fw_mount_named(const char *name, uint8_t *sense)
{
    uint8_t s = 0;
    if (stage_name(name) != 0) { if (sense) *sense = 0; return -1; }
    if (atapi_s((const uint8_t[12]){ 0xE0, 2, 0 }, NULL, 0, NULL, &s, 1) != 0) { if (sense) *sense = s; return -1; }
    if (sense) *sense = 0;
    return 0;
}

int fw_mount_default(uint8_t *sense)
{
    uint8_t p[12] = { 0xE0, 2, 0xFF };
    return atapi_s(p, NULL, 0, NULL, sense, 1);
}

int fw_test_unit_ready(uint8_t *sense)
{
    uint8_t p[12] = { 0x00 };
    return atapi_s(p, NULL, 0, NULL, sense, 0);
}
