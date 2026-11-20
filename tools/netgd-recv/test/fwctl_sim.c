/* Model of the firmware's write session (docs: common/discimg/src/wsession.rs). */
#include <string.h>
#include "fwctl.h"
#include "simdisk.h"

int sim_session;
uint8_t sim_caps = 7;
int sim_end_calls = 0, sim_end_mode = -1, sim_begin_calls = 0;
char sim_end_name[128];

int fw_version(ng_fwinfo_t *out)
{
    out->proto = 3;
    out->caps = sim_caps;
    strcpy(out->fwstr, "sim-0.0");
    return 0;
}

int fw_session_begin(const ng_fwinfo_t *fw)
{
    (void)fw;
    sim_begin_calls++;
    sim_session = 1;
    return 0;
}

int fw_session_end(const ng_fwinfo_t *fw, int mode, const char *name)
{
    (void)fw;
    if (!sim_session) return -1;                    /* END without a session: ILLEGAL REQUEST */
    sim_session = 0;
    sim_end_calls++;
    sim_end_mode = mode;
    sim_end_name[0] = 0;
    if (name) strncpy(sim_end_name, name, sizeof sim_end_name - 1);
    return 0;
}

int fw_status(ng_fwstatus_t *out)
{
    memset(out, 0, sizeof *out);
    out->flags = (uint8_t)((sim_session ? FW_ST_SESSION : 0) | (sim_last_ok ? FW_ST_LASTOK : 0));
    out->last_lba = sim_last_lba;
    out->last_count = sim_last_n;
    out->last_fnv = sim_last_fnv;
    out->part_lba = sim_protect_lba;
    return 0;
}
