/* t2_host: the netgd-t2 core on the host, for test/run_tests.py.
     t2_host serve <img> <sectors> --port P --code C [--want N] [--depth N] [--nofw] [--caps N] [--no-drive] [--cut N]
             [--idle-close-ms N] [--idle-exit-ms N] [--stopafter N] [--hold-b-ms N] [--confirm allow|deny]
             [--leave-after-ms N] [--nobba] [--timeout-ms N] [--trace] [--dump]
   The card is a disk image (netgd-recv test/dev_file.c), the firmware the packet-level model (test/fwctl_sim.c), the
   network a real 127.0.0.1 socket (netgd-menu test/net_host.c), the clock the real one.  Prints LISTENING when the port is
   open, T2-END at the end, and a SIM line (what the model saw) at exit. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "t2_app.h"
#include "nm_plat.h"
#include "plat_host.h"
#include "simdisk.h"

static t2_app_t app;
static int confirm_mode;            /* 1 = A while the question is on the screen, 2 = B */
static uint32_t leave_after_ms;     /* B (not busy) this long after LISTENING; 0 = never */
static uint64_t t_listen;

static void sim_summary(void)
{
    printf("SIM cmd_writes=%llu sector_writes=%llu begin=%d end=%d unsess=%d violations=%llu packets=%d endmode=%d "
           "mount_in_session=%d session=%d\n",
           (unsigned long long)sim_cmd_writes, (unsigned long long)sim_sector_writes, sim_begin_calls, sim_end_calls,
           sim_unsessioned_writes, (unsigned long long)sim_violations, sim_packets, sim_end_mode, sim_mount_in_session, sim_session);
    fflush(stdout);
}

static uint32_t host_buttons(void)
{
    char row[NM_COLS + 1];
    nm_screen_row_text(&nm_host_screen, 10, row);
    if (strstr(row, "Hold A")) return confirm_mode == 1 ? NM_BTN_A : confirm_mode == 2 ? NM_BTN_B : 0;
    if (leave_after_ms && t_listen && !app.busy && nm_host_real_ms() - t_listen >= leave_after_ms) return NM_BTN_B;
    return 0;
}

static void dump(void)
{
    char row[NM_COLS + 1];
    for (int r = 0; r < NM_ROWS; r++) {
        nm_screen_row_text(&nm_host_screen, r, row);
        printf("S%02d|%s\n", r, row);
    }
}

int main(int argc, char **argv)
{
    t2_cfg_t cfg;
    const char *img;
    uint32_t sectors, timeout_ms = 120000;
    int do_dump = 0;
    uint64_t t0;
    if (argc < 4 || strcmp(argv[1], "serve")) {
        fprintf(stderr, "usage: t2_host serve <img> <sectors> --port P --code C [options]\n");
        return 2;
    }
    img = argv[2];
    sectors = (uint32_t)strtoul(argv[3], NULL, 10);
    t2_cfg_defaults(&cfg, 0);
    for (int i = 4; i < argc; i++)                  /* first: the defaults of the patched-TCP build, then the options */
        if (!strcmp(argv[i], "--tcpfix")) t2_cfg_defaults(&cfg, 1);
    for (int i = 4; i < argc; i++) {
        const char *a = argv[i], *v = i + 1 < argc ? argv[i + 1] : NULL;
        if (!strcmp(a, "--port") && v) { nm_host_net_port = (uint16_t)atoi(v); i++; }
        else if (!strcmp(a, "--code") && v) { cfg.code = (uint16_t)atoi(v); i++; }
        else if (!strcmp(a, "--want") && v) { cfg.want_bytes = (uint32_t)atol(v); i++; }
        else if (!strcmp(a, "--depth") && v) { cfg.depth = atoi(v); i++; }
        else if (!strcmp(a, "--tcpfix")) { }
        else if (!strcmp(a, "--nofw")) sim_fw_dead = 1;
        else if (!strcmp(a, "--caps") && v) { sim_caps = (uint8_t)atoi(v); i++; }
        else if (!strcmp(a, "--no-drive")) cfg.drive = 0;
        else if (!strcmp(a, "--cut") && v) { sim_cut_after = atoll(v); i++; }
        else if (!strcmp(a, "--idle-close-ms") && v) { cfg.idle_close_ms = (uint32_t)atol(v); i++; }
        else if (!strcmp(a, "--idle-exit-ms") && v) { cfg.idle_exit_ms = (uint32_t)atol(v); i++; }
        else if (!strcmp(a, "--stopafter") && v) { nm_host_stop_after_polls = atoi(v); i++; }
        else if (!strcmp(a, "--hold-b-ms") && v) { nm_host_b_hold_ms = (uint32_t)atol(v); i++; }
        else if (!strcmp(a, "--confirm") && v) { confirm_mode = !strcmp(v, "allow") ? 1 : 2; i++; }
        else if (!strcmp(a, "--leave-after-ms") && v) { leave_after_ms = (uint32_t)atol(v); i++; }
        else if (!strcmp(a, "--nobba")) nm_host_net_nodev = 1;
        else if (!strcmp(a, "--timeout-ms") && v) { timeout_ms = (uint32_t)atol(v); i++; }
        else if (!strcmp(a, "--trace")) sim_trace = 1;
        else if (!strcmp(a, "--dump")) do_dump = 1;
        else { fprintf(stderr, "unknown option %s\n", a); return 2; }
    }
    if (sim_open(img, sectors)) { printf("cannot open image\n"); return 2; }
    sim_protect_lba = 2048;
    nm_host_realtime = 1;
    nm_host_buttons_hook = host_buttons;
    atexit(sim_summary);
    t2_init(&app, &cfg);
    t0 = nm_host_real_ms();
    for (;;) {
        int run = t2_step(&app);
        if (app.state == T2_S_SERVE && !t_listen) {
            t_listen = nm_host_real_ms();
            printf("LISTENING %u code %04u want %u depth %d\n", (unsigned)nm_host_net_port, (unsigned)app.srv.code,
                   (unsigned)app.srv.want_max, app.srv.depth);
            fflush(stdout);
        }
        if (sim_dead) { printf("POWERCUT\n"); fflush(stdout); return 0; }
        if (!run) break;
        if (nm_host_real_ms() - t0 > timeout_ms) { printf("T2-TIMEOUT\n"); break; }
        nm_host_real_sleep(1);
    }
    printf("T2-END state=%d result=%d wants=%u data=%u fw_checked=%d fw_ok=%d\n", app.state, app.result,
           (unsigned)app.srv.wants, (unsigned)app.srv.data_frames, app.fw_checked, app.fw_ok);
    if (do_dump) dump();
    fflush(stdout);
    return 0;
}
