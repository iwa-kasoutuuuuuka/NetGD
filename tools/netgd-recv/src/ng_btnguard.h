/* A button that counts only after it has been seen released.

   Real hardware: a controller reported B in 400 of 400 samples, whatever was done with it.  A level ("B is down") then means
   "stop" or "leave" at once.  This guard ignores the button until the pad has shown it up for a while:
   - no pad (pad == 0) is never "released": the buttons of a missing pad read 0;
   - a different pad (unplug and replug, another port) starts over, so a stuck pad plugged back in is ignored again;
   - the release must be stable: NG_BTN_RELEASE_SAMPLES samples in a row over NG_BTN_RELEASE_MS, because one sample that reads 0
     is not a release, and the status only changes once per Maple frame (16 ms) however often it is polled.

   Used by netgd-menu, netgd-t2 and netgd-recv.  Header only: no platform calls, the caller passes what it read. */
#ifndef NG_BTNGUARD_H
#define NG_BTNGUARD_H
#include <stdint.h>

#define NG_BTN_RELEASE_MS      120u
#define NG_BTN_RELEASE_SAMPLES 4u

typedef struct {
    uint32_t pad;                   /* the pad of the last sample; 0 = none */
    uint32_t up_since;              /* when the current run of "up" samples began */
    uint8_t up_n;                   /* samples in that run (saturates) */
    uint8_t armed;                  /* seen released: the level counts now */
    uint8_t seen_down;              /* not armed, and this pad has shown the button down */
} ng_btnguard_t;

/* One sample.  pad: which pad it was read from (0 = none), down: the button's level.  1 = the button counts as down. */
static inline int ng_btnguard_update(ng_btnguard_t *g, uint32_t pad, int down, uint32_t now_ms)
{
    if (pad != g->pad) {
        g->pad = pad;
        g->up_n = 0;
        g->armed = 0;
        g->seen_down = 0;
    }
    if (!pad) return 0;
    if (g->armed) return down != 0;
    if (down) {
        g->up_n = 0;
        g->seen_down = 1;
        return 0;
    }
    if (!g->up_n) g->up_since = now_ms;
    if (g->up_n < 255) g->up_n++;
    if (g->up_n >= NG_BTN_RELEASE_SAMPLES && (uint32_t)(now_ms - g->up_since) >= NG_BTN_RELEASE_MS) {
        g->armed = 1;
        g->seen_down = 0;
    }
    return 0;
}

/* 1 while a present pad holds the button without ever having released it: the screens offer another way out */
static inline int ng_btnguard_stuck(const ng_btnguard_t *g)
{
    return g->pad && !g->armed && g->seen_down;
}

/* host tests: this pad is known to be healthy, its next press counts */
static inline void ng_btnguard_prearm(ng_btnguard_t *g, uint32_t pad)
{
    g->pad = pad;
    g->up_n = 0;
    g->armed = 1;
    g->seen_down = 0;
}

#endif
