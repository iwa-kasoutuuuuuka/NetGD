#include <string.h>
#include "nm_text.h"

size_t nm_text_clean(char *dst, size_t dstn, const char *src, size_t srcn, size_t maxchars)
{
    size_t o = 0, i = 0;
    int pending_space = 0;
    if (!dstn) return 0;
    if (maxchars > dstn - 1) maxchars = dstn - 1;
    while (i < srcn && src[i] && o < maxchars) {
        unsigned char c = (unsigned char)src[i++];
        char out;
        if (c >= 0x80) {
            out = '?';
            if (c >= 0xC0) while (i < srcn && ((unsigned char)src[i] & 0xC0) == 0x80) i++;   /* rest of a UTF-8 sequence */
        } else if (c <= 0x20 || c == 0x7F) {
            pending_space = 1;
            continue;
        } else {
            out = (char)c;
        }
        if (pending_space && o) {
            if (o + 2 > maxchars) break;
            dst[o++] = ' ';
        }
        pending_space = 0;
        dst[o++] = out;
    }
    dst[o] = 0;
    return o;
}

size_t nm_text_fit(char *dst, size_t dstn, const char *src, size_t width)
{
    size_t n = strlen(src);
    if (!dstn) return 0;
    if (width > dstn - 1) width = dstn - 1;
    if (n <= width) {
        memcpy(dst, src, n);
        dst[n] = 0;
        return n;
    }
    if (width < 2) {
        memset(dst, '.', width);
        dst[width] = 0;
        return width;
    }
    memcpy(dst, src, width - 2);
    dst[width - 2] = '.';
    dst[width - 1] = '.';
    dst[width] = 0;
    return width;
}
