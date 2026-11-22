#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "../g2b_core.h"
#include "../../netgd-recv/src/crc32.h"

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)

int main(void)
{
    /* standard vectors (same implementation as netgd-recv) */
    CHECK(ng_crc32(0, "123456789", 9) == 0xCBF43926u);
    CHECK(ng_crc32(ng_crc32(0, "1234", 4), "56789", 5) == 0xCBF43926u);   /* chainable */
    CHECK(ng_fnv1a(2166136261u, "a", 1) == 0xE40C292Cu);
    CHECK(ng_fnv1a(2166136261u, "", 0) == 2166136261u);

    /* generator is deterministic and blocks differ */
    static uint8_t a[G2B_BLOCK], b[G2B_BLOCK];
    g2b_gen_block(0, a); g2b_gen_block(0, b);
    CHECK(!memcmp(a, b, sizeof a));
    g2b_gen_block(1, b);
    CHECK(memcmp(a, b, sizeof a));
    CHECK(g2b_expected_crc(2) == ng_crc32(ng_crc32(0, a, sizeof a), b, sizeof b));

    /* mkdata.py writes exactly the C pattern (2 MiB = 32 blocks) */
    const char *tf = getenv("G2B_TESTFILE");
    if (tf) {
        FILE *f = fopen(tf, "rb");
        CHECK(f != NULL);
        if (f) {
            uint32_t crc = 0; size_t tot = 0, n;
            static uint8_t fb[G2B_BLOCK];
            while ((n = fread(fb, 1, sizeof fb, f)) > 0) {
                g2b_gen_block((uint32_t)(tot / G2B_BLOCK), a);
                CHECK(n == G2B_BLOCK && !memcmp(a, fb, n));
                crc = ng_crc32(crc, fb, n); tot += n;
            }
            fclose(f);
            CHECK(tot == 2u << 20);
            CHECK(crc == g2b_expected_crc(32));
        }
    }

    CHECK(g2b_expected_crc_bytes(2u * G2B_BLOCK) == g2b_expected_crc(2));
    g2b_gen_block(0, a); g2b_gen_block(1, b);
    CHECK(g2b_expected_crc_bytes(G2B_BLOCK + 4096) == ng_crc32(ng_crc32(0, a, sizeof a), b, 4096));
    CHECK(g2b_expected_crc_bytes(0) == 0);

    /* rate */
    CHECK(g2b_kbps(32u << 20, 1000000) == 32768);
    CHECK(g2b_kbps(1, 0) == 0);
    CHECK(g2b_kbps(1024, 1000000) == 1);

    /* kv writer + parser round trip */
    char buf[512]; g2b_kv kv; g2b_kv_init(&kv, buf, sizeof buf);
    g2b_kv_str(&kv, "tool", "g2bench");
    g2b_kv_u64(&kv, "read_64k_kbps", 812);
    g2b_kv_hex32(&kv, "crc", 0xCBF43926u);
    g2b_kv_str(&kv, "status", "running");
    g2b_kv_str(&kv, "status", "done");
    CHECK(!kv.err);
    static g2b_result r;
    g2b_parse(&r, buf, kv.len);
    CHECK(r.n == 5 && r.bad_lines == 0);
    uint64_t v = 0;
    CHECK(g2b_get_u64(&r, "read_64k_kbps", &v) == 0 && v == 812);
    CHECK(g2b_get_u64(&r, "crc", &v) == 0 && v == 0xCBF43926u);
    CHECK(!strcmp(g2b_get(&r, "status"), "done"));         /* later key wins */
    CHECK(g2b_get(&r, "nope") == NULL);
    CHECK(g2b_get_u64(&r, "tool", &v) == -1);              /* not a number */

    /* parser robustness: comments, CRLF, junk, truncated last line */
    const char *t = "# c\r\nfoo=1\r\n\r\nbad line\n=x\nbar=2";
    g2b_parse(&r, t, strlen(t));
    CHECK(r.n == 2 && r.bad_lines == 2);
    CHECK(g2b_get_u64(&r, "foo", &v) == 0 && v == 1);
    CHECK(g2b_get_u64(&r, "bar", &v) == 0 && v == 2);
    g2b_parse(&r, "", 0);
    CHECK(r.n == 0);
    char big[300]; memset(big, 'x', sizeof big - 1); big[sizeof big - 1] = 0;
    char line[400]; snprintf(line, sizeof line, "k=%s\n", big);
    g2b_parse(&r, line, strlen(line));
    CHECK(r.n == 0 && r.bad_lines == 1);                   /* over-long value is rejected, no overflow */

    /* writer overflow is flagged, not written past the buffer */
    char small[16]; g2b_kv_init(&kv, small, sizeof small);
    g2b_kv_str(&kv, "a", "1");
    g2b_kv_str(&kv, "long_key_name", "long_value_here");
    CHECK(kv.err && strlen(small) < sizeof small);

    if (fails) { printf("%d failures\n", fails); return 1; }
    printf("g2bench tests passed\n");
    return 0;
}
