/* The gate of the update confirmation: the whole combination must be pressed after it was seen released, on the same pads, with
   the cancel button up, for the full hold.  pad 0 = no pad, or a status no real pad produces (see pad_sample in main.c): never
   counts.  A stuck pad can therefore never start a flash.  Pure logic, tested on the host (tests/test_all.c). */
#ifndef FWCONFIRM_H
#define FWCONFIRM_H
#include <stdint.h>
#include "../netgd-recv/src/ng_btnguard.h"

static inline int fw_confirm_step(ng_holdconfirm_t *c, uint32_t pad, uint32_t buttons, uint32_t combo, uint32_t cancel,
                                  uint32_t now_ms, uint32_t hold_ms)
{
    int want = pad && (buttons & combo) == combo && !(buttons & cancel);
    return ng_holdconfirm_update(c, pad, want, now_ms, hold_ms);
}

#endif
