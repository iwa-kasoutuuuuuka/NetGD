#include <stdio.h>
#include <string.h>
#include <time.h>
#include "ng_plat.h"
#include "ff.h"
#include "simdisk.h"

char sim_pc_root[260] = "";
int sim_quiet;
int sim_stop_after_polls = -1;
int sim_polls;
int sim_b_stuck;                        /* B reads down on every poll (a pad that reports it all the time) */

uint32_t ng_plat_millis(void) { return (uint32_t)((double)clock() * 1000.0 / CLOCKS_PER_SEC); }

const char *ng_plat_pcpath(const char *pc, char *buf, size_t n)
{
    if (!strncmp(pc, "/pc/", 4) || !strcmp(pc, "/pc")) {
        snprintf(buf, n, "%s%s", sim_pc_root, pc + 3);
        return buf;
    }
    return pc;
}

void ng_plat_console(const char *line)
{
    if (!sim_quiet) printf("%s\n", line);
}

int ng_plat_user_stop(uint32_t *pad)
{
    *pad = 1;
    sim_polls++;
    return sim_b_stuck || (sim_stop_after_polls >= 0 && sim_polls > sim_stop_after_polls);
}

DWORD get_fattime(void) { return ((DWORD)(2025 - 1980) << 25) | (6u << 21) | (1u << 16); }
