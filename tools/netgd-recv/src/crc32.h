#ifndef NG_CRC32_H
#define NG_CRC32_H
#include <stdint.h>
#include <stddef.h>

/* zlib-compatible, chainable: crc = ng_crc32(crc, data, n) starting from 0.
   The value is the finished CRC at every step, which is also what NETGD.CKP
   stores and what netgd-send writes to manifest.ngc. */
uint32_t ng_crc32(uint32_t crc, const void *data, size_t n);

#define NG_FNV_INIT 2166136261u
uint32_t ng_fnv1a(uint32_t h, const void *data, size_t n);
#endif
