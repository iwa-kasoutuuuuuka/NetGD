/* g2x: custom UDP transfer for G2 (R7 of docs/design/bba-sd-write.md, docs/research/g2-transport.md).
 * The PC streams 1,400-byte datagrams (20-byte header + 1,380 data bytes, CRC-32 per datagram); the Dreamcast
 * keeps a window of G2X_WIN slots, delivers the data in order and answers with cumulative + selective ACKs
 * (64-bit SACK bitmap) and a credit limit. Pacing and loss recovery are the sender's job (g2n_send.py; the
 * same algorithm is in g2x_tx_* here for the simulated lossy link of tests/test_xport.c).
 * No KOS dependency: host-tested. The CRC is netgd-recv's ng_crc32 (zlib CRC-32).
 *
 * DATA (PC -> Dreamcast UDP 20040), little-endian:
 *   0 "G2X1"  4 u8 type=1  5 u8 flags (bit0 resend, bit1 crc present)  6 u16 pass
 *   8 u32 seq (datagram number; data offset = seq * G2X_PAYLOAD)  12 u16 len  14 u16 0
 *  16 u32 crc (CRC-32 of bytes 0..15 and the data; 0 when bit1 is clear)  20 data[len]
 * ACK (Dreamcast -> the DATA's source address):
 *   0 "G2XA"  4 u8 type=2  5 u8 flags (bit0 complete, bit1 stop: time limit)  6 u16 pass
 *   8 u32 cum (every seq < cum delivered)  12 u32 limit (send only seq < limit)
 *  16 u32 sack_lo, 20 u32 sack_hi (bit i: seq cum + 1 + i is held)  24 u32 ack_no
 *  28 u32 dgrams (all DATA seen)  32 u32 dups  36 u32 bad (CRC / header) */
#ifndef G2X_CORE_H
#define G2X_CORE_H
#include <stddef.h>
#include <stdint.h>

#define G2X_HDR      20u
#define G2X_PAYLOAD  1380u                       /* datagram = 1,400 bytes, as in the R178 UDP bursts */
#define G2X_DGRAM    (G2X_HDR + G2X_PAYLOAD)
#define G2X_ACK_LEN  40u
#define G2X_WIN      64u                         /* receive slots (88 KB); also the SACK range + 1 */
#define G2X_T_DATA   1
#define G2X_T_ACK    2
#define G2X_F_RESEND 0x01
#define G2X_F_CRC    0x02
#define G2X_A_DONE   0x01
#define G2X_A_STOP   0x02

typedef struct { uint8_t type, flags; uint16_t pass; uint32_t seq; uint16_t len; uint32_t crc; } g2x_data_hdr;
typedef struct { uint8_t flags; uint16_t pass; uint32_t cum, limit, sack_lo, sack_hi, ack_no, dgrams, dups, bad; } g2x_ack;

/* builds a DATA datagram (with_crc: compute the CRC); returns G2X_HDR + len */
size_t g2x_data_build(uint8_t *out, uint16_t pass, uint32_t seq, uint8_t flags, const uint8_t *data, uint16_t len, int with_crc);
/* 0 ok (header sane, CRC right when present); -1 not G2X / bad length; -2 CRC wrong */
int g2x_data_parse(const uint8_t *p, size_t n, g2x_data_hdr *h, const uint8_t **data);
void g2x_ack_build(uint8_t out[G2X_ACK_LEN], const g2x_ack *a);
int g2x_ack_parse(const uint8_t *p, size_t n, g2x_ack *a);            /* 0 ok */

/* number of datagrams and the length of datagram `seq` for a transfer of `total` bytes */
uint32_t g2x_count(uint64_t total);
uint16_t g2x_len(uint64_t total, uint32_t seq);

