/* g2netbench core: NGTP framing (docs/design/menu-app.md 8), the WANT/DATA pass bookkeeping and the UDP burst
 * packets. No KOS dependency: host-tested (tests/test_net.c). The CRC is netgd-recv's ng_crc32. */
#ifndef G2N_CORE_H
#define G2N_CORE_H
#include <stddef.h>
#include <stdint.h>

#define G2N_TCP_PORT   20039u     /* the NGTP port of menu-app.md 8 */
#define G2N_UDP_PORT   20040u     /* UDP burst test */
#define G2N_HDR        16u
#define G2N_MAX_DATA   65536u
#define G2N_MAX_PAYLOAD (G2N_MAX_DATA + 16u)

/* frame types (same numbers as tools/netgd-send/netgd_send/ngtp.py) */
#define G2N_T_HELLO 0x01
#define G2N_T_PUT   0x04
#define G2N_T_DATA  0x05
#define G2N_T_HELLO_OK 0x81
#define G2N_T_LINE  0x90
#define G2N_T_DONE  0x91
#define G2N_T_WANT  0x92
#define G2N_T_ERROR 0x7F

typedef struct { uint8_t type, flags; uint32_t seq, len; } g2n_hdr;

/* "NGT1", type, flags, 0, 0, seq, len, payload, CRC-32 of everything before it. Returns n + 20. */
size_t g2n_frame_build(uint8_t *out, uint8_t type, uint8_t flags, uint32_t seq, const void *payload, uint32_t n);
/* 0 ok; -1 bad magic / reserved bytes / length over G2N_MAX_PAYLOAD */
int g2n_hdr_parse(const uint8_t hdr[G2N_HDR], g2n_hdr *h);
/* CRC-32 of header + payload, to compare with the 4 bytes after the payload */
uint32_t g2n_frame_crc(const uint8_t hdr[G2N_HDR], const void *payload, uint32_t n);

/* Incremental frame reassembly (what recv_all + parse do on the Dreamcast, but for any chunking of the byte stream).
 * buf holds the payload + 4 CRC bytes of the frame being built (cap >= G2N_MAX_PAYLOAD + 4). */
typedef struct { uint8_t hdr[G2N_HDR]; uint32_t have, need; g2n_hdr h; uint8_t *buf; uint32_t cap; int in_body; } g2n_rx;
void g2n_rx_init(g2n_rx *r, uint8_t *buf, uint32_t cap);
/* Feeds up to n bytes; *used = bytes taken. Returns 1 = one whole frame, CRC ok (r->h, payload in r->buf),
 * 0 = need more, -1 = bad header (stream lost), -2 = bad CRC (frame was complete: the stream is still aligned). */
int g2n_rx_feed(g2n_rx *r, const uint8_t *p, size_t n, size_t *used);

/* WANT payload: u16 file, u16 0, u64 offset, u32 length (16 bytes) */
void g2n_want_build(uint8_t out[16], uint16_t idx, uint64_t off, uint32_t len);
/* DATA payload: u16 file, u16 0, u64 offset, data. 0 ok, -1 too short */
int g2n_data_parse(const uint8_t *p, uint32_t n, uint16_t *idx, uint64_t *off, uint32_t *dlen, const uint8_t **data);

/* One measured pass: the Dreamcast asks for `total` bytes of file `idx` in `want`-byte pieces with up to
 * `depth` WANTs in flight (depth 1 = netgd-recv's plan, 2+ = read-ahead). */
typedef struct {
    uint16_t idx;
    uint64_t total, issue_off, recv_off;
    uint32_t want, depth, outstanding;
    int stopped;
} g2n_pass;
void g2n_pass_init(g2n_pass *p, uint16_t idx, uint64_t total, uint32_t want, uint32_t depth);
/* 1 = send a WANT(off, len) now (the pass counts it as in flight), 0 = none */
int g2n_pass_next_want(g2n_pass *p, uint64_t *off, uint32_t *len);
void g2n_pass_stop(g2n_pass *p);                      /* no more WANTs (time limit); in-flight ones still arrive */
/* A DATA frame arrived: must be the oldest WANT (file, offset, exact length). 0 ok, else G2N_E_* */
int g2n_pass_on_data(g2n_pass *p, uint16_t idx, uint64_t off, uint32_t len);
int g2n_pass_done(const g2n_pass *p);                 /* nothing in flight and nothing left to ask */
#define G2N_E_STATE  1                                /* DATA without a WANT in flight */
#define G2N_E_FILE   2
#define G2N_E_OFFSET 3
#define G2N_E_LENGTH 4

/* UDP burst packet: "G2U1", type (1 data, 2 end), 0, u16 pass, u32 seq, then filler up to `size` bytes */
#define G2N_U_DATA 1
#define G2N_U_END  2
#define G2N_U_HDR  12u
void g2n_udp_build(uint8_t *out, uint32_t size, uint8_t type, uint16_t pass, uint32_t seq);
int g2n_udp_parse(const uint8_t *p, uint32_t n, uint8_t *type, uint16_t *pass, uint32_t *seq);   /* 0 ok */

typedef struct { uint32_t next, got, lost, late; uint64_t bytes; int started; } g2n_udp_stat;
void g2n_udp_stat_init(g2n_udp_stat *s);
/* a data packet with sequence `seq` of `bytes` bytes: counts holes (lost, until a late one fills it) and late ones */
void g2n_udp_stat_pkt(g2n_udp_stat *s, uint32_t seq, uint32_t bytes);
/* packets that never arrived = (next) - got, once the end is reached */
uint32_t g2n_udp_stat_missing(const g2n_udp_stat *s);
#endif
