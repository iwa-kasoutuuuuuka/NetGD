/* The menu's state machine and screens (design 7).  One call of nm_ui_step() is one frame: read the
   buttons, advance the state, draw, present.  Everything here is platform independent. */
#ifndef NM_UI_H
#define NM_UI_H
#include "nm.h"
#include "nm_screen.h"
#include "nm_input.h"
#include "nm_list.h"
#include "nm_scan.h"
#include "nm_launch.h"
#include "fwctl.h"

#ifndef NM_HANDOFF_DEFAULT
#define NM_HANDOFF_DEFAULT 0            /* NM_HANDOFF_BIOS */
#endif
#ifndef NM_SHOW_SLOT1
#define NM_SHOW_SLOT1 0
#endif

typedef enum {
    NM_S_BOOT = 0,
    NM_S_NOFW,
    NM_S_NOSD,
    NM_S_LIST,
    NM_S_INFO,
    NM_S_LAUNCH,                        /* mounting the chosen game, waiting for READY */
    NM_S_DIALOG,                        /* a message after a failed launch */
    NM_S_CONFIRM_BIOS,                  /* START: really go back to the BIOS menu? */
    NM_S_HANDOFF                        /* "Starting ..." shown; the BIOS gets control after a short pause */
} nm_state_id_t;

typedef enum { NM_DLG_NOIMAGE = 0, NM_DLG_INCOMPLETE, NM_DLG_FWERR, NM_DLG_NOTREADY } nm_dialog_t;

typedef struct {
    int state;
    int boot_stage;
    nm_screen_t scr;
    nm_input_t in;
    uint32_t now;
    uint32_t events;                    /* button events of this frame */
    int controller;                     /* a controller was seen at the last frame */

    nm_list_t list;
    nm_scan_t scan;
    nm_ini_t ini;
    int cursor, top;
    char cursor_folder[NM_FOLDER_MAX + 1];
    int scan_gen_seen;

    ng_fwinfo_t fw;
    uint32_t gdemu_fw;                  /* the 52 00 firmware word (0 = not answered) */
    int handoff_mode;
    char why[96];                       /* NOSD: the reason */

    int sd_mounted;                     /* the light mount is active */
    nm_launch_t launch;
    int dialog;                         /* nm_dialog_t */
    unsigned dlg_number;
    char dlg_folder[NM_FOLDER_MAX + 1];
    char start_msg[NM_COLS + 1];        /* HANDOFF: the line shown */
    int handoff_target;                 /* HANDOFF: NM_HANDOFF_* */
    uint32_t handoff_since;
} nm_ui_t;

void nm_ui_init(nm_ui_t *ui);
void nm_ui_step(nm_ui_t *ui);

/* the "Reset: ..." line for a reset_goto value (design 6.4) */
void nm_ui_reset_text(unsigned reset_goto, char *out, size_t n);

#endif
