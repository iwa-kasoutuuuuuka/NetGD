#include <string.h>
#include "nm.h"
#include "nm_input.h"

void nm_input_init(nm_input_t *in)
{
    memset(in, 0, sizeof *in);
}

uint32_t nm_input_update(nm_input_t *in, uint32_t buttons, uint32_t now_ms)
{
    uint32_t ev = 0;
    for (int b = 0; b < 16; b++) {
        uint32_t bit = 1u << b;
        if (buttons & bit) {
            if (!(in->held & bit)) {
                ev |= bit;
                in->press_ms[b] = now_ms;
                in->next_rep[b] = now_ms + NM_REPEAT_DELAY_MS;
            } else if ((NM_REPEAT_MASK & bit) && (int32_t)(now_ms - in->next_rep[b]) >= 0) {
                ev |= bit;
                in->next_rep[b] = now_ms + NM_REPEAT_PERIOD_MS;
            }
        }
    }
    in->held = buttons;
    return ev;
}

uint32_t nm_input_held_ms(const nm_input_t *in, uint32_t btn, uint32_t now_ms)
{
    for (int b = 0; b < 16; b++)
        if ((btn & (1u << b)) && (in->held & (1u << b))) return now_ms - in->press_ms[b];
    return 0;
}
