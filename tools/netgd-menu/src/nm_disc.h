/* The UDP discovery of the transfer screen (design 8.1, step 9).  Pure text handling: the sockets are nm_net.h's.
     PC -> Dreamcast  "NGTP-DISCOVER 1"                    (anything after a space or a line end is ignored)
     Dreamcast -> PC  "NGTP-HERE 1 name=NetGD app=0.1.0 state=idle"
   state is "idle" (no PC is attached) or "busy" (a PC is attached: a second one would not get in).  The pass code is
   never in the reply, nor is the TCP port: the PC uses NGTP's 20039 (--port).  Nothing here has run on a Dreamcast. */
#ifndef NM_DISC_H
#define NM_DISC_H
#include <stddef.h>
#include <stdint.h>

#define NM_DISC_MAX 96          /* the longest reply */

/* 1 when `req` (n bytes, not terminated) is a version 1 discovery request */
int nm_disc_is_request(const char *req, size_t n);
/* The reply text for a request.  Returns its length, or 0 when `req` is not a request (nothing is to be sent). */
size_t nm_disc_reply(const char *req, size_t n, int busy, char *out, size_t outn);
/* Answer one waiting datagram, if any (nm_net_udp_recv / nm_net_udp_reply).  Returns 1 when a reply was sent. */
int nm_disc_service(int busy);

#endif
