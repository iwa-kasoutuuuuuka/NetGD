#include <string.h>
#include "nm.h"
#include "nm_text.h"
#include "nm_cfgparse.h"

static int is_ws(char c) { return c == ' ' || (c >= '\t' && c <= '\r'); }

/* a [b, e) trimmed of ASCII white space */
static void trim(const char **b, const char **e)
{
    while (*b < *e && is_ws(**b)) (*b)++;
    while (*e > *b && is_ws((*e)[-1])) (*e)--;
}

static int ieq(const char *b, const char *e, const char *lit)
{
    size_t n = strlen(lit);
    if ((size_t)(e - b) != n) return 0;
    for (size_t i = 0; i < n; i++) {
        char c = b[i];
        if (c >= 'A' && c <= 'Z') c = (char)(c + 32);
        if (c != lit[i]) return 0;
    }
    return 1;
}

/* a decimal integer, optionally signed or quoted (ini.rs num); 0 = not a number */
static int ini_num(const char *b, const char *e, int *out)
{
    int neg = 0;
    long v = 0;
    trim(&b, &e);
    if (e - b >= 2 && *b == '"' && e[-1] == '"') { b++; e--; }       /* trim_matches('"') */
    trim(&b, &e);
    if (b < e && *b == '-') { neg = 1; b++; }
    else if (b < e && *b == '+') b++;
    if (b == e || e - b > 9) return 0;
    for (const char *p = b; p < e; p++) {
        if (*p < '0' || *p > '9') return 0;
        v = v * 10 + (*p - '0');
    }
    *out = (int)(neg ? -v : v);
    return 1;
}

static uint16_t clamp_time(int v) { return (uint16_t)(v < 150 ? 150 : v > 5000 ? 5000 : v); }

void nm_ini_defaults(nm_ini_t *ini)
{
    memset(ini, 0, sizeof *ini);
    ini->open_time = 500;
    ini->detect_time = 250;
}

static void ini_line(nm_ini_t *ini, const char *b, const char *e, int first)
{
    const char *eq, *kb, *ke, *vb, *ve;
    int v;
    if (first && e - b >= 3 && (uint8_t)b[0] == 0xEF && (uint8_t)b[1] == 0xBB && (uint8_t)b[2] == 0xBF) b += 3;
    for (const char *p = b; p < e; p++)                     /* the settings part ends at # or ; */
        if (*p == '#' || *p == ';') { e = p; break; }
    for (const char *p = b; p < e; p++)
        if ((uint8_t)*p >= 0x80) return;                    /* a non-ASCII setting: this line only is dropped */
    trim(&b, &e);
    if (b == e || *b == '[') return;
    eq = memchr(b, '=', (size_t)(e - b));
    if (!eq) return;
    kb = b; ke = eq; vb = eq + 1; ve = e;
    trim(&kb, &ke);
    trim(&vb, &ve);
    if (ieq(kb, ke, "open_time")) {
        if (ini_num(vb, ve, &v)) { ini->open_time = clamp_time(v); ini->open_time_set = 1; }
    } else if (ieq(kb, ke, "detect_time")) {
        if (ini_num(vb, ve, &v)) { ini->detect_time = clamp_time(v); ini->detect_time_set = 1; }
    } else if (ieq(kb, ke, "reset_goto")) {
        if (ini_num(vb, ve, &v)) ini->reset_goto = (uint16_t)((v >= 0 && v <= 9999) ? v : 0);
    }
}

void nm_ini_parse(nm_ini_t *ini, const char *data, size_t n)
{
    size_t i = 0;
    int first = 1;
    nm_ini_defaults(ini);
    while (i <= n) {
        size_t s = i;
        while (i < n && data[i] != '\r' && data[i] != '\n') i++;
        ini_line(ini, data + s, data + i, first);
        first = 0;
        i++;                    /* CR, LF and CRLF each just end a line (an empty line in between is harmless) */
    }
}

uint32_t nm_ini_ready_limit_ms(const nm_ini_t *ini)
{
    return 10000u + (ini->open_time_set ? ini->open_time : 0u) + (ini->detect_time_set ? ini->detect_time : 0u);
}

/* ---- GAME.CFG / NAME.TXT ---- */

/* the line [b,e) without a trailing '#' comment (a '#' inside quotes does not count) */
static const char *strip_comment(const char *b, const char *e)
{
    int q = 0;
    for (const char *p = b; p < e; p++) {
        if (*p == '"') q = !q;
        else if (*p == '#' && !q) return p;
    }
    return e;
}

int nm_gamecfg_title(const char *data, size_t n, char *out, size_t outn)
{
    size_t i = 0;
    if (outn) out[0] = 0;
    if (n >= 3 && (uint8_t)data[0] == 0xEF && (uint8_t)data[1] == 0xBB && (uint8_t)data[2] == 0xBF) { data += 3; n -= 3; }
    while (i < n) {
        const char *b = data + i, *e, *eq, *kb, *ke, *vb, *ve;
        size_t s = i;
        while (i < n && data[i] != '\n') i++;
        e = data + i;
        i++;
        (void)s;
        e = strip_comment(b, e);
        trim(&b, &e);
        if (b == e || *b == '[') continue;
        eq = memchr(b, '=', (size_t)(e - b));
        if (!eq) continue;
        kb = b; ke = eq; vb = eq + 1; ve = e;
        trim(&kb, &ke);
        trim(&vb, &ve);
        if ((size_t)(ke - kb) != 5 || memcmp(kb, "title", 5)) continue;
        if (ve - vb >= 2 && *vb == '"' && ve[-1] == '"') { vb++; ve--; }
        else if (vb == ve || *vb == '"') continue;              /* an unterminated quote is a typo, not a value */
        /* a later `title` line replaces an earlier one, as in the Rust parser (the last one wins) */
        if (nm_text_clean(out, outn, vb, (size_t)(ve - vb), NM_TITLE_MAX) == 0 && outn) out[0] = 0;
    }
    return outn && out[0] ? 1 : 0;
}

int nm_nametxt_title(const char *data, size_t n, char *out, size_t outn)
{
    size_t i = 0;
    if (outn) out[0] = 0;
    if (n >= 3 && (uint8_t)data[0] == 0xEF && (uint8_t)data[1] == 0xBB && (uint8_t)data[2] == 0xBF) { data += 3; n -= 3; }
    while (i < n && data[i] != '\r' && data[i] != '\n') i++;
    nm_text_clean(out, outn, data, i, NM_TITLE_MAX);
    return outn && out[0] ? 1 : 0;
}
