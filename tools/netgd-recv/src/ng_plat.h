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

/* 1 when the user pressed B on the controller (KOS) -- stop at the next safe point */
int ng_plat_user_stop(void);

#endif
