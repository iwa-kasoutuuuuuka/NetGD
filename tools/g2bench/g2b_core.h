/* g2bench core: test-pattern generator, key=value writer/parser, rate math.
 * No KOS dependency: host-tested (tests/test_all.c). */
#ifndef G2B_CORE_H
#define G2B_CORE_H
#include <stddef.h>
#include <stdint.h>

#define G2B_BLOCK 65536u          /* pattern block size = 64 KiB */
#define G2B_WORDS (G2B_BLOCK / 4)

/* Deterministic data file: block k (64 KiB, little-endian 32-bit words) word j =
 * xorshift32 sequence value j (seed 0x2545F491) XOR (k * 0x9E3779B1). mkdata.py writes the same. */
void g2b_gen_block(uint32_t k, uint8_t *out /* G2B_BLOCK bytes */);
/* CRC-32 (zlib) of the first `blocks` blocks of the file. */
uint32_t g2b_expected_crc(uint32_t blocks);

/* Same, for the first n bytes (n need not be a multiple of the block size). */
uint32_t g2b_expected_crc_bytes(uint64_t n);

/* KB/s (1 KB = 1024 B) from bytes and microseconds; 0 when us == 0. */
uint64_t g2b_kbps(uint64_t bytes, uint64_t us);

/* key=value writer into a caller buffer. Overflow sets err and keeps what fits. */
typedef struct { char *buf; size_t cap, len; int err; } g2b_kv;
void g2b_kv_init(g2b_kv *kv, char *buf, size_t cap);
void g2b_kv_str(g2b_kv *kv, const char *key, const char *val);
void g2b_kv_u64(g2b_kv *kv, const char *key, uint64_t v);
void g2b_kv_hex32(g2b_kv *kv, const char *key, uint32_t v);

/* Parser for the same format ('#' lines and blank lines ignored). */
#define G2B_MAX_ENTRIES 160
typedef struct { char key[48]; char val[96]; } g2b_entry;
typedef struct { g2b_entry e[G2B_MAX_ENTRIES]; int n; int bad_lines; } g2b_result;
void g2b_parse(g2b_result *r, const char *text, size_t len);
const char *g2b_get(const g2b_result *r, const char *key);      /* NULL when absent */
int g2b_get_u64(const g2b_result *r, const char *key, uint64_t *out); /* 0 ok, -1 absent/not a number */
#endif
