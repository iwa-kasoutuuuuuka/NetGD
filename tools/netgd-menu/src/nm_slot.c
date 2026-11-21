#include <string.h>
#include "nm_slot.h"

unsigned nm_slot_number(const char *name)
{
    size_t n = strlen(name);
    unsigned v = 0;
    if (n < 2 || n > 4) return 0;
    for (size_t i = 0; i < n; i++) {
        if (name[i] < '0' || name[i] > '9') return 0;
        v = v * 10 + (unsigned)(name[i] - '0');
    }
    return v;       /* 0 for "00", "000", "0000" */
}
