#include "toc.h"
#include <string.h>

static uint32_t fad3(const uint8_t *p) { return ((uint32_t)p[0] << 16) | ((uint32_t)p[1] << 8) | p[2]; }

int gp_toc_parse(const uint8_t toc[408], gp_toc_area *a)
{
    int t;
    uint32_t lo;
    memset(a, 0, sizeof(*a));
    for (t = 0; t < 99; t++) {
        const uint8_t *e = toc + 4 * t;
        uint32_t f = fad3(e + 1);
        if (e[0] == 0xFF || f < 150 || f >= 0x00F00000)
            continue;
        a->num[a->n] = (uint8_t)(t + 1);
        a->ctl[a->n] = e[0];
        a->fad[a->n] = f;
        a->n++;
    }
    lo = fad3(toc + 405);
    if (toc[404] != 0xFF && lo && lo < 0x00F00000)
        a->leadout = lo;
    a->valid = a->n > 0;
    return a->valid;
}

typedef gp_track trk;

/* both areas' tracks in FAD order, without the duplicates a CD's copied area 1 adds */
static int merge(const gp_toc_area area[2], trk *out)
{
    int n = 0, s, i, j;
    for (s = 0; s < 2; s++) {
        if (!area[s].valid)
            continue;
        for (i = 0; i < area[s].n; i++) {
            uint32_t f = area[s].fad[i];
            for (j = 0; j < n && out[j].fad != f; j++)
                ;
            if (j < n || n >= 198)
                continue;
            for (j = n; j > 0 && out[j - 1].fad > f; j--)
                out[j] = out[j - 1];
            out[j].num = area[s].num[i];
            out[j].ctl = area[s].ctl[i];
            out[j].fad = f;
            n++;
        }
    }
    return n;
}

void gp_gap_plan_make(const gp_toc_area area[2], gp_gap_plan *p)
{
    trk t[198];
    int n = merge(area, t), i;
    memset(p, 0, sizeof(*p));
    p->trk_why = p->lo_why = p->hd_why = "-";

    /* (1) */
    if (n < 1) {
        p->trk_why = "no_toc";
    } else if (n < 2) {
        p->trk_why = "one_track";
    } else if (t[1].fad < t[0].fad + 2) {
        p->trk_why = "no_room";
    } else {
        p->trk_ok = 1;
        p->a_num = t[0].num; p->a_fad = t[0].fad; p->a_ctl = t[0].ctl;
        p->b_num = t[1].num; p->b_fad = t[1].fad; p->b_ctl = t[1].ctl;
    }

    /* (2) the highest lead-out of a valid area */
    for (i = 0; i < 2; i++)
        if (area[i].valid && area[i].leadout > p->leadout)
            p->leadout = area[i].leadout;
    for (i = 0; i < n; i++)
        if (t[i].fad < p->leadout && t[i].fad >= p->last_fad) {
            p->last_fad = t[i].fad;
            p->last_num = t[i].num;
        }
    if (!p->leadout)
        p->lo_why = "no_leadout";
    else if (!p->last_fad || p->leadout < p->last_fad + 2)
        p->lo_why = "no_track_before_leadout";
    else
        p->lo_ok = 1;

    /* (3) a high-density area that starts after the low-density lead-out (a CD's area 1 is a copy of area 0) */
    if (!area[0].valid || !area[1].valid) {
        p->hd_why = "no_hd_area";
    } else if (!area[0].leadout) {
        p->hd_why = "no_ld_leadout";
    } else {
        uint32_t h = 0, hn = 0;
        for (i = 0; i < area[1].n; i++)
            if (area[1].fad[i] > area[0].leadout && (!h || area[1].fad[i] < h)) {
                h = area[1].fad[i];
                hn = area[1].num[i];
            }
        if (!h) {
            p->hd_why = "no_hd_area";
        } else if (h < area[0].leadout + 2) {
            p->hd_why = "no_gap";
        } else {
            p->hd_ok = 1;
            p->ld_leadout = area[0].leadout;
            p->hd_fad = h;
            p->hd_num = hn;
        }
    }

    /* reference: first data track + 16 (inside it), else first track + 16 */
    for (i = 0; i < n && !(t[i].ctl & 0x40); i++)
        ;
    if (i == n)
        i = 0;
    if (n) {
        uint32_t end = (i + 1 < n) ? t[i + 1].fad : p->leadout;
        p->ref_fad = (end > t[i].fad + 17) ? t[i].fad + 16 : t[i].fad;
    }
}

uint32_t gp_bsearch_next(uint32_t lo, uint32_t hi)
{
    if (hi <= lo + 1)
        return 0;
    return lo + (hi - lo) / 2;
}

int gp_toc_tracks(const gp_toc_area area[2], gp_track *out)
{
    return merge(area, out);
}

int gp_offset_fads(uint32_t start, const int *offs, int n, uint32_t *out)
{
    int i, m = 0;
    for (i = 0; i < n; i++) {
        long f = (long)start + offs[i];
        if (f >= 0 && f < 0x00F00000L)
            out[m++] = (uint32_t)f;
    }
    return m;
}
