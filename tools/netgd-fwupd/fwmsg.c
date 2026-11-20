/* netgd-fwupd: message table (generated, see gen_msgs.py) and EUC-JP text layout. */
#include <string.h>
#include "fwcore.h"

static const struct {
    const char *en, *ja8, *euc;
} g_msgs[FWM__COUNT] = {
#define FWMSG(id, en, ja8, jaeuc) [id] = { en, ja8, jaeuc },
#include "fwmsg_gen.h"
#undef FWMSG
};

static int ok_id(int id) { return id > 0 && id < FWM__COUNT && g_msgs[id].en; }

const char *fw_msg_en(int id) { return ok_id(id) ? g_msgs[id].en : ""; }
const char *fw_msg_ja(int id) { return ok_id(id) ? g_msgs[id].ja8 : ""; }
const char *fw_msg_euc(int id) { return ok_id(id) ? g_msgs[id].euc : ""; }

/* EUC-JP as the BIOS font reads it (biosfont.c:347-377): a byte >= 0x80 starts a pair,
 * 0x8E + one byte is half-width kana (1 cell), anything else is one wide character. */
static int char_bytes(const unsigned char *p, size_t left, int *cells)
{
    if (*p < 0x80) { *cells = 1; return 1; }
    if (*p == 0x8E) { *cells = 1; return left >= 2 ? 2 : 1; }
    *cells = 2;
    return left >= 2 ? 2 : 1;
}

int fw_text_cells(const char *euc, size_t len)
{
    const unsigned char *p = (const unsigned char *)euc;
    size_t i = 0;
    int total = 0;
    while (i < len && p[i] && p[i] != '\n') {
        int c, n = char_bytes(p + i, len - i, &c);
        total += c;
        i += (size_t)n;
    }
    return total;
}

int fw_text_wrap(const char *euc, int cells, char *out, size_t linecap, int maxlines)
{
    const unsigned char *p = (const unsigned char *)euc;
    int lines = 0;
    size_t i = 0, fill = 0;
    int used = 0;
    long last_space = -1;           /* index in the current line of the last ' ' */
    size_t n_in = strlen(euc);

    if (maxlines <= 0 || linecap < 4 || cells < 2) return 0;

#define FLUSH_LINE() do { \
        out[(size_t)lines * linecap + fill] = 0; \
        lines++; fill = 0; used = 0; last_space = -1; \
    } while (0)

    while (i < n_in) {
        int c, n;
        char *line;
        if (lines >= maxlines) break;
        line = out + (size_t)lines * linecap;
        if (p[i] == '\n') {
            FLUSH_LINE();
            i++;
            continue;
        }
        n = char_bytes(p + i, n_in - i, &c);
        if (used + c > cells || fill + (size_t)n >= linecap) {
            /* Overflow. In the middle of an ASCII word, break at the last space instead. */
            if (p[i] < 0x80 && p[i] != ' ' && last_space >= 0) {
                size_t keep = (size_t)last_space;          /* the space itself is dropped */
                size_t rest = fill - keep - 1;
                char tail[256];
                int tail_cells = 0;
                if (rest >= sizeof tail) rest = sizeof tail - 1;
                memcpy(tail, line + keep + 1, rest);
                tail[rest] = 0;
                tail_cells = fw_text_cells(tail, rest);
                fill = keep;
                FLUSH_LINE();
                if (lines >= maxlines) break;
                line = out + (size_t)lines * linecap;
                memcpy(line, tail, rest);
                fill = rest;
                used = tail_cells;
                last_space = -1;
            } else {
                FLUSH_LINE();
                if (lines >= maxlines) break;
                line = out + (size_t)lines * linecap;
            }
            if (p[i] == ' ' && fill == 0) { i++; continue; }  /* no leading space */
        }
        if (p[i] == ' ') last_space = (long)fill;
        memcpy(line + fill, p + i, (size_t)n);
        fill += (size_t)n;
        used += c;
        i += (size_t)n;
    }
    if (lines < maxlines && (fill > 0 || lines == 0)) FLUSH_LINE();
#undef FLUSH_LINE
    return lines;
}
