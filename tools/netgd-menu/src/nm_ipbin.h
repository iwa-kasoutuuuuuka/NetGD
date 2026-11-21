/* Title from a GDI's high-density track (design 6.3 step 3). */
#ifndef NM_IPBIN_H
#define NM_IPBIN_H
#include <stddef.h>
#include <stdint.h>

/* From the text of a .gdi: the file name and the sector length (2048 or 2352) of track 3 (else the
   track that starts at LBA 45000).  A leading BOM and a quoted name are handled.  1 = found. */
int nm_gdi_track3(const char *data, size_t n, char *fname, size_t fnamen, unsigned *sector_size);

/* From the first sector of that file (secsize bytes: 2048, or 2352 raw with the user data at byte 16):
   if it starts with "SEGA SEGAKATANA ", the 128 bytes at 0x80 are the title -> cleaned to out.
   1 = a title was found. */
int nm_ipbin_title(const uint8_t *sector, size_t n, unsigned secsize, char *out, size_t outn);

#endif
