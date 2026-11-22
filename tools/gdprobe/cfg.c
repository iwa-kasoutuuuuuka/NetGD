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
    c->swap_regpoll_ms = 1000;
    c->swap_regpoll_interval_ms = 2;
    c->devreset_ms[0] = 50;
    c->devreset_ms[1] = 200;
    c->n_devreset = 2;
    c->swap_reswitch_wait_ms = 1000;
    c->devreset_only_wait_ms = 1000;
    c->devreset_cmd_ms[0] = 10;
    c->devreset_cmd_ms[1] = 50;
    c->devreset_cmd_ms[2] = 100;
    c->n_devreset_cmd = 3;
    c->head_flags = 0x20;
    c->head_counts[0] = 1;
    c->head_counts[1] = 2;
    c->head_counts[2] = 7;
    c->head_counts[3] = 16;
    c->n_head_counts = 4;
    c->head_late_ms = 300;
    c->head_idle_ms = 2000;
    c->head_scd = 1;
    c->gap_flags = 0x20;
    c->gap_late_ms = 300;
    c->gap_scd = 1;
    c->gap_leadout[0] = 0;
    c->gap_leadout[1] = 1;
    c->gap_leadout[2] = 100;
    c->n_gap_leadout = 3;
    c->gap_low[0] = 0;
    c->gap_low[1] = 100;
    c->n_gap_low = 2;
    c->gap_count0 = 1;
    {
        static const int lo[4] = { 0, 1, 100, 149 }, of[8] = { -156, -151, -150, -149, -100, -2, -1, 0 };
        memcpy(c->pregap_low, lo, sizeof(lo));
        c->n_pregap_low = 4;
        memcpy(c->pregap_offs, of, sizeof(of));
        c->n_pregap_offs = 8;
    }
    c->pregap_cross = 16;
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

/* "50,200": up to max_n numbers in lo..hi. Any bad token rejects the whole value (the previous list stays).
 * Returns 1 if parsed. */
static int parse_list(int *dst, int *n_dst, char *v, int max_n, long lo, long hi)
{
    int tmp[16], n = 0;
    char *tok = v, *next;
    while (tok) {
        int ok;
        long d;
        next = strchr(tok, ',');
        if (next) *next++ = 0;
        while (isspace((unsigned char)*tok)) tok++;
        trim(tok);
        d = parse_num(tok, &ok);
        if (!ok || d < lo || d > hi || n >= max_n || n >= 16)
            return 0;
        tmp[n++] = (int)d;
        tok = next;
    }
    if (!n)
        return 0;
    memcpy(dst, tmp, sizeof(tmp[0]) * (size_t)n);
    *n_dst = n;
    return 1;
}

