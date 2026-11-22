/* Packet-level model of the firmware's IMAGE_CTL commands and write session (docs:
   common/discimg/src/wsession.rs, mcu/netgd/src/ide.rs).  It is the transport under the
   real src/fwctl.c (fw_xfer), so the packet layouts and the UNIT ATTENTION retry are tested. */
#include <string.h>
#include <stdio.h>
#include "fwctl.h"
#include "fwxfer.h"
#include "simdisk.h"

int sim_session;
uint8_t sim_caps = 7;
int sim_end_calls = 0, sim_end_mode = -1, sim_begin_calls = 0;
char sim_end_name[128];
int sim_ua_mode;                    /* a UNIT ATTENTION is raised at start and after every END / MOUNT */
int sim_ua_pending, sim_ua_served;
int sim_fail_end;                   /* END of any mode fails: 1 = the session ends anyway, 2 = it stays open */
int sim_fail_end2;                  /* END mode 2 fails (ILLEGAL REQUEST) but still ends the session */
int sim_default_mounts, sim_named_mounts, sim_packets;

int sim_slots[64];
int sim_mounted_number;
int sim_notready_polls;
int sim_gdemu_mounts;
char sim_names[256];
int sim_trace;
int sim_fw_dead;
int sim_mount_in_session;
int sim_tur_calls;
uint32_t sim_fw_version = 0x05200000u;     /* 5.20.0: bytes 00 00 20 05 */
uint32_t sim_stack_reserved = 7128, sim_stack_used = 4816, sim_stack_unpainted = 96;   /* STACK (E0 09), with FW_CAP_STACK */
int sim_stack_calls;
static int notready_left;

static char staged[128];

static int slot_ok(int n) { for (int i = 0; i < 63 && sim_slots[i]; i++) if (sim_slots[i] == n) return 1; return 0; }

static int name_ok(void)
{
    char *p, up[sizeof sim_names + 2];
    size_t i, l = strlen(staged);
    if (!sim_names[0]) return 1;
    snprintf(up, sizeof up, "|%s|", sim_names);
    for (i = 0; up[i]; i++) if (up[i] >= 'a' && up[i] <= 'z') up[i] = (char)(up[i] - 32);
    {
        char key[160];
        if (l > 120) return 0;
        key[0] = '|';
        for (i = 0; i < l; i++) key[1 + i] = (staged[i] >= 'a' && staged[i] <= 'z') ? (char)(staged[i] - 32) : staged[i];
        key[1 + l] = '|'; key[2 + l] = 0;
        p = strstr(up, key);
    }
    return p != NULL;
}

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
    if (sim_fw_dead) {                          /* not a NetGD board: nothing answers (timeout, no sense key) */
        if (sim_trace) printf("FW (no answer)\n");
        return -1;
    }
    if (sim_trace) {
        printf("FW");
        for (int i = 0; i < 12; i++) printf(" %02X", pkt[i]);
        printf("\n");
    }
    if (sim_ua_pending) {                       /* not executed: CHECK CONDITION, sense key 6 */
        sim_ua_pending = 0;
        sim_ua_served++;
        *err = 0x60;
        return -1;
    }
    if (pkt[0] == 0x00) {                       /* TEST UNIT READY */
        sim_tur_calls++;
        if (notready_left > 0) { notready_left--; *err = 0x20; return -1; }
        return 0;
    }
    if (pkt[0] == 0x52) {                       /* GDEMU: 00 version, 81 step, 82 mount by number */
        if (pkt[1] == 0x00) {
            uint8_t b[8];
            uint32_t v = sim_fw_version;
            memcpy(b, &v, 4);
            memcpy(b + 4, &v, 4);
            return copy_out(b, 8, data, alloc, got);
        }
        if (pkt[1] != 0x81 && pkt[1] != 0x82) { *err = 0x50; return -1; }
        if (sim_session) { sim_mount_in_session++; *err = 0x50; return -1; }
        if (pkt[1] == 0x82) {
            int n = pkt[2] | pkt[3] << 8;
            if (n == 0) { sim_mounted_number = 0; sim_gdemu_mounts++; raise_ua(); notready_left = sim_notready_polls; return 0; }
            if (!slot_ok(n)) { *err = 0x50; return -1; }
            sim_mounted_number = n;
        } else {
            int cnt = 0, idx = -1, next;
            while (cnt < 63 && sim_slots[cnt]) { if (sim_slots[cnt] == sim_mounted_number) idx = cnt; cnt++; }
            if (!cnt || (pkt[2] != 0x55 && pkt[2] != 0x44)) { *err = 0x50; return -1; }
            next = pkt[2] == 0x55 ? (idx + 1) % cnt : (idx <= 0 ? cnt - 1 : idx - 1);
            sim_mounted_number = sim_slots[next];
        }
        sim_gdemu_mounts++;
        raise_ua();
        notready_left = sim_notready_polls;
        return 0;
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
        if (sim_session) { sim_mount_in_session++; *err = 0x50; return -1; }
        if (pkt[2] == 0xFF) sim_default_mounts++;
        else {
            if (!name_ok()) { *err = 0x50; return -1; }
            sim_named_mounts++;
        }
        raise_ua();
        notready_left = sim_notready_polls;
        return 0;
    case 7: {                                   /* WSESSION_END */
        int mode = pkt[2];
        if (!sim_session) { *err = 0x50; return -1; }       /* END without a session: ILLEGAL REQUEST */
        if (sim_fail_end == 2) { *err = 0x50; return -1; }  /* fails and the session stays open */
        sim_session = 0;
        sim_end_calls++;
        sim_end_mode = mode;
        sim_end_name[0] = 0;
        raise_ua();
        if (sim_fail_end == 1) { *err = 0x50; return -1; }  /* fails after the session ended */
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
    case 9: {                                   /* STACK (firmware feature stack-hw) */
        uint8_t b[20];
        sim_stack_calls++;
        if (!(sim_caps & FW_CAP_STACK)) { *err = 0x50; return -1; }
        memset(b, 0, sizeof b);
        memcpy(b, "NGSK", 4);
        b[4] = 1;
        memcpy(b + 8, &sim_stack_reserved, 4);
        memcpy(b + 12, &sim_stack_used, 4);
        memcpy(b + 16, &sim_stack_unpainted, 4);
        return copy_out(b, sizeof b, data, alloc, got);
    }
    default:
        *err = 0x50;
        return -1;
    }
}
