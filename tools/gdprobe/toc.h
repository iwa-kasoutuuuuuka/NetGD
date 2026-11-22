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
    uint8_t leadout_ctl;  /* seq 24: CTRL/ADR byte of the lead-out entry */
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

/* seq 23: both areas' tracks in FAD order without a CD's duplicated area 1; out needs 198 entries. Returns the count. */
typedef struct { uint32_t num, ctl, fad; } gp_track;
int gp_toc_tracks(const gp_toc_area area[2], gp_track *out);

/* seq 23: the FADs start + offs[i] that are >= 0, in order; out needs n entries. Returns the count. */
int gp_offset_fads(uint32_t start, const int *offs, int n, uint32_t *out);

/* One step of the search for the last readable sector in [lo, hi] (lo known readable, hi known unreadable):
 * returns the FAD to try next, or 0 when hi == lo + 1 (done: lo is the last readable one). */
uint32_t gp_bsearch_next(uint32_t lo, uint32_t hi);

/* seq 24: off-track regions for the CTRL case. For each: the track before it (prev, read at in_fad), the track after
 * it (next, read at next_in; for the lead-out none: next_ctl = the lead-out entry's CTRL, next_in = first track + 16)
 * and up to 3 off-track FADs.
 *   GAP  between track 1 and track 2 (CD): off = track 2 start - 151 (below its pregap)
 *   HDG  GD-ROM: LD lead-out .. HD pregap: off = LD lead-out, the middle, HD start - 151
 *   LO   lead-out of the last area: off = lead-out + 1 */
typedef struct {
    const char *name;
    uint32_t prev_num, prev_ctl, in_fad;
    int has_next;
    uint32_t next_num, next_ctl, next_in;
    uint32_t rev_num, rev_ctl, rev_in;   /* the reverse order's first read: next track, or first track (LO) */
    int n_off;
    uint32_t off[3];
} gp_ctl_region;
int gp_ctl_regions(const gp_toc_area area[2], const gp_gap_plan *p, gp_ctl_region out[3]);

#endif
