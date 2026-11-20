/* KOS build platform glue. */
#include <stdio.h>
#include <string.h>
#include <time.h>
#include <kos.h>
#include <dc/maple.h>
#include <dc/maple/controller.h>
#include "ng_plat.h"
#include "ff.h"

uint32_t ng_plat_millis(void) { return (uint32_t)timer_ms_gettime64(); }

const char *ng_plat_pcpath(const char *pc, char *buf, size_t n)
{
    (void)buf; (void)n;
    return pc;                  /* dcload-ip serves "/pc/..." as is */
}

void ng_plat_console(const char *line)
{
    printf("%s\n", line);       /* one dcload call per line; the caller limits progress lines to 1/s */
    fflush(stdout);
}

int ng_plat_user_stop(void)
{
    maple_device_t *cont = maple_enum_type(0, MAPLE_FUNC_CONTROLLER);
    cont_state_t *st;
    if (!cont) return 0;
    st = (cont_state_t *)maple_dev_status(cont);
    return st && (st->buttons & CONT_B) ? 1 : 0;
}

/* FatFs clock (FF_FS_NORTC = 0) */
DWORD get_fattime(void)
{
    time_t t = time(NULL);
    struct tm *tm = localtime(&t);
    if (!tm || tm->tm_year < 80) return ((DWORD)(2025 - 1980) << 25) | (1u << 21) | (1u << 16);
    return ((DWORD)(tm->tm_year - 80) << 25) | ((DWORD)(tm->tm_mon + 1) << 21) | ((DWORD)tm->tm_mday << 16) |
           ((DWORD)tm->tm_hour << 11) | ((DWORD)tm->tm_min << 5) | ((DWORD)tm->tm_sec >> 1);
}
