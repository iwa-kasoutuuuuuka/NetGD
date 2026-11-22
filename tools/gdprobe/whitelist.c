#include "whitelist.h"
#include <stddef.h>

#define B(i) (1u << (i))

/* Why each command is safe:
 *  - The ATAPI/Sega commands below only ask the drive to REPORT something
 *    (status, error/sense, TOC, session table, sub-code Q, mode page) or to READ
 *    data sectors. None of them changes the drive's state, the medium, or the
 *    firmware.
 *  - Never present, on purpose: SET_MODE 0x12, CD_OPEN 0x16, CD_PLAY/SEEK/SCAN
 *    0x20-0x22 (move the pickup / audio state), every write/format, the
 *    security commands 0x70/0x71, every other vendor opcode, and every 0x52
 *    sub-code except 0x00 (version) and 0x82 (mount image N, opt-in).
 *  - Nothing is ever sent to the secondary IDE device: gdraw.c never writes
 *    the device-select register.
 */
const wl_entry wl_table[] = {
    { "TEST_UNIT_READY", 0x00, B(0), 0,
      "ATAPI: reports ready/not ready only; no data, no state change (UNIT ATTENTION is cleared, as for any host)" },
    { "REQ_STAT", 0x10, B(0) | B(2) | B(4), 0,
      "Sega: reads the audio/drive status block (10 bytes)" },
    { "REQ_MODE", 0x11, B(0) | B(2) | B(4), 0,
      "Sega: reads mode data (the BIOS reads this at every boot); this is NOT SET_MODE" },
    { "REQ_ERROR", 0x13, B(0) | B(4), 0,
      "Sega: reads the 10-byte error/sense block (byte 2 low nibble = sense key, 8 = ASC, 9 = ASCQ); the GD-ROM replacement for REQUEST SENSE, which is not a GD-ROM command" },
    { "GET_TOC", 0x14, B(0) | B(1) | B(3) | B(4), 0,
      "Sega: reads the 408-byte TOC of the SD (0) or HD (1) area" },
    { "REQ_SES", 0x15, B(0) | B(2) | B(4), 0,
      "Sega: reads the start of session N (the BIOS does this at boot)" },
    { "GET_SCD", 0x40, B(0) | B(1) | B(3) | B(4), 0,
      "Sega: reads the sub-code Q (format 0: 100 bytes, 1: 14 bytes) at the current position" },
    { "CD_READ", 0x30, B(0) | B(1) | B(2) | B(3) | B(4) | B(8) | B(9) | B(10), 0,
      "Sega: reads data sectors (FAD addressing, at most 64 sectors; byte 1 only 0x20/0x24/0x28/0x10 or the configured cdread_flags); read-only" },
    { "GDEMU_VERSION", 0x52, B(0), 0,
      "GDEMU: `52 00` returns the 8-byte bootloader/firmware version (read-only; used by GDmenu tools)" },
    { "GDEMU_MOUNT", 0x52, B(0) | B(1) | B(2) | B(3), 1,
      "GDEMU: `52 82 lo hi` mounts folder N; the same command openMenu sends when it boots a game. Opt-in" },
};
const int wl_count = (int)(sizeof(wl_table) / sizeof(wl_table[0]));

/* ATA commands (COMMAND register). Both are opt-in; nothing here touches the medium or the firmware.
 * Never present: every write / format / download-microcode / set-features / sleep / standby command, and
 * EXECUTE DEVICE DIAGNOSTIC (0x90, which also resets the secondary device). */
const wl_ata_entry wl_ata_table[] = {
    { "IDENTIFY_PACKET_DEVICE", 0xA1, WL_ATA_IDENTIFY,
      "ATA: returns the 80-byte identification block; read-only (config identify=on)" },
    { "DEVICE_RESET", 0x08, WL_ATA_DEVRESET,
      "ATA (ATA-4 8.7, mandatory for packet devices): resets the device's interface/protocol state; no data, "
      "nothing written to the medium; addressed to the selected (master) device only, DEVICE_SELECT is never "
      "written. Only for the R148 swap-window probe (switch=on and swap_devreset=on or devreset_only=on)" },
};
const int wl_ata_count = (int)(sizeof(wl_ata_table) / sizeof(wl_ata_table[0]));

const wl_ata_entry *wl_ata_check(uint8_t command, int identify_enabled, int devreset_enabled)
{
    int i;
    for (i = 0; i < wl_ata_count; i++) {
        const wl_ata_entry *e = &wl_ata_table[i];
        if (e->command != command)
            continue;
        if (e->optin == WL_ATA_IDENTIFY && identify_enabled)
            return e;
        if (e->optin == WL_ATA_DEVRESET && devreset_enabled)
            return e;
    }
    return NULL;
}

static unsigned be16(const uint8_t *p) { return ((unsigned)p[0] << 8) | p[1]; }

int wl_cdread_flags_probe_ok(uint8_t b)
{
    return b == 0x20 || b == 0x24 || b == 0x28 || b == 0x10;
}

static int args_ok(const wl_entry *e, const uint8_t *c, uint8_t cdread_flags)
{
    switch (c[0]) {
    case 0x10: return c[4] <= 10;
    case 0x11: return (unsigned)c[2] + c[4] <= 32;
    case 0x13: return c[4] <= 10;
    case 0x14: return c[1] <= 1 && be16(c + 3) <= 408;
    case 0x15: return c[2] <= 3 && c[4] <= 16;
    case 0x40: return c[1] <= 1 && be16(c + 3) <= 100;
    case 0x30: {
        unsigned n = ((unsigned)c[8] << 16) | ((unsigned)c[9] << 8) | c[10];
        return (c[1] & 1) == 0 && n >= 1 && n <= 64 &&
               (wl_cdread_flags_probe_ok(c[1]) || c[1] == (uint8_t)(cdread_flags & 0xFE));
    }
    case 0x52:
        if (e->opcode == 0x52 && e->mask == (B(0))) /* version */
            return c[1] == 0x00;
        {
            unsigned n = (unsigned)c[2] | ((unsigned)c[3] << 8);
            return c[1] == 0x82 && n >= 1 && n <= 999;
        }
    default: return 1;
    }
}

const wl_entry *wl_check(const uint8_t cdb[12], int switch_enabled)
{
    return wl_check_cfg(cdb, switch_enabled, WL_CDREAD_FLAGS_DEFAULT);
}

const wl_entry *wl_check_cfg(const uint8_t cdb[12], int switch_enabled, uint8_t cdread_flags)
{
    int i, k;
    for (i = 0; i < wl_count; i++) {
        const wl_entry *e = &wl_table[i];
        if (e->opcode != cdb[0])
            continue;
        if (e->optin && !switch_enabled)
            continue;
        for (k = 0; k < 12; k++)
            if (cdb[k] && !(e->mask & (1u << k)))
                break;
        if (k != 12)
            continue;
        if (!args_ok(e, cdb, cdread_flags))
            continue;
        return e;
    }
    return NULL;
}
