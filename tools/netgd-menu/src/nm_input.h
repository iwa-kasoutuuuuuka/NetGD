/* Button edges and auto-repeat (design 7.3). */
#ifndef NM_INPUT_H
#define NM_INPUT_H
#include <stdint.h>
#include "nm.h"

#define NM_REPEAT_DELAY_MS 300
#define NM_REPEAT_PERIOD_MS 80
#define NM_REPEAT_MASK (NM_BTN_UP | NM_BTN_DOWN)

typedef struct {
    uint32_t held;              /* the buttons down at the last update */
    uint32_t press_ms[16];      /* when each bit went down */
    uint32_t next_rep[16];
} nm_input_t;

void nm_input_init(nm_input_t *in);
/* Feed the current button bits; returns the events of this frame: every button that went down, plus
   UP/DOWN again after NM_REPEAT_DELAY_MS and then every NM_REPEAT_PERIOD_MS while it stays down. */
uint32_t nm_input_update(nm_input_t *in, uint32_t buttons, uint32_t now_ms);
/* how long `btn` (one bit) has been held, in ms; 0 when it is up */
uint32_t nm_input_held_ms(const nm_input_t *in, uint32_t btn, uint32_t now_ms);

#endif
