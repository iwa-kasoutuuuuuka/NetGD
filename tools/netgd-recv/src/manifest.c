#include <stdio.h>
#include <string.h>
#include <stdarg.h>
#include "manifest.h"
#include "crc32.h"

static void seterr(char *err, size_t n, const char *fmt, ...)
{
    va_list ap;
    if (!err || !n) return;
    va_start(ap, fmt);
    vsnprintf(err, n, fmt, ap);
    va_end(ap);
}

static int ieq(const char *a, const char *b)
{
    for (; *a && *b; a++, b++) {
        char x = *a, y = *b;
        if (x >= 'a' && x <= 'z') x -= 32;
        if (y >= 'a' && y <= 'z') y -= 32;
        if (x != y) return 0;
    }
    return *a == *b;
}

/* one path component: printable ASCII, FAT-safe */
static int name_ok(const char *s, char *err, size_t en, const char *what)
{
    size_t n = strlen(s);
    if (n == 0) { seterr(err, en, "%s is empty", what); return -1; }
    if (n > NG_NAME_MAX) { seterr(err, en, "%s too long", what); return -1; }
    for (size_t i = 0; i < n; i++) {
        unsigned char c = (unsigned char)s[i];
        if (c < 0x20 || c > 0x7E) { seterr(err, en, "%s '%s' has a non-ASCII or control character", what, s); return -1; }
        if (strchr("\\/:*?\"<>|", c)) { seterr(err, en, "%s '%s' has a forbidden character", what, s); return -1; }
    }
    if (s[0] == ' ' || s[n - 1] == ' ' || s[n - 1] == '.' || !strcmp(s, ".") || !strcmp(s, "..")) {
        seterr(err, en, "%s '%s' has a leading/trailing space or dot", what, s);
        return -1;
    }
    return 0;
}

int ng_path_ok(const char *path, char *err, size_t en)
{
    char tmp[NG_PATH_MAX + 1];
    int parts = 0;
    size_t n = strlen(path);
    if (n == 0 || n > NG_PATH_MAX) { seterr(err, en, "image path empty or longer than %d", NG_PATH_MAX); return -1; }
    memcpy(tmp, path, n + 1);
    for (char *p = tmp, *q;; p = q + 1) {
        q = strchr(p, '/');
        if (q) *q = 0;
        if (++parts > NG_PATH_PARTS) { seterr(err, en, "image path has more than %d levels", NG_PATH_PARTS); return -1; }
        if (name_ok(p, err, en, "image folder")) return -1;
        if (!q) break;
    }
    return 0;
}

/* The descriptor base name must be a plain 8.3 name so the .NGP entry carries no LFN
   entries (their checksum would depend on the extension).  Letters, digits and the
   FAT-safe punctuation; the name is upper-cased when it is created. */
