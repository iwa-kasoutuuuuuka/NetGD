#include <stdarg.h>
#include <stdio.h>
#include <string.h>
#include "nm_screen.h"

void nm_screen_clear(nm_screen_t *s)
{
    memset(s->ch, ' ', sizeof s->ch);
    memset(s->attr, NM_ATTR_NORMAL, sizeof s->attr);
}

void nm_screen_put(nm_screen_t *s, int row, int col, const char *text, int attr)
{
    if (row < 0 || row >= NM_ROWS) return;
    for (; *text && col < NM_COLS; text++, col++) {
        unsigned char c = (unsigned char)*text;
        if (col < 0) continue;
        s->ch[row][col] = (c >= 0x20 && c < 0x7F) ? (char)c : '?';
        s->attr[row][col] = (uint8_t)attr;
    }
}

void nm_screen_row(nm_screen_t *s, int row, const char *text, int attr)
{
    if (row < 0 || row >= NM_ROWS) return;
    memset(s->ch[row], ' ', NM_COLS);
    memset(s->attr[row], attr, NM_COLS);
    nm_screen_put(s, row, 0, text, attr);
}

void nm_screen_printf(nm_screen_t *s, int row, int col, int attr, const char *fmt, ...)
{
    char buf[256];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(buf, sizeof buf, fmt, ap);
    va_end(ap);
    nm_screen_put(s, row, col, buf, attr);
}

void nm_screen_bar(nm_screen_t *s, int row, int col, int width, int pct, int attr)
{
    char buf[NM_COLS + 1];
    int inner = width - 2, fill;
    if (width < 3 || width > NM_COLS) return;
    if (pct < 0) pct = 0;
    if (pct > 100) pct = 100;
    fill = inner * pct / 100;
    buf[0] = '[';
    for (int i = 0; i < inner; i++) buf[1 + i] = i < fill ? '#' : ' ';
    buf[width - 1] = ']';
    buf[width] = 0;
    nm_screen_put(s, row, col, buf, attr);
}

void nm_screen_put_right(nm_screen_t *s, int row, const char *text, int attr)
{
    int n = (int)strlen(text);
    nm_screen_put(s, row, NM_COLS - n, text, attr);
}

void nm_screen_row_text(const nm_screen_t *s, int row, char *out)
{
    int n = NM_COLS;
    memcpy(out, s->ch[row], NM_COLS);
    while (n > 0 && out[n - 1] == ' ') n--;
    out[n] = 0;
}

int nm_screen_equal(const nm_screen_t *a, const nm_screen_t *b)
{
    return !memcmp(a->ch, b->ch, sizeof a->ch) && !memcmp(a->attr, b->attr, sizeof a->attr);
}
