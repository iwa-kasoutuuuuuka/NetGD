#include <stdio.h>
#include <string.h>
#include "nm_art.h"

static uint32_t le16(const uint8_t *p) { return (uint32_t)p[0] | (uint32_t)p[1] << 8; }

int nm_art_check_header(const uint8_t *h)
{
    if (memcmp(h, "NGAR", 4)) return -1;
    if (le16(h + 4) != 1) return -1;
    if (le16(h + 6) != NM_ART_PW || le16(h + 8) != NM_ART_PH) return -1;
    return 0;
}

void nm_art_init(nm_art_t *a)
{
    memset(a, 0, sizeof *a);
}

void nm_art_cancel(nm_art_t *a)
{
    if (a->open) f_close(&a->f);
    a->open = 0;
    a->state = NM_ART_NONE;
    a->folder[0] = 0;
    a->got = 0;
}

void nm_art_select(nm_art_t *a, const char *folder, uint32_t now)
{
    if (folder && !strcmp(folder, a->folder) && a->state != NM_ART_NONE) return;
    if (!folder && a->state == NM_ART_NONE) return;
    nm_art_cancel(a);
    if (!folder || !folder[0]) return;
    snprintf(a->folder, sizeof a->folder, "%s", folder);
    a->since = now;
    a->state = NM_ART_WAIT;
}

static void fail(nm_art_t *a)
{
    if (a->open) f_close(&a->f);
    a->open = 0;
    a->state = NM_ART_FAILED;
}

void nm_art_step(nm_art_t *a, uint32_t now)
{
    static uint8_t buf[NM_ART_CHUNK];
    UINT br = 0;
    if (a->state == NM_ART_WAIT) {
        char path[NM_FOLDER_MAX + 16];
        uint8_t hdr[NM_ART_HDR];
        if ((int32_t)(now - a->since) < NM_ART_DEBOUNCE_MS) return;
        snprintf(path, sizeof path, "0:/%s/NETGD.ART", a->folder);
        if (f_open(&a->f, path, FA_READ) != FR_OK) { a->state = NM_ART_FAILED; return; }
        a->open = 1;
        if (f_size(&a->f) < NM_ART_HDR + NM_ART_BYTES || f_read(&a->f, hdr, sizeof hdr, &br) != FR_OK || br != sizeof hdr ||
            nm_art_check_header(hdr) != 0) { fail(a); return; }
        a->got = 0;
        a->state = NM_ART_LOADING;
        return;
    }
    if (a->state == NM_ART_LOADING) {
        uint32_t n = NM_ART_BYTES - a->got;
        if (n > NM_ART_CHUNK) n = NM_ART_CHUNK;
        if (f_read(&a->f, buf, n, &br) != FR_OK || br != n) { fail(a); return; }
        for (uint32_t i = 0; i < n; i += 2) a->px[(a->got + i) / 2] = (uint16_t)(buf[i] | buf[i + 1] << 8);
        a->got += n;
        if (a->got >= NM_ART_BYTES) {
            f_close(&a->f);
            a->open = 0;
            a->state = NM_ART_READY;
        }
    }
}

const uint16_t *nm_art_pixels(const nm_art_t *a)
{
    return a->state == NM_ART_READY ? a->px : NULL;
}
