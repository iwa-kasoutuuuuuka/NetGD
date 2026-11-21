#include <stdio.h>
#include <string.h>
#include "nm_launch.h"
#include "nm_plat.h"
#include "fwctl.h"
#include "ngfs.h"

#define POLL_MS 20

void nm_launch_begin(nm_launch_t *l, unsigned number, const char *folder, uint32_t limit_ms)
{
    memset(l, 0, sizeof *l);
    l->number = number;
    snprintf(l->folder, sizeof l->folder, "%s", folder);
    l->limit_ms = limit_ms;
    ngfs_unmount();                     /* a write session or a light mount must not be open while the image changes */
}

void nm_launch_wait_again(nm_launch_t *l)
{
    l->phase = 1;
    l->result = NM_L_BUSY;
    l->wait_start_ms = nm_plat_millis();
    l->last_poll_ms = l->wait_start_ms - POLL_MS;
    l->ready_streak = 0;
}

uint32_t nm_launch_waited_ms(const nm_launch_t *l)
{
    return l->phase == 1 ? nm_plat_millis() - l->wait_start_ms : 0;
}

nm_launch_result_t nm_launch_step(nm_launch_t *l)
{
    uint32_t now;
    uint8_t sense = 0;
    int rc;
    if (l->phase == 2) return (nm_launch_result_t)l->result;

    if (l->phase == 0) {
        rc = l->number ? fw_gdemu_mount((uint16_t)l->number, &sense) : fw_mount_named(l->folder, &sense);
        l->sense = sense;
        if (rc != 0) {
            l->result = sense == 5 ? NM_L_NOIMAGE : NM_L_FWERR;
            l->phase = 2;
            return (nm_launch_result_t)l->result;
        }
        nm_launch_wait_again(l);
        return NM_L_BUSY;
    }

    /* phase 1: TEST UNIT READY every POLL_MS; READY = 0 twice in a row (a UNIT ATTENTION or NOT READY resets the count) */
    now = nm_plat_millis();
    if ((uint32_t)(now - l->last_poll_ms) >= POLL_MS) {
        l->last_poll_ms = now;
        l->polls++;
        if (fw_test_unit_ready(&sense) == 0) {
            if (++l->ready_streak >= 2) { l->result = NM_L_OK; l->phase = 2; return NM_L_OK; }
        } else {
            l->ready_streak = 0;
            l->sense = sense;
        }
    }
    if ((uint32_t)(now - l->wait_start_ms) >= l->limit_ms) {
        l->result = NM_L_NOTREADY;
        l->phase = 2;
        return NM_L_NOTREADY;
    }
    return NM_L_BUSY;
}
