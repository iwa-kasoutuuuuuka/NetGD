/* netgd-menu (KOS build): start, then run the frames of the UI core.  NOT run on hardware. */
#include <kos.h>
#include <dc/video.h>
#include <dc/biosfont.h>
#include "nm_ui.h"
#include "nm_net.h"

/* No INIT_CDROM: the firmware serves the GD-ROM side itself, and the menu talks to it with raw ATAPI
   packets (fwxfer_kos.c), as netgd-recv does.  No INIT_NET: the network is started on demand (design D12), by hand with a
   static IP and no DHCP (net_kos.c, shared with netgd-t2; make NM_IP=0x...). */
KOS_INIT_FLAGS(INIT_DEFAULT & ~INIT_CDROM);

static nm_ui_t ui;

int main(int argc, char **argv)
{
    (void)argc; (void)argv;
    vid_set_mode(DM_640x480, PM_RGB565);
    vid_empty();
    bfont_set_background_color(0x0000);
#ifdef NM_RCVBUF
    nm_net_rcvbuf = NM_RCVBUF;      /* the TCP receive window (default NM_NET_RCVBUF_DEFAULT, 20 KiB) */
#endif
    nm_ui_init(&ui);
    for (;;) {
        nm_ui_step(&ui);
        thd_sleep(16);
    }
    return 0;
}
