#include "hash.h"

static uint32_t g_table[256];
static int g_ready;

static void make_table(void)
{
    uint32_t i, j, c;
    for (i = 0; i < 256; i++) {
        c = i;
        for (j = 0; j < 8; j++)
            c = (c & 1) ? (0xEDB88320u ^ (c >> 1)) : (c >> 1);
        g_table[i] = c;
    }
    g_ready = 1;
}

uint32_t crc32_update(uint32_t crc, const void *data, size_t len)
{
    const uint8_t *p = (const uint8_t *)data;
    if (!g_ready)
        make_table();
    crc = ~crc;
    while (len--)
        crc = g_table[(crc ^ *p++) & 0xFF] ^ (crc >> 8);
    return ~crc;
}

uint32_t crc32_calc(const void *data, size_t len)
{
    return crc32_update(0, data, len);
}
