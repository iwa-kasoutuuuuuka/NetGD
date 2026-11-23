/* The gate of the update confirmation.  First EVERY combination button and the cancel button must be seen up together (a stable
   release, on the same pads); only then does the full combination, with cancel up, count for the full hold.  So buttons stuck
   down cannot be completed by pressing the others.  pad 0 = no pad, or a status no real pad produces: never counts.
   Pure logic, tested on the host (tests/test_all.c). */
#ifndef FWCONFIRM_H
#define FWCONFIRM_H
#include <stdint.h>
#include "../netgd-recv/src/ng_btnguard.h"

static inline int fw_confirm_step(ng_holdconfirm_t *c, uint32_t pad, uint32_t buttons, uint32_t combo, uint32_t cancel,
                                  uint32_t now_ms, uint32_t hold_ms)
{
    uint32_t watch = combo | cancel;
    if (pad != c->g.pad) c->holding = 0;
    ng_btnguard_update(&c->g, pad, (buttons & watch) != 0, now_ms);
    if (!pad || !c->g.armed || (buttons & watch) != combo) {
        c->holding = 0;
        return 0;
    }
    if (!c->holding) {
        c->holding = 1;
        c->since = now_ms;
    }
    return (uint32_t)(now_ms - c->since) >= hold_ms;
}

/* All pads of one sample.  b: the buttons of the healthy pads (so B on a healthy pad can cancel); a missing status or an
   implausible pad on any port makes fw_pads_id 0, so nothing can approve. */
typedef struct {
    uint32_t id, b;
    int bad;
} fw_pads_t;

static inline void fw_pads_add(fw_pads_t *p, uint32_t unit_id, int have_status, uint32_t buttons, uint32_t up, uint32_t down,
                               uint32_t left, uint32_t right, uint32_t face)
{
    if (!have_status || ng_pad_implausible(buttons, up, down, left, right, face)) {
        p->bad = 1;
        return;
    }
    p->b |= buttons;
    p->id = p->id * 37u + unit_id;
}

static inline uint32_t fw_pads_id(const fw_pads_t *p) { return p->bad ? 0 : p->id; }

#endif