/* delay lists: up to GP_MAX_DEVRESET values of 0..5000 ms */
static int parse_delays(int *dst, int *n_dst, char *v) { return parse_list(dst, n_dst, v, GP_MAX_DEVRESET, 0, 5000); }

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
    } else if (!strcmp(k, "swap_regpoll")) { if (parse_bool(v, &b)) c->swap_regpoll = b; else ok = 0;
    } else if (!strcmp(k, "swap_regpoll_ms")) { n = parse_num(v, &ok); if (ok) c->swap_regpoll_ms = (uint32_t)clampi(n, 100, 10000);
    } else if (!strcmp(k, "swap_regpoll_interval_ms")) { n = parse_num(v, &ok); if (ok) c->swap_regpoll_interval_ms = (uint32_t)clampi(n, 1, 50);
    } else if (!strcmp(k, "swap_devreset")) { if (parse_bool(v, &b)) c->swap_devreset = b; else ok = 0;
    } else if (!strcmp(k, "swap_devreset_ms")) { ok = parse_delays(c->devreset_ms, &c->n_devreset, v);
    } else if (!strcmp(k, "swap_reswitch")) { if (parse_bool(v, &b)) c->swap_reswitch = b; else ok = 0;
    } else if (!strcmp(k, "swap_reswitch_wait_ms")) { n = parse_num(v, &ok); if (ok) c->swap_reswitch_wait_ms = (uint32_t)clampi(n, 100, 10000);
    } else if (!strcmp(k, "devreset_only")) { if (parse_bool(v, &b)) c->devreset_only = b; else ok = 0;
    } else if (!strcmp(k, "devreset_only_wait_ms")) { n = parse_num(v, &ok); if (ok) c->devreset_only_wait_ms = (uint32_t)clampi(n, 100, 10000);
    } else if (!strcmp(k, "devreset_cmd")) { if (parse_bool(v, &b)) c->devreset_cmd = b; else ok = 0;
    } else if (!strcmp(k, "devreset_cmd_ms")) { ok = parse_delays(c->devreset_cmd_ms, &c->n_devreset_cmd, v);
    } else if (!strcmp(k, "head_probe")) { if (parse_bool(v, &b)) c->head_probe = b; else ok = 0;
    } else if (!strcmp(k, "head_flags")) { n = parse_num(v, &ok); if (ok) c->head_flags = (uint8_t)(n & 0xFE);
    } else if (!strcmp(k, "head_counts")) { ok = parse_list(c->head_counts, &c->n_head_counts, v, GP_MAX_HEADCNT, 1, 16);
    } else if (!strcmp(k, "head_late_ms")) { n = parse_num(v, &ok); if (ok) c->head_late_ms = (uint32_t)clampi(n, 0, 5000);
    } else if (!strcmp(k, "head_idle_ms")) { n = parse_num(v, &ok); if (ok) c->head_idle_ms = (uint32_t)clampi(n, 0, 10000);
    } else if (!strcmp(k, "head_scd")) { if (parse_bool(v, &b)) c->head_scd = b; else ok = 0;
    } else if (!strcmp(k, "gap_probe")) { if (parse_bool(v, &b)) c->gap_probe = b; else ok = 0;
    } else if (!strcmp(k, "gap_flags")) { n = parse_num(v, &ok); if (ok) c->gap_flags = (uint8_t)(n & 0xFE);
    } else if (!strcmp(k, "gap_late_ms")) { n = parse_num(v, &ok); if (ok) c->gap_late_ms = (uint32_t)clampi(n, 0, 5000);
    } else if (!strcmp(k, "gap_scd")) { if (parse_bool(v, &b)) c->gap_scd = b; else ok = 0;
    } else if (!strcmp(k, "gap_track_end")) { n = parse_num(v, &ok); if (ok && (n < 0 || n >= 0x00F00000)) ok = 0; if (ok) c->gap_track_end = (uint32_t)n;
    } else if (!strcmp(k, "gap_leadout")) { ok = parse_list(c->gap_leadout, &c->n_gap_leadout, v, GP_MAX_GAPLIST, 0, 10000);
    } else if (!strcmp(k, "gap_low")) { ok = parse_list(c->gap_low, &c->n_gap_low, v, GP_MAX_GAPLIST, 0, 149);
    } else if (!strcmp(k, "gap_count0")) { if (parse_bool(v, &b)) c->gap_count0 = b; else ok = 0;
    } else if (!strcmp(k, "gap_crc")) { if (parse_bool(v, &b)) c->gap_crc = b; else ok = 0;
    } else if (!strcmp(k, "pregap_probe")) { if (parse_bool(v, &b)) c->pregap_probe = b; else ok = 0;
    } else if (!strcmp(k, "pregap_low")) { ok = parse_list(c->pregap_low, &c->n_pregap_low, v, GP_MAX_GAPLIST, 0, 149);
    } else if (!strcmp(k, "pregap_offs")) { ok = parse_list(c->pregap_offs, &c->n_pregap_offs, v, GP_MAX_PGOFFS, -300, 300);
    } else if (!strcmp(k, "pregap_cross")) { n = parse_num(v, &ok); if (ok) c->pregap_cross = clampi(n, 2, 16);
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
