/* Packet-level model of the firmware's IMAGE_CTL commands and write session (docs:
   common/discimg/src/wsession.rs, mcu/netgd/src/ide.rs).  It is the transport under the
   real src/fwctl.c (fw_xfer), so the packet layouts and the UNIT ATTENTION retry are tested. */
#include <string.h>
#include "fwctl.h"
#include "fwxfer.h"
#include "simdisk.h"

int sim_session;
uint8_t sim_caps = 7;
int sim_end_calls = 0, sim_end_mode = -1, sim_begin_calls = 0;
char sim_end_name[128];
int sim_ua_mode;                    /* a UNIT ATTENTION is raised at start and after every END / MOUNT */
int sim_ua_pending, sim_ua_served;
int sim_fail_end2;                  /* END mode 2 fails (ILLEGAL REQUEST) but still ends the session */
int sim_default_mounts, sim_named_mounts, sim_packets;

static char staged[128];

static void raise_ua(void) { if (sim_ua_mode) sim_ua_pending = 1; }

static int copy_out(const uint8_t *src, uint32_t n, void *data, uint32_t alloc, uint32_t *got)
{
    uint32_t c = n < alloc ? n : alloc;
    if (data) memcpy(data, src, c);
    if (got) *got = c;
    return 0;
}

int fw_xfer(const uint8_t pkt[12], void *data, uint32_t alloc, uint32_t *got, uint8_t *err)
{
    *err = 0;
    if (got) *got = 0;
    sim_packets++;
    if (sim_ua_pending) {                       /* not executed: CHECK CONDITION, sense key 6 */
        sim_ua_pending = 0;
        sim_ua_served++;
        *err = 0x60;
        return -1;
    }
    if (pkt[0] != 0xE0) { *err = 0x50; return -1; }
    switch (pkt[1]) {
    case 3: {                                   /* VERSION */
        uint8_t b[32];
        memset(b, 0, sizeof b);
        memcpy(b, "OGDM", 4);
        b[4] = 3;
        b[5] = sim_caps;
        strcpy((char *)b + 8, "sim-0.0");
        return copy_out(b, sizeof b, data, alloc, got);
    }
    case 6:                                     /* WSESSION_BEGIN */
        if (memcmp(pkt + 2, "NGWS", 4) || !(sim_caps & FW_CAP_SESSION) || sim_session) { *err = 0x50; return -1; }
        sim_begin_calls++;
        sim_session = 1;
        return 0;
    case 5: {                                   /* MOUNT_NAME */
        size_t off = (size_t)pkt[2] * 9;
        if (off + 10 > sizeof staged) { *err = 0x50; return -1; }
        if (pkt[2] == 0) memset(staged, 0, sizeof staged);
        memcpy(staged + off, pkt + 3, 9);
        return 0;
    }
    case 2:                                     /* MOUNT */
        if (pkt[2] == 0xFF) sim_default_mounts++; else sim_named_mounts++;
        raise_ua();
        return 0;
    case 7: {                                   /* WSESSION_END */
        int mode = pkt[2];
        if (!sim_session) { *err = 0x50; return -1; }       /* END without a session: ILLEGAL REQUEST */
        sim_session = 0;
        sim_end_calls++;
        sim_end_mode = mode;
        sim_end_name[0] = 0;
        raise_ua();
        if (mode == 2) {
            if (sim_fail_end2) { *err = 0x50; return -1; }
            strncpy(sim_end_name, staged, sizeof sim_end_name - 1);
        }
        return 0;
    }
    case 8: {                                   /* STATUS */
        uint8_t b[48];
        uint32_t v;
        memset(b, 0, sizeof b);
        memcpy(b, "NGST", 4);
        b[4] = (uint8_t)((sim_session ? FW_ST_SESSION : 0) | (sim_last_ok ? FW_ST_LASTOK : 0));
        v = sim_last_lba;  memcpy(b + 16, &v, 4);
        v = sim_last_n;    memcpy(b + 20, &v, 4);
        v = sim_last_fnv;  memcpy(b + 24, &v, 4);
        v = sim_protect_lba; memcpy(b + 28, &v, 4);
        return copy_out(b, sizeof b, data, alloc, got);
    }
    default:
        *err = 0x50;
        return -1;
    }
}
