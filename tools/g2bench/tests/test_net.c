/* Host tests of g2n_core (NGTP frames, WANT/DATA pass bookkeeping, UDP burst packets).
 * Writes xcheck.bin for tests/test_sender.py: a HELLO frame, then g2b_gen_block(5). */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "../g2n_core.h"
#include "../g2b_core.h"
#include "../../netgd-recv/src/crc32.h"

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)

static void test_frames(void)
{
    uint8_t buf[G2N_HDR + 64 + 4];
    size_t n = g2n_frame_build(buf, G2N_T_LINE, 1, 7, "hello", 5);
    CHECK(n == 16 + 5 + 4);
    CHECK(!memcmp(buf, "NGT1", 4));
    g2n_hdr h;
    CHECK(g2n_hdr_parse(buf, &h) == 0);
    CHECK(h.type == G2N_T_LINE && h.flags == 1 && h.seq == 7 && h.len == 5);
    uint32_t crc = g2n_frame_crc(buf, buf + 16, 5);
    CHECK(crc == (uint32_t)buf[21] + ((uint32_t)buf[22] << 8) + ((uint32_t)buf[23] << 16) + ((uint32_t)buf[24] << 24));
    CHECK(crc == ng_crc32(0, buf, 21));
    /* bad headers */
    uint8_t bad[16];
    memcpy(bad, buf, 16); bad[0] = 'X';
    CHECK(g2n_hdr_parse(bad, &h) == -1);
    memcpy(bad, buf, 16); bad[6] = 1;
    CHECK(g2n_hdr_parse(bad, &h) == -1);
    memcpy(bad, buf, 16); bad[12] = 0x11; bad[13] = 0; bad[14] = 1; bad[15] = 0;    /* 65536 + 17 */
    CHECK(g2n_hdr_parse(bad, &h) == -1);
    memcpy(bad, buf, 16); bad[12] = 0x10; bad[13] = 0; bad[14] = 1; bad[15] = 0;    /* 65536 + 16: still allowed */
    CHECK(g2n_hdr_parse(bad, &h) == 0);

    uint8_t w[16];
    g2n_want_build(w, 3, 0x0102030405ull, 65536);
    CHECK(w[0] == 3 && w[1] == 0 && w[2] == 0 && w[3] == 0);
    CHECK(w[4] == 5 && w[5] == 4 && w[6] == 3 && w[7] == 2 && w[8] == 1 && w[9] == 0);
    CHECK(w[12] == 0 && w[13] == 0 && w[14] == 1 && w[15] == 0);

    uint8_t d[12 + 4] = { 9, 0, 0, 0, 0x10, 0, 0, 0, 0, 0, 0, 0, 'a', 'b', 'c', 'd' };
    uint16_t idx; uint64_t off; uint32_t dl; const uint8_t *data;
    CHECK(g2n_data_parse(d, sizeof d, &idx, &off, &dl, &data) == 0);
    CHECK(idx == 9 && off == 0x10 && dl == 4 && data == d + 12);
    CHECK(g2n_data_parse(d, 11, &idx, &off, &dl, &data) == -1);
}

static void test_pass(void)
{
    g2n_pass p; uint64_t off; uint32_t len;
    g2n_pass_init(&p, 4, 150000, 65536, 2);            /* 65536 + 65536 + 18928 */
    CHECK(g2n_pass_next_want(&p, &off, &len) && off == 0 && len == 65536);
    CHECK(g2n_pass_next_want(&p, &off, &len) && off == 65536 && len == 65536);
    CHECK(!g2n_pass_next_want(&p, &off, &len));        /* depth 2 reached */
    CHECK(!g2n_pass_done(&p));
    CHECK(g2n_pass_on_data(&p, 4, 0, 65536) == 0);
    CHECK(g2n_pass_next_want(&p, &off, &len) && off == 131072 && len == 18928);
    CHECK(g2n_pass_on_data(&p, 3, 65536, 65536) == G2N_E_FILE);
    CHECK(g2n_pass_on_data(&p, 4, 0, 65536) == G2N_E_OFFSET);
    CHECK(g2n_pass_on_data(&p, 4, 65536, 65000) == G2N_E_LENGTH);
    CHECK(g2n_pass_on_data(&p, 4, 65536, 65536) == 0);
    CHECK(g2n_pass_on_data(&p, 4, 131072, 18928) == 0);
    CHECK(p.recv_off == 150000 && p.outstanding == 0);
    CHECK(g2n_pass_done(&p));
    CHECK(g2n_pass_on_data(&p, 4, 150000, 1) == G2N_E_STATE);

    /* stop: no new WANTs, the one in flight still counts */
    g2n_pass_init(&p, 1, 1u << 20, 16384, 1);
    CHECK(g2n_pass_next_want(&p, &off, &len));
    g2n_pass_stop(&p);
    CHECK(!g2n_pass_done(&p) && !g2n_pass_next_want(&p, &off, &len));
    CHECK(g2n_pass_on_data(&p, 1, 0, 16384) == 0);
    CHECK(g2n_pass_done(&p) && p.recv_off == 16384);

    /* depth 1 lockstep over 32 MiB in 64 KiB: 512 WANTs */
    g2n_pass_init(&p, 2, 32u << 20, 65536, 1);
    unsigned wants = 0;
    while (!g2n_pass_done(&p)) {
        while (g2n_pass_next_want(&p, &off, &len)) { wants++; CHECK(g2n_pass_on_data(&p, 2, off, len) == 0); }
    }
    CHECK(wants == 512 && p.recv_off == (32u << 20));
}

static void test_udp(void)
{
    uint8_t pk[1400];
    g2n_udp_build(pk, sizeof pk, G2N_U_DATA, 3, 41);
    uint8_t t; uint16_t pass; uint32_t seq;
    CHECK(g2n_udp_parse(pk, sizeof pk, &t, &pass, &seq) == 0 && t == G2N_U_DATA && pass == 3 && seq == 41);
    CHECK(g2n_udp_parse(pk, 11, &t, &pass, &seq) == -1);
    pk[0] = 'X';
    CHECK(g2n_udp_parse(pk, sizeof pk, &t, &pass, &seq) == -1);

    g2n_udp_stat s; g2n_udp_stat_init(&s);
    uint32_t order[] = { 0, 1, 2, 5, 6, 3, 7 };       /* 3 arrives late, 4 never */
    for (unsigned i = 0; i < sizeof order / sizeof order[0]; i++) g2n_udp_stat_pkt(&s, order[i], 1400);
    CHECK(s.got == 7 && s.next == 8 && s.late == 1 && s.lost == 1 && g2n_udp_stat_missing(&s) == 1);
    CHECK(s.bytes == 7u * 1400);
}

static void write_xcheck(const char *path)
{
    FILE *f = fopen(path, "wb");
    CHECK(f != NULL);
    if (!f) return;
    uint8_t fr[64];
    size_t n = g2n_frame_build(fr, G2N_T_HELLO, 0, 12, "abc", 3);
    fwrite(fr, 1, n, f);
    static uint8_t blk[G2B_BLOCK];
    g2b_gen_block(5, blk);
    fwrite(blk, 1, sizeof blk, f);
    fclose(f);
}

int main(int argc, char **argv)
{
    test_frames();
    test_pass();
    test_udp();
    if (argc > 1) write_xcheck(argv[1]);
    if (fails) { printf("%d FAILED\n", fails); return 1; }
    printf("test_net: all ok\n");
    return 0;
}
