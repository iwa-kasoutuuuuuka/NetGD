#include <string.h>
#include "nm.h"
#include "nm_text.h"
#include "nm_ipbin.h"

/* the next blank-separated field of [*p, e); a "quoted name" may hold blanks.  Returns 0 at the end. */
static int field(const char **p, const char *e, const char **fb, const char **fe)
{
    while (*p < e && (**p == ' ' || **p == '\t')) (*p)++;
    if (*p >= e) return 0;
    if (**p == '"') {
        *fb = ++*p;
        while (*p < e && **p != '"') (*p)++;
        *fe = *p;
        if (*p < e) (*p)++;
        return 1;
    }
    *fb = *p;
    while (*p < e && **p != ' ' && **p != '\t') (*p)++;
    *fe = *p;
    return 1;
}

static int to_u(const char *b, const char *e, unsigned long *v)
{
    unsigned long x = 0;
    if (b == e || e - b > 9) return 0;
    for (; b < e; b++) {
        if (*b < '0' || *b > '9') return 0;
        x = x * 10 + (unsigned long)(*b - '0');
    }
    *v = x;
    return 1;
}

int nm_gdi_track3(const char *data, size_t n, char *fname, size_t fnamen, unsigned *sector_size)
{
    size_t i = 0;
    int lineno = 0, have = 0;
    char alt_name[NM_FOLDER_MAX + 1];
    unsigned alt_size = 0;
    int have_alt = 0;
    if (n >= 3 && (uint8_t)data[0] == 0xEF && (uint8_t)data[1] == 0xBB && (uint8_t)data[2] == 0xBF) { data += 3; n -= 3; }
    while (i < n) {
        const char *p = data + i, *e, *fb[6], *fe[6];
        unsigned long track, lba, ssz;
        int ok = 1;
        while (i < n && data[i] != '\n' && data[i] != '\r') i++;
        e = data + i;
        i++;
        if (lineno++ == 0) continue;                    /* the first line is the track count */
        for (int k = 0; k < 5 && ok; k++) ok = field(&p, e, &fb[k], &fe[k]);
        if (!ok) continue;                              /* a broken line is skipped */
        if (!to_u(fb[0], fe[0], &track) || !to_u(fb[1], fe[1], &lba) || !to_u(fb[3], fe[3], &ssz)) continue;
        if ((ssz != 2048 && ssz != 2352) || fe[4] == fb[4] || (size_t)(fe[4] - fb[4]) >= fnamen ||
            (size_t)(fe[4] - fb[4]) > NM_FOLDER_MAX)
            continue;
        if (track == 3) {
            memcpy(fname, fb[4], (size_t)(fe[4] - fb[4]));
            fname[fe[4] - fb[4]] = 0;
            *sector_size = (unsigned)ssz;
            have = 1;
            break;
        }
        if (lba == 45000 && !have_alt) {
            memcpy(alt_name, fb[4], (size_t)(fe[4] - fb[4]));
            alt_name[fe[4] - fb[4]] = 0;
            alt_size = (unsigned)ssz;
            have_alt = 1;
        }
    }
    if (!have && have_alt && strlen(alt_name) < fnamen) {
        strcpy(fname, alt_name);
        *sector_size = alt_size;
        have = 1;
    }
    return have;
}

int nm_ipbin_title(const uint8_t *sector, size_t n, unsigned secsize, char *out, size_t outn)
{
    size_t base = secsize == 2352 ? 16 : 0;
    if (outn) out[0] = 0;
    if ((secsize != 2048 && secsize != 2352) || n < base + 0x100) return 0;
    if (memcmp(sector + base, "SEGA SEGAKATANA ", 16)) return 0;
    nm_text_clean(out, outn, (const char *)sector + base + 0x80, 128, NM_TITLE_MAX);
    return outn && out[0] ? 1 : 0;
}