/* ---- receiver (Dreamcast) ---- */
typedef void (*g2x_deliver_fn)(void *ctx, uint32_t seq, const uint8_t *data, uint16_t len);
typedef struct {
    uint16_t pass;
    uint32_t count;                      /* datagrams in the transfer */
    uint64_t total;
    uint32_t cum;                        /* next seq to deliver */
    uint32_t high;                       /* highest seq held + 1 (>= cum) */
    uint8_t have[G2X_WIN];
    uint16_t len[G2X_WIN];
    uint8_t (*slot)[G2X_PAYLOAD];        /* G2X_WIN slots, caller memory */
    g2x_deliver_fn deliver; void *ctx;
    int need_crc;                        /* reject DATA without a CRC */
    /* ACK policy state */
    uint32_t since_ack;                  /* new datagrams since the last ACK */
    uint32_t ack_no;
    int gap_pending;                     /* a hole appeared / a dup arrived since the last ACK */
    uint64_t last_ack_us, last_rx_us;
    /* counters */
    uint32_t dgrams, dups, bad, beyond, gaps, acks;
} g2x_rx;
#define G2X_ACK_EVERY   16u              /* in-order datagrams per ACK */
#define G2X_ACK_GAP_US  1000u            /* a hole: ACK at once, but not more often than this */
#define G2X_ACK_IDLE_US 4000u            /* data held but no ACK for this long: ACK anyway */

void g2x_rx_init(g2x_rx *r, uint16_t pass, uint64_t total, uint8_t (*slots)[G2X_PAYLOAD], g2x_deliver_fn fn, void *ctx);
/* one received datagram. Returns 1 if an ACK should be sent now (then call g2x_rx_make_ack), 0 otherwise,
   -1 when it was not a DATA datagram of this pass (ignored) */
int g2x_rx_input(g2x_rx *r, const uint8_t *p, size_t n, uint64_t now_us);
/* timer: 1 when an ACK is due (idle with something unacknowledged) */
int g2x_rx_poll(g2x_rx *r, uint64_t now_us);
void g2x_rx_make_ack(g2x_rx *r, g2x_ack *a, uint8_t flags, uint64_t now_us);
int g2x_rx_complete(const g2x_rx *r);

/* ---- sender (the PC algorithm; g2n_send.py implements the same) ---- */
typedef struct {
    uint32_t count;
    uint32_t next_new;                   /* next never-sent seq */
    uint32_t cum, limit;
    uint8_t *acked;                      /* count bytes, caller memory */
    uint64_t *stamp;                     /* count: transmission order of the latest send of each seq */
    uint64_t tx_no;                      /* transmissions so far */
    uint32_t *rq; uint32_t rq_n, rq_cap; /* retransmit queue (seqs) */
    uint8_t *queued;                     /* count: seq is in rq */
    uint64_t last_progress_us;           /* last ACK with progress, or the last RTO */
    uint64_t acked_stamp;                /* newest transmission (stamp) known to have arrived */
    uint32_t rto_us;                     /* current RTO: doubles per RTO without progress */
    int64_t probe;                       /* seq last resent by an RTO (until acknowledged), or -1: an ACK of it
                                            may be for the earlier copy (Karn), so its stamp says nothing */
    double pps, pps_min, pps_max;        /* pacing rate; adaptive when pps_max > pps_min */
    uint64_t last_cut_us;
    uint32_t sent, resent, loss_events, rto_fires;
} g2x_tx;
#define G2X_RTO_US     30000u            /* nothing acknowledged for this long: resend the oldest unacked */
#define G2X_RTO_MAX_US 500000u           /* the RTO doubles per expiry without progress, up to this */
#define G2X_CUT_GAP_US 20000u            /* at most one rate cut per this */

void g2x_tx_init(g2x_tx *t, uint32_t count, uint8_t *acked, uint64_t *stamp, uint32_t *rq, uint8_t *queued,
                 double pps, double pps_min, double pps_max);
/* an ACK arrived */
void g2x_tx_on_ack(g2x_tx *t, const g2x_ack *a, uint64_t now_us);
/* the next seq to transmit (resend first), or -1 (window closed / all sent); *resend set */
int64_t g2x_tx_next(g2x_tx *t, uint64_t now_us, int *resend);
/* when the RTO fires if nothing is acknowledged (0: no timer running) */
uint64_t g2x_tx_rto_at(const g2x_tx *t);
void g2x_tx_sent(g2x_tx *t, uint32_t seq, int resend, uint64_t now);
int g2x_tx_done(const g2x_tx *t);
#endif
