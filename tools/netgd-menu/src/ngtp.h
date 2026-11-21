/* NGTP version 1: the frames of the PC -> Dreamcast transfer (design 8.2, 8.3).  Little endian.
     0  4  "NGT1"        4  1 type       5  1 flags      6  2 zero
     8  4  sequence      12 4 payload length n (<= NGTP_MAX_PAYLOAD)
     16 n  payload       16+n 4  CRC-32 (zlib) of everything before it
   Nothing here has run on a Dreamcast. */
#ifndef NGTP_H
#define NGTP_H
#include <stddef.h>
#include <stdint.h>

#define NGTP_VERSION       1
#define NGTP_PORT          20039
#define NGTP_HDR_LEN       16
#define NGTP_MAX_DATA      65536
#define NGTP_MAX_PAYLOAD   (65536 + 16)
#define NGTP_MAX_FRAME     (NGTP_HDR_LEN + NGTP_MAX_PAYLOAD + 4)

/* PC -> Dreamcast */
#define NGTP_HELLO   0x01
#define NGTP_STATFS  0x02
#define NGTP_LIST    0x03
#define NGTP_PUT     0x04
#define NGTP_DATA    0x05
#define NGTP_DELETE  0x06
#define NGTP_VERIFY  0x07
#define NGTP_FINISH  0x08
#define NGTP_PING    0x09
/* Dreamcast -> PC */
#define NGTP_HELLO_OK 0x81
#define NGTP_PONG     0x89
#define NGTP_LINE     0x90      /* flags bit0: a progress line that may be dropped */
#define NGTP_DONE     0x91
#define NGTP_WANT     0x92
#define NGTP_ERROR    0x7F

#define NGTP_F_THROTTLED 0x01   /* LINE */
#define NGTP_F_ABORT     0x01   /* DATA: the PC asks to stop (after this data) */

typedef struct {
    uint8_t type, flags;
    uint32_t seq, len;
} ngtp_hdr_t;

#define NGTP_OK        0
#define NGTP_E_MAGIC  (-1)
#define NGTP_E_LENGTH (-2)
#define NGTP_E_CRC    (-3)

static inline void ngtp_put16(uint8_t *p, uint32_t v) { p[0] = (uint8_t)v; p[1] = (uint8_t)(v >> 8); }
static inline void ngtp_put32(uint8_t *p, uint32_t v) { ngtp_put16(p, v); ngtp_put16(p + 2, v >> 16); }
static inline void ngtp_put64(uint8_t *p, uint64_t v) { ngtp_put32(p, (uint32_t)v); ngtp_put32(p + 4, (uint32_t)(v >> 32)); }
static inline uint32_t ngtp_get16(const uint8_t *p) { return (uint32_t)p[0] | (uint32_t)p[1] << 8; }
static inline uint32_t ngtp_get32(const uint8_t *p) { return ngtp_get16(p) | ngtp_get16(p + 2) << 16; }
static inline uint64_t ngtp_get64(const uint8_t *p) { return ngtp_get32(p) | (uint64_t)ngtp_get32(p + 4) << 32; }

/* the 16 header bytes: magic, zero field and length are checked.  0 or NGTP_E_MAGIC / NGTP_E_LENGTH */
int ngtp_hdr_parse(const uint8_t *h, ngtp_hdr_t *out);
/* CRC-32 over the 16 header bytes and the payload (what the 4 bytes after the payload must hold) */
uint32_t ngtp_frame_crc(const uint8_t *hdr16, const uint8_t *payload, uint32_t n);
/* 0 or NGTP_E_CRC: crc4 are the 4 bytes that followed the payload */
int ngtp_frame_check(const uint8_t *hdr16, const uint8_t *payload, uint32_t n, const uint8_t *crc4);
/* header + payload + CRC into out.  Returns the total length, 0 if out is too small or n is too large. */
size_t ngtp_frame_build(uint8_t *out, size_t outn, uint8_t type, uint8_t flags, uint32_t seq, const void *payload, uint32_t n);
/* a request type the server knows */
int ngtp_is_request(uint8_t type);

#endif
