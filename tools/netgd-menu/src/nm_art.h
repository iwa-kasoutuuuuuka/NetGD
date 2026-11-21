/* Box art (design step 11): NETGD.ART in a game's folder, shown at the right of the list for the game under the cursor.
     0  4  "NGAR"    4  2  version = 1    6  2  width = 128    8  2  height = 128    10  6  reserved (ignored)
     16 32768  128 x 128 pixels, rows from the top, RGB565, little endian
   Read on the light mount (read-only FatFs), 4 KiB per UI frame, after the cursor has stood still for 300 ms.
   A file that is missing, short, of another size or version, or unreadable is simply not shown: the list never waits for it
   and never reports an error.  Pillow-free PC side: tools/netgd-send/netgd_send/art.py.  Nothing here has run on a Dreamcast. */
#ifndef NM_ART_H
#define NM_ART_H
#include <stdint.h>
#include "nm.h"
#include "ff.h"

#define NM_ART_PW         128
#define NM_ART_PH         128
#define NM_ART_HDR       16
#define NM_ART_BYTES     (NM_ART_PW * NM_ART_PH * 2)
#define NM_ART_DEBOUNCE_MS 300
#define NM_ART_CHUNK     4096

/* where the picture goes: a block of the text grid that the core keeps blank while a picture is shown (12 x 24 px cells) */
#define NM_ART_COL       42
#define NM_ART_ROW       2
#define NM_ART_COLS      (NM_COLS - NM_ART_COL)     /* 11 cells = 132 px for 128 */
#define NM_ART_ROWS      6                          /* 144 px for 128 */

enum { NM_ART_NONE = 0, NM_ART_WAIT, NM_ART_LOADING, NM_ART_READY, NM_ART_FAILED };

typedef struct {
    int state;
    char folder[NM_FOLDER_MAX + 1];     /* the folder the state is about ("" = none) */
    uint32_t since;                     /* when the folder was chosen (ms) */
    int open;                           /* `f` is open */
    uint32_t got;                       /* pixel bytes read so far */
    FIL f;
    uint16_t px[NM_ART_PW * NM_ART_PH];
} nm_art_t;

void nm_art_init(nm_art_t *a);
/* The cursor is on `folder` (NULL = on nothing).  A folder different from the current one drops the current picture and
   starts the 300 ms wait; the same folder changes nothing. */
void nm_art_select(nm_art_t *a, const char *folder, uint32_t now);
/* Close the file and forget the folder (before the volume is unmounted or a launch starts). */
void nm_art_cancel(nm_art_t *a);
/* One frame of work (the debounce, then at most one 4 KiB read).  Needs the light mount. */
void nm_art_step(nm_art_t *a, uint32_t now);
/* the finished picture, or NULL */
const uint16_t *nm_art_pixels(const nm_art_t *a);

/* 0 when these 16 bytes are the header of a picture the menu shows; -1 otherwise (the reserved bytes are not looked at) */
int nm_art_check_header(const uint8_t *h);

#endif
