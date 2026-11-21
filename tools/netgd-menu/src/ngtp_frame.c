#include <string.h>
#include "ngtp.h"
#include "crc32.h"

int ngtp_hdr_parse(const uint8_t *h, ngtp_hdr_t *out)
{
    if (memcmp(h, "NGT1", 4) || h[6] || h[7]) return NGTP_E_MAGIC;
    out->type = h[4];
    out->flags = h[5];
    out->seq = ngtp_get32(h + 8);
    out->len = ngtp_get32(h + 12);
    if (out->len > NGTP_MAX_PAYLOAD) return NGTP_E_LENGTH;
    return NGTP_OK;
}

uint32_t ngtp_frame_crc(const uint8_t *hdr16, const uint8_t *payload, uint32_t n)
{
    return ng_crc32(ng_crc32(0, hdr16, NGTP_HDR_LEN), payload, n);
}

int ngtp_frame_check(const uint8_t *hdr16, const uint8_t *payload, uint32_t n, const uint8_t *crc4)
{
    return ngtp_frame_crc(hdr16, payload, n) == ngtp_get32(crc4) ? NGTP_OK : NGTP_E_CRC;
}

size_t ngtp_frame_build(uint8_t *out, size_t outn, uint8_t type, uint8_t flags, uint32_t seq, const void *payload, uint32_t n)
{
    size_t total = (size_t)NGTP_HDR_LEN + n + 4;
    if (n > NGTP_MAX_PAYLOAD || outn < total) return 0;
    memcpy(out, "NGT1", 4);
    out[4] = type;
    out[5] = flags;
    out[6] = out[7] = 0;
    ngtp_put32(out + 8, seq);
    ngtp_put32(out + 12, n);
    if (n) memmove(out + NGTP_HDR_LEN, payload, n);
    ngtp_put32(out + NGTP_HDR_LEN + n, ngtp_frame_crc(out, out + NGTP_HDR_LEN, n));
    return total;
}

int ngtp_is_request(uint8_t type)
{
    return type >= NGTP_HELLO && type <= NGTP_PING;
}
