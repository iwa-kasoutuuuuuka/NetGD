/* netgd-menu: common definitions.  Design: docs/design/menu-app.md.
   Nothing here has run on a Dreamcast. */
#ifndef NM_H
#define NM_H

#include <stdint.h>
#include <stddef.h>

#define NM_VERSION "0.1.0"

#define NM_MAX_GAMES    1024
#define NM_FOLDER_MAX   96      /* longest folder name kept (buffers hold one more) */
#define NM_TITLE_MAX    64
#include "ng_btnguard.h"

#define NM_COLS         53
#define NM_ROWS         20

/* buttons: the bits nm_plat_buttons() returns */
#define NM_BTN_UP     0x0001u
#define NM_BTN_DOWN   0x0002u
#define NM_BTN_LEFT   0x0004u
#define NM_BTN_RIGHT  0x0008u
#define NM_BTN_A      0x0010u
#define NM_BTN_B      0x0020u
#define NM_BTN_X      0x0040u
#define NM_BTN_Y      0x0080u
#define NM_BTN_START  0x0100u
#define NM_BTN_LT     0x0200u
#define NM_BTN_RT     0x0400u
#define NM_PAD_IMPLAUSIBLE(b) ng_pad_implausible((b), NM_BTN_UP, NM_BTN_DOWN, NM_BTN_LEFT, NM_BTN_RIGHT, \
                                                 NM_BTN_A | NM_BTN_B | NM_BTN_X | NM_BTN_Y | NM_BTN_START)

typedef enum { NM_KIND_UNKNOWN = 0, NM_KIND_GDI, NM_KIND_CDI } nm_kind_t;
typedef enum { NM_ST_UNSCANNED = 0, NM_ST_COMPLETE, NM_ST_PARTIAL, NM_ST_EMPTY, NM_ST_ERROR } nm_state_t;

#define NM_F_DEFAULT  0x01      /* the image= folder of GDEMU.CFG (protected by the receive core) */
#define NM_F_DUPLICATE 0x02     /* another folder has the same number (the one with fewer digits is kept) */

typedef struct {
    uint16_t number;                    /* 0 = a named folder (mounted with MOUNT_NAME) */
    uint8_t kind, state, flags;
    char folder[NM_FOLDER_MAX + 1];
    char title[NM_TITLE_MAX + 1];
} nm_game_t;

#endif
