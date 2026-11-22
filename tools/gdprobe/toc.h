/* gdprobe: GET_TOC parsing and the seq 22 gap-probe FAD plan. Pure C (host tested).
 *
 * A GET_TOC area is 408 bytes: 99 entries of 4 bytes (byte 0 = CTRL << 4 | ADR, bytes 1..3 = FAD, 0xFF.. = no
 * track), then first track (396), last track (400) and the lead-out (404: CTRL/ADR, 405..407 = FAD).
 */
#ifndef GDPROBE_TOC_H
#define GDPROBE_TOC_H
#include <stdint.h>

typedef struct {
    int valid;            /* the area was read and lists at least one track */
    int n;                /* tracks, in TOC order */
    uint8_t num[99];      /* track number (entry index + 1) */
    uint8_t ctl[99];      /* CTRL/ADR byte */
    uint32_t fad[99];
    uint32_t leadout;     /* 0 = none */
} gp_toc_area;

/* Parse one area. Returns a->valid. */
int gp_toc_parse(const uint8_t toc[408], gp_toc_area *a);

/* The FADs the seq 22 gap probe uses, from the TOC of both areas (area 1 may be invalid, or a copy of area 0 on a CD).
 * A field is 0 when its case does not apply; the matching *_why says why (a short token for the record). */
typedef struct {
    /* (1) first track A and the next track B (in FAD order over both areas); the gap is searched in [a_fad, b_fad) */
    int trk_ok;
    uint32_t a_num, a_fad, a_ctl, b_num, b_fad, b_ctl;
    const char *trk_why;
    /* (2) lead-out of the last area (the highest valid one) and the start of the track before it */
    int lo_ok;
    uint32_t leadout, last_fad, last_num;
    const char *lo_why;
    /* (3) GD-ROM: the low-density area's lead-out and the high-density area's first track */
    int hd_ok;
    uint32_t ld_leadout, hd_fad, hd_num;
    const char *hd_why;
    /* a readable reference FAD for the cases that need one (first data track + 16, else first track + 16) */
    uint32_t ref_fad;
} gp_gap_plan;

void gp_gap_plan_make(const gp_toc_area area[2], gp_gap_plan *p);

/* One step of the search for the last readable sector in [lo, hi] (lo known readable, hi known unreadable):
 * returns the FAD to try next, or 0 when hi == lo + 1 (done: lo is the last readable one). */
uint32_t gp_bsearch_next(uint32_t lo, uint32_t hi);

#endif
