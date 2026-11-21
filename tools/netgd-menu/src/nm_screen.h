/* The screen: a 53 x 20 grid of characters and attributes (design D9).  The core fills it; the platform
   layer (bfont on the Dreamcast, a text dump on the host) only draws it. */
#ifndef NM_SCREEN_H
#define NM_SCREEN_H
#include <stdint.h>
#include "nm.h"

enum { NM_ATTR_NORMAL = 0, NM_ATTR_INVERT, NM_ATTR_DIM, NM_ATTR_WARN };

typedef struct {
    char ch[NM_ROWS][NM_COLS];
    uint8_t attr[NM_ROWS][NM_COLS];
} nm_screen_t;

void nm_screen_clear(nm_screen_t *s);
/* text at (row, col), clipped at the right edge; characters outside 0x20..0x7E become '?' */
void nm_screen_put(nm_screen_t *s, int row, int col, const char *text, int attr);
/* a whole row: text clipped/padded to the full width, all in `attr` */
void nm_screen_row(nm_screen_t *s, int row, const char *text, int attr);
void nm_screen_printf(nm_screen_t *s, int row, int col, int attr, const char *fmt, ...);
/* "[#####     ]" `width` columns wide (brackets included), pct 0..100 */
void nm_screen_bar(nm_screen_t *s, int row, int col, int width, int pct, int attr);
/* right-aligned text ending at the last column */
void nm_screen_put_right(nm_screen_t *s, int row, const char *text, int attr);
/* row `row` as text without trailing blanks (for tests); out holds NM_COLS + 1 bytes */
void nm_screen_row_text(const nm_screen_t *s, int row, char *out);
int nm_screen_equal(const nm_screen_t *a, const nm_screen_t *b);

#endif
