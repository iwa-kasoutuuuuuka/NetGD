/* gdprobe: CRC-32 (IEEE 802.3, same as zlib.crc32 / Python binascii.crc32).
 * Pure C, no KOS dependency. */
#ifndef GDPROBE_HASH_H
#define GDPROBE_HASH_H
#include <stddef.h>
#include <stdint.h>

/* One-shot CRC-32 of a buffer. crc32_calc("123456789", 9) == 0xCBF43926. */
uint32_t crc32_calc(const void *data, size_t len);

/* Incremental form: start with crc = 0, feed chunks. */
uint32_t crc32_update(uint32_t crc, const void *data, size_t len);

#endif
