/* KOS network layer: a placeholder until menu step 8.  Reports "no BBA", so the menu says so and goes on. */
#include "nm_net.h"

int nm_net_up(char *ip, size_t n)
{
    if (n) ip[0] = 0;
    return NM_NET_NODEV;
}

int nm_net_listen(uint16_t port) { (void)port; return -1; }
int nm_net_accept(uint32_t timeout_ms) { (void)timeout_ms; return -1; }
int nm_net_recv_all(void *buf, uint32_t n, uint32_t timeout_ms) { (void)buf; (void)n; (void)timeout_ms; return -1; }
int nm_net_send_all(const void *buf, uint32_t n) { (void)buf; (void)n; return -1; }
void nm_net_close_client(void) {}
void nm_net_stop(void) {}
