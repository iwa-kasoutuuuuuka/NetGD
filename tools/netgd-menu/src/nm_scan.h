/* Scanning the SD card for games (design 6.2): read-only FatFs on the light mount, one folder per step. */
#ifndef NM_SCAN_H
#define NM_SCAN_H
#include "nm.h"
#include "nm_list.h"
#include "nm_cfgparse.h"
#include "ngfs.h"

enum { NM_SCAN_IDLE = 0, NM_SCAN_RUNNING, NM_SCAN_DONE, NM_SCAN_ABORTED };

typedef struct {
    nm_list_t *list;
    nm_ini_t ini;
    int have_ini;                   /* GDEMU.INI was found */
    char default_image[NG_PATH_BUF];/* GDEMU.CFG image= (normalised), "" = none */
    int state;
    int next;                       /* index of the next folder to look into */
    int total;                      /* folders at the start of the scan (for "n/total") */
    int fail_streak;                /* consecutive folders that could not be read */
    int failed;                     /* folders marked NM_ST_ERROR */
    int generation;                 /* +1 whenever an item is removed from the list (indexes moved) */
    int show_slot1;                 /* -DNM_SHOW_SLOT1=1: list folder 01 as well */
} nm_scan_t;

/* The root pass: folders, GDEMU.INI, GDEMU.CFG; sorts the list.  0 ok, -1 the root could not be read
   (state ABORTED).  The volume must be mounted with ngfs_mount_light. */
int nm_scan_begin(nm_scan_t *sc, nm_list_t *list, int show_slot1);
/* Look into one folder.  1 = more to do, 0 = finished, -1 = stopped after 3 unreadable folders in a row. */
int nm_scan_step(nm_scan_t *sc);

#endif
