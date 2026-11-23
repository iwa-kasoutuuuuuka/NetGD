/* A button that counts only after it has been seen released.

   Real hardware: a controller reported B in 400 of 400 samples, whatever was done with it.  A level ("B is down") then means
   "stop" or "leave" at once.  This guard ignores the button until the pad has shown it up for a while:
   - no pad (pad == 0) is never "released": the buttons of a missing pad read 0;
   - a different pad (unplug and replug, another port) starts over, so a stuck pad plugged back in is ignored again;
   - the release must be stable: NG_BTN_RELEASE_SAMPLES samples in a row over NG_BTN_RELEASE_MS, because one sample that reads 0
     is not a release, and the status only changes once per Maple frame (16 ms) however often it is polled.

   Limit (KOS 2.3): the status buffer of a new device is calloc'd and has no "valid" flag, so until its first reply a pad reads
   "nothing pressed", which cannot be told from a real reply.  The first reply normally comes within a frame or two, far inside
   NG_BTN_RELEASE_MS, and then resets the run.  A pad that attaches and never replies for 120 ms would count as released.

   Used by netgd-menu, netgd-t2, netgd-recv and netgd-fwupd.  Header only: no platform calls, the caller passes what it read. */
#ifndef NG_BTNGUARD_H
#define NG_BTNGUARD_H
#include <stdint.h>

#define NG_BTN_RELEASE_MS      120u
#define NG_BTN_RELEASE_SAMPLES 4u
#define NG_BTN_SAMPLE_MS       16u      /* samples closer than one Maple frame are the same status: counted once */

typedef struct {
    uint32_t pad;                   /* the pad of the last sample; 0 = none */
    uint32_t up_since;              /* when the current run of "up" samples began */
    uint32_t up_last;               /* the last sample counted in that run */
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
    if (!g->up_n) {
        g->up_since = g->up_last = now_ms;
        g->up_n = 1;
    } else if ((uint32_t)(now_ms - g->up_last) >= NG_BTN_SAMPLE_MS) {
        g->up_last = now_ms;
        if (g->up_n < 255) g->up_n++;
    }
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

/* A status no real pad produces: opposite directions, or all five face buttons.  KOS cooks buttons = ~raw, so an all-zero raw
   reply reads as every button pressed.  The platform layers report such a status as "no pad". */
static inline int ng_pad_implausible(uint32_t b, uint32_t up, uint32_t down, uint32_t left, uint32_t right, uint32_t face)
{
    return ((b & up) && (b & down)) || ((b & left) && (b & right)) || (b & face) == face;
}

/* "Hold it to approve", for anything destructive.  Zero it when the question appears: the button must then be seen released on
   this pad, the hold starts at the guarded press, and any release, pad change or missing pad starts over. */
typedef struct {
    ng_btnguard_t g;
    uint32_t since;
    uint8_t holding;
} ng_holdconfirm_t;

/* 1 = held for hold_ms */
static inline int ng_holdconfirm_update(ng_holdconfirm_t *c, uint32_t pad, int down, uint32_t now_ms, uint32_t hold_ms)
{
    if (pad != c->g.pad) c->holding = 0;
    if (!ng_btnguard_update(&c->g, pad, down, now_ms)) {
        c->holding = 0;
        return 0;
    }
    if (!c->holding) {
        c->holding = 1;
        c->since = now_ms;
    }
    return (uint32_t)(now_ms - c->since) >= hold_ms;
}

static inline uint32_t ng_holdconfirm_ms(const ng_holdconfirm_t *c, uint32_t now_ms)
{
    return c->holding ? (uint32_t)(now_ms - c->since) : 0;
}

#endif
