#include <stdio.h>
#include <string.h>
#include "nm.h"
#include "nm_net.h"
#include "nm_disc.h"

#define REQ "NGTP-DISCOVER 1"

int nm_disc_is_request(const char *req, size_t n)
{
    size_t l = sizeof REQ - 1;
    if (n < l || memcmp(req, REQ, l)) return 0;
    /* "NGTP-DISCOVER 10" is another version, not ours */
    return n == l || req[l] == ' ' || req[l] == '\r' || req[l] == '\n' || req[l] == 0 || req[l] == '\t';
}

size_t nm_disc_reply(const char *req, size_t n, int busy, char *out, size_t outn)
{
    int w;
    if (!nm_disc_is_request(req, n)) return 0;
    w = snprintf(out, outn, "NGTP-HERE 1 name=NetGD app=%s state=%s", NM_VERSION, busy ? "busy" : "idle");
    return (w > 0 && (size_t)w < outn) ? (size_t)w : 0;
}

int nm_disc_service(int busy)
{
    char req[64], out[NM_DISC_MAX];
    int n = nm_net_udp_recv(req, sizeof req);
    size_t l;
    if (n <= 0) return 0;
    l = nm_disc_reply(req, (size_t)n, busy, out, sizeof out);
    return l && nm_net_udp_reply(out, l) == 0;
}
