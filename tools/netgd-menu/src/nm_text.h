/* Text for the 53 x 20 character grid: ASCII only. */
#ifndef NM_TEXT_H
#define NM_TEXT_H
#include <stddef.h>

/* Copy src[0..srcn) to dst (NUL-terminated, at most dstn-1 bytes and maxchars characters):
   a non-ASCII character (a UTF-8 sequence counts as one) becomes '?', control characters and
   runs of white space become one space, leading and trailing white space is dropped.
   Returns the length of dst. */
size_t nm_text_clean(char *dst, size_t dstn, const char *src, size_t srcn, size_t maxchars);

/* src fitted to `width` characters: longer text is cut and ends in "..".  dst gets width+1 bytes at most
   and is NOT padded.  Returns the length. */
size_t nm_text_fit(char *dst, size_t dstn, const char *src, size_t width);

#endif