static int descriptor_ok(const char *name, char *err, size_t en)
{
    const char *dot = strrchr(name, '.');
    size_t base = dot ? (size_t)(dot - name) : strlen(name);
    if (!dot || base == 0 || base > 8 || strlen(dot + 1) != 3) {
        seterr(err, en, "descriptor '%s' is not an 8.3 name", name);
        return -1;
    }
    for (size_t i = 0; i < base; i++) {
        char c = name[i];
        int ok = (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') ||
                 (c && strchr("$%'-_@~!(){}^#&", c));
        if (!ok) { seterr(err, en, "descriptor '%s': character not allowed in an 8.3 base name", name); return -1; }
    }
    return 0;
}

static int parse_u32(const char *s, size_t n, uint32_t *out)
{
    uint64_t v = 0;
    if (!n || n > 10) return -1;
    for (size_t i = 0; i < n; i++) {
        if (s[i] < '0' || s[i] > '9') return -1;
        v = v * 10 + (uint64_t)(s[i] - '0');
    }
    if (v > 0xFFFFFFFFull) return -1;
    *out = (uint32_t)v;
    return 0;
}

static int parse_hex8(const char *s, uint32_t *out)
{
    uint32_t v = 0;
    for (int i = 0; i < 8; i++) {
        char c = s[i];
        int d;
        if (c >= '0' && c <= '9') d = c - '0';
        else if (c >= 'A' && c <= 'F') d = c - 'A' + 10;
        else if (c >= 'a' && c <= 'f') d = c - 'a' + 10;
        else return -1;
        v = (v << 4) | (uint32_t)d;
    }
    *out = v;
    return 0;
}

int ng_manifest_parse(const char *text, size_t len, ng_manifest_t *m, char *err, size_t errn)
{
    const char *p = text, *end = text + len;
    int have_image = 0, have_type = 0, have_desc = 0, ended = 0, lineno = 0;
    char type[8] = "";
    char desc[NG_NAME_MAX + 1] = "";

    memset(m, 0, sizeof *m);
    m->crc = ng_crc32(0, text, len);
    m->descriptor = -1;
    if (len == 0 || memchr(text, '\r', len) || memchr(text, 0, len)) {
        seterr(err, errn, "manifest empty, or has CR/NUL (must be LF-only text)");
        return -1;
    }
    while (p < end) {
        const char *nl = memchr(p, '\n', (size_t)(end - p));
        size_t ll = nl ? (size_t)(nl - p) : (size_t)(end - p);
        char line[NG_NAME_MAX + 64];
        char *rest;
        lineno++;
        if (ll >= sizeof line) { seterr(err, errn, "line %d too long", lineno); return -1; }
        memcpy(line, p, ll);
        line[ll] = 0;
        p = nl ? nl + 1 : end;

        if (lineno == 1) {
            if (strcmp(line, "NETGD-MANIFEST 1")) { seterr(err, errn, "bad header or unsupported version: '%s'", line); return -1; }
            continue;
        }
        if (ended) { seterr(err, errn, "data after 'end'"); return -1; }
        if (!strcmp(line, "end")) { ended = 1; continue; }
        rest = strchr(line, ' ');
        if (!rest) { seterr(err, errn, "line %d: unknown line", lineno); return -1; }
        *rest++ = 0;
        if (!strcmp(line, "image")) {
            if (strlen(rest) > NG_PATH_MAX) { seterr(err, errn, "image path too long"); return -1; }
            strcpy(m->image, rest);
            have_image = 1;
        } else if (!strcmp(line, "type")) {
            if (strlen(rest) > 3) { seterr(err, errn, "bad type"); return -1; }
            strcpy(type, rest);
            have_type = 1;
        } else if (!strcmp(line, "descriptor")) {
            if (strlen(rest) > NG_NAME_MAX) { seterr(err, errn, "descriptor too long"); return -1; }
            strcpy(desc, rest);
            have_desc = 1;
        } else if (!strcmp(line, "file")) {
            const char *sp1 = strchr(rest, ' ');
            ng_mfile_t *f;
            if (m->nfiles >= NG_MAX_FILES) { seterr(err, errn, "more than %d files", NG_MAX_FILES); return -1; }
            f = &m->files[m->nfiles];
            /* "<size> <8 hex> <name>" */
            if (!sp1 || strlen(sp1 + 1) < 10 || sp1[9] != ' ' || parse_u32(rest, (size_t)(sp1 - rest), &f->size) ||
                parse_hex8(sp1 + 1, &f->crc)) {
                seterr(err, errn, "line %d: bad file line", lineno);
                return -1;
            }
            if (strlen(sp1 + 10) > NG_NAME_MAX) { seterr(err, errn, "file name too long"); return -1; }
            strcpy(f->name, sp1 + 10);
            if (f->size > 0xFFFFFFFEu) { seterr(err, errn, "file %s: size beyond FAT32 limit", f->name); return -1; }
            m->nfiles++;
        } else {
            seterr(err, errn, "line %d: unknown line", lineno);
            return -1;
        }
    }
    if (!ended) { seterr(err, errn, "manifest is incomplete (no 'end')"); return -1; }
    if (!have_image || !have_type || !have_desc) { seterr(err, errn, "manifest lacks image/type/descriptor"); return -1; }
    if (!strcmp(type, "gdi")) m->is_cdi = 0;
    else if (!strcmp(type, "cdi")) m->is_cdi = 1;
    else { seterr(err, errn, "type must be gdi or cdi"); return -1; }
    if (ng_path_ok(m->image, err, errn)) return -1;
    if (m->nfiles < 1) { seterr(err, errn, "no files"); return -1; }
    for (int i = 0; i < m->nfiles; i++) {
        if (name_ok(m->files[i].name, err, errn, "file name")) return -1;
        for (int j = 0; j < i; j++)
            if (ieq(m->files[i].name, m->files[j].name)) {
                seterr(err, errn, "duplicate file name %s", m->files[i].name);
                return -1;
            }
        m->total_bytes += m->files[i].size;
        if (ieq(m->files[i].name, desc)) m->descriptor = i;
        if (ieq(m->files[i].name, "NETGD.MF") || ieq(m->files[i].name, "NETGD.CKP")) {
            seterr(err, errn, "file name %s is reserved", m->files[i].name);
            return -1;
        }
    }
    if (m->descriptor < 0) { seterr(err, errn, "descriptor %s is not one of the file lines", desc); return -1; }
    {
        const char *dot = strrchr(desc, '.');
        if (!dot || !ieq(dot + 1, m->is_cdi ? "cdi" : "gdi")) {
            seterr(err, errn, "descriptor %s does not end in .%s", desc, m->is_cdi ? "cdi" : "gdi");
            return -1;
        }
    }
    if (descriptor_ok(m->files[m->descriptor].name, err, errn)) return -1;
    {
        /* no data file may carry the name of the descriptor's .NGP placeholder (it would be the same
           directory entry while the image is being written) */
        char ngp[16], fin[16];
        ng_manifest_ngp_name(m, ngp, fin);
        for (int i = 0; i < m->nfiles; i++)
            if (i != m->descriptor && ieq(m->files[i].name, ngp)) {
                seterr(err, errn, "file name %s collides with the descriptor placeholder", m->files[i].name);
                return -1;
            }
    }
    return 0;
}

void ng_manifest_ngp_name(const ng_manifest_t *m, char *ngp, char *final)
{
    const char *name = m->files[m->descriptor].name;
    const char *dot = strrchr(name, '.');
    size_t base = (size_t)(dot - name), i;
    for (i = 0; i < base; i++) {
        char c = name[i];
        if (c >= 'a' && c <= 'z') c -= 32;
        ngp[i] = final[i] = c;
    }
    strcpy(ngp + base, ".NGP");
    strcpy(final + base, m->is_cdi ? ".CDI" : ".GDI");
}
