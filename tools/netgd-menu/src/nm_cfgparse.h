/* Small text-file parsers (design 6.3, 6.4).  All read from memory and never fail. */
#ifndef NM_CFGPARSE_H
#define NM_CFGPARSE_H
#include <stddef.h>
#include <stdint.h>

/* GDEMU.INI: the rules of common/gamecfg/src/ini.rs (only the keys the menu shows or uses). */
typedef struct {
    uint16_t open_time, detect_time;        /* 150..5000 ms; defaults 500 / 250 */
    uint16_t reset_goto;                    /* 0..9999; 0 = off; out of range = 0 */
    uint8_t open_time_set, detect_time_set; /* the file said it (only then a swap delay applies) */
} nm_ini_t;

void nm_ini_defaults(nm_ini_t *ini);
void nm_ini_parse(nm_ini_t *ini, const char *data, size_t n);
/* 10000 + the delays of the keys that were set: the READY wait of the launch (design 5.3) */
uint32_t nm_ini_ready_limit_ms(const nm_ini_t *ini);

/* GAME.CFG: the `title = "..."` value (common/gamecfg/src/lib.rs rules: `#` comments outside quotes,
   [sections] skipped, key case-sensitive, bare or quoted value).  Cleaned (nm_text_clean) to out.
   Returns 1 when a non-empty title was found. */
int nm_gamecfg_title(const char *data, size_t n, char *out, size_t outn);

/* NAME.TXT (GDMENU Card Manager): the first line, cleaned.  1 when non-empty. */
int nm_nametxt_title(const char *data, size_t n, char *out, size_t outn);

#endif
