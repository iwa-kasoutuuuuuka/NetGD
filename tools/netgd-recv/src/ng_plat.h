/* The few things that differ between the KOS build (plat_kos.c) and the host test
   build (test/plat_host.c). */
#ifndef NG_PLAT_H
#define NG_PLAT_H
#include <stdint.h>
#include <stddef.h>

uint32_t ng_plat_millis(void);
/* "/pc/..." as the program sees it -> the path fopen() takes (KOS: unchanged) */
const char *ng_plat_pcpath(const char *pc, char *buf, size_t n);
/* a line for the console (dc-tool-ip prints it); NUL-terminated, no newline */
void ng_plat_console(const char *line);

/* The level of B on the first controller (1 = down); in *pad, which pad it was read from: 0 = no pad (B reads 0 then), otherwise a
   number that stays the same for one port and unit; in *odd, 1 when the whole status is implausible (ng_pad_implausible).  Not a
   decision: callers go through ng_btnguard_sample, because a pad can report B all the time. */
int ng_plat_user_stop(uint32_t *pad, int *odd);

#endif
