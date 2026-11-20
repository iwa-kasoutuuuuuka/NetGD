/* gdprobe: the ONE table of commands this tool may send to the drive.
 * The runner refuses (rc=refused, nothing sent) anything wl_check() rejects.
 * Pure C: host-tested. */
#ifndef GDPROBE_WHITELIST_H
#define GDPROBE_WHITELIST_H
#include <stdint.h>

typedef struct wl_entry {
    const char *name;
    uint8_t opcode;      /* CDB byte 0 */
    uint16_t mask;       /* bit i set = CDB byte i may be non-zero; all others must be 0 */
    int optin;           /* 1 = only when the config enabled image switching */
    const char *why;     /* why this command cannot harm the drive */
} wl_entry;

extern const wl_entry wl_table[];
extern const int wl_count;

/* Returns the matching entry, or NULL = refuse. `switch_enabled` is the config
 * flag for the opt-in entries. Besides the opcode and byte mask each entry has
 * argument limits (e.g. 0x52 only with sub-code 0x00 or 0x82, CD_READ at most
 * 64 sectors, no MSF addressing). */
const wl_entry *wl_check(const uint8_t cdb[12], int switch_enabled);

/* CD_READ byte 1 (the Sega flags: bit7 Header, 6 Subheader, 5 Data, 4 Other =
 * whole 2352-byte sector, bits 3-1 expected data type, bit 0 = MSF) is only
 * accepted for these values (FAD mode, bit 0 clear): 0x20 0x24 0x28 0x10 -- the
 * CDREAD_FLAGS probe -- plus the one value the config's cdread_flags names.
 * Anything else (0x30, 0xE6, 0xF0 ...) is refused. wl_check() uses the config
 * default (0x24); the runner calls wl_check_cfg() with the configured value. */
#define WL_CDREAD_FLAGS_DEFAULT 0x24
const wl_entry *wl_check_cfg(const uint8_t cdb[12], int switch_enabled, uint8_t cdread_flags);
int wl_cdread_flags_probe_ok(uint8_t byte1);   /* byte1 in {0x20,0x24,0x28,0x10} */

#endif
