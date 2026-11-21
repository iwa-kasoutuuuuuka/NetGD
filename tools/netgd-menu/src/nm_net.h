/* The network boundary (design 4.4): KOS net_kos.c, host test/net_host.c.  One connection at a time.
   Nothing here has run on a Dreamcast. */
#ifndef NM_NET_H
#define NM_NET_H
#include <stddef.h>
#include <stdint.h>

#define NM_NET_NODEV  1         /* nm_net_up: no BBA */
#define NM_NET_NOADDR 2         /* nm_net_up: no IP address (DHCP failed) */

/* Start the network (the first call) and give the IP address as text.  0, NM_NET_NODEV or NM_NET_NOADDR. */
int nm_net_up(char *ip, size_t n);
int nm_net_listen(uint16_t port);                       /* 0 / -1 */
int nm_net_accept(uint32_t timeout_ms);                 /* 1 a client / 0 timeout / -1 error */
/* Receive exactly n bytes.  0 = all of them; NM_NET_AGAIN = not one byte arrived within timeout_ms (nothing was
   consumed: try again later; an addition to design 4.4, which has only 0 / -1); -1 = the peer closed, an error, or
   the time ran out in the middle of the data.  timeout_ms is the wait for the FIRST byte; once data has started to
   arrive the rest may take up to max(timeout_ms, 5000) ms (a 16-byte header can come in pieces). */
#define NM_NET_AGAIN 1
int nm_net_recv_all(void *buf, uint32_t n, uint32_t timeout_ms);
int nm_net_send_all(const void *buf, uint32_t n);       /* 0 / -1 */
void nm_net_close_client(void);
void nm_net_stop(void);                                 /* stop listening (the BBA stays initialised) */

#endif
