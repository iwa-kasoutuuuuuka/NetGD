#include "cfg.h"
#include <ctype.h>
#include <stdlib.h>
#include <string.h>

void gp_config_defaults(gp_config *c)
{
    memset(c, 0, sizeof(*c));
    strcpy(c->label, "unknown");
    c->switch_timeout_ms = 15000;
    c->do_throughput = 1;
    c->tp_batches = 64;
    c->tp_sectors = 16;
    c->do_cdread_flags = 1;
    c->do_seek = 1;
    c->seek_n = 16;
    c->seek_span = 50000;
    c->cdread_flags = 0x24;
    c->timeout_ms = 3000;
}

static int parse_bool(const char *v, int *out)
{
    if (!strcmp(v, "on") || !strcmp(v, "1") || !strcmp(v, "yes") || !strcmp(v, "true")) { *out = 1; return 1; }
    if (!strcmp(v, "off") || !strcmp(v, "0") || !strcmp(v, "no") || !strcmp(v, "false")) { *out = 0; return 1; }
    return 0;
}

static long parse_num(const char *v, int *ok)
{
    char *end;
    long n = strtol(v, &end, 0);
    *ok = (end != v && *end == 0);
    return n;
}

static int clampi(long v, long lo, long hi) { return (int)(v < lo ? lo : v > hi ? hi : v); }

static void trim(char *s);
static void parse_images(gp_config *c, char *v)
{
    char *tok = v, *next;
    c->n_images = 0;
    while (tok && *tok) {
        int a, b, ok;
        char *dash;
        next = strchr(tok, ',');
        if (next) *next++ = 0;
        trim(tok);
        while (isspace((unsigned char)*tok)) tok++;
        dash = strchr(tok, '-');
        if (dash) {
            *dash = 0;
            trim(tok);
            a = (int)parse_num(tok, &ok); if (!ok) { c->unknown_keys++; return; }
            b = (int)parse_num(dash + 1, &ok); if (!ok) { c->unknown_keys++; return; }
        } else {
            a = b = (int)parse_num(tok, &ok); if (!ok) { c->unknown_keys++; return; }
        }
        /* folder numbers 1..999 only (what `52 82` accepts) */
        for (; a <= b && a >= 1 && a <= 999 && c->n_images < GP_MAX_IMAGES; a++)
            c->images[c->n_images++] = a;
        tok = next;
    }
}

static void trim(char *s)
{
    size_t n = strlen(s);
    while (n && isspace((unsigned char)s[n - 1])) s[--n] = 0;
}

static void apply(gp_config *c, const char *k, char *v)
{
    int ok = 1, b = 0;
    long n;
    if (!strcmp(k, "label")) {
        size_t i = 0;
        for (; *v && i < sizeof(c->label) - 1; v++)
            if (isalnum((unsigned char)*v) || *v == '_' || *v == '-' || *v == '.')
                c->label[i++] = *v;
        c->label[i] = 0;
        if (!i) strcpy(c->label, "unknown");
    } else if (!strcmp(k, "seq")) { n = parse_num(v, &ok); if (ok) c->seq = clampi(n, 0, 99999);
    } else if (!strcmp(k, "switch")) { if (parse_bool(v, &b)) c->do_switch = b; else ok = 0;
    } else if (!strcmp(k, "images")) { parse_images(c, v);
    } else if (!strcmp(k, "return_to")) { n = parse_num(v, &ok); if (ok) c->return_to = (n >= 1 && n <= 999) ? (int)n : 0;
    } else if (!strcmp(k, "switch_timeout_ms")) { n = parse_num(v, &ok); if (ok) c->switch_timeout_ms = (uint32_t)clampi(n, 1000, 60000);
    } else if (!strcmp(k, "identify")) { if (parse_bool(v, &b)) c->do_identify = b; else ok = 0;
    } else if (!strcmp(k, "throughput")) { if (parse_bool(v, &b)) c->do_throughput = b; else ok = 0;
    } else if (!strcmp(k, "tp_batches")) { n = parse_num(v, &ok); if (ok) c->tp_batches = clampi(n, 1, 256);
    } else if (!strcmp(k, "tp_sectors")) { n = parse_num(v, &ok); if (ok) c->tp_sectors = clampi(n, 1, 16);
    } else if (!strcmp(k, "seek")) { if (parse_bool(v, &b)) c->do_seek = b; else ok = 0;
    } else if (!strcmp(k, "seek_n")) { n = parse_num(v, &ok); if (ok) c->seek_n = clampi(n, 2, 64);
    } else if (!strcmp(k, "seek_span")) { n = parse_num(v, &ok); if (ok) c->seek_span = (uint32_t)clampi(n, 16, 1000000);
    } else if (!strcmp(k, "ip_fad")) { n = parse_num(v, &ok); if (ok) c->ip_fad = (uint32_t)clampi(n, 0, 0xFFFFFF);
    } else if (!strcmp(k, "cdread_flags_test")) { if (parse_bool(v, &b)) c->do_cdread_flags = b; else ok = 0;
    } else if (!strcmp(k, "cdread_flags")) { n = parse_num(v, &ok); if (ok) c->cdread_flags = (uint8_t)(n & 0xFE);
    } else if (!strcmp(k, "timeout_ms")) { n = parse_num(v, &ok); if (ok) c->timeout_ms = (uint32_t)clampi(n, 100, 30000);
    } else ok = 0;
    if (!ok) c->unknown_keys++;
}

void gp_config_parse(gp_config *c, const char *text)
{
    char line[256];
    const char *p = text;
    while (*p) {
        size_t n = 0;
        char *eq, *k, *v, *h;
        while (*p && *p != '\n' && n < sizeof(line) - 1) line[n++] = *p++;
        while (*p && *p != '\n') p++;   /* drop the rest of an overlong line */
        if (*p == '\n') p++;
        line[n] = 0;
        h = strchr(line, '#');
        if (h) *h = 0;
        k = line;
        while (isspace((unsigned char)*k)) k++;
        if (!*k) continue;
        eq = strchr(k, '=');
        if (!eq) { c->unknown_keys++; continue; }
        *eq = 0;
        v = eq + 1;
        trim(k);
        while (isspace((unsigned char)*v)) v++;
        trim(v);
        apply(c, k, v);
    }
}
