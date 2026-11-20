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

#endif
