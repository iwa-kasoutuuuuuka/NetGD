/* g2bench: M0 of docs/research/g2-perf-plan.md. Needs only a Dreamcast + BBA + dcload-ip
 * (no NetGD board). Measures
 *   A  /pc/ fread throughput for 1 / 4 / 16 / 64 KiB blocks (30 s per size, up to 32 MiB), plus raw read()
 *   B  SH-4 CRC-32 and FNV-1a throughput on 64 KiB buffers (the netgd-recv implementations)
 *   C  whether dcload /pc/ calls work from a second KOS thread, and what fread (thread) + CRC (main)
 *      gives compared with doing them one after the other
 * Never touches the GD-ROM drive: INIT_CDROM is off, no G1 / cdrom / ATA call is made.
 * Result: /pc/<host folder>/g2bench_<seq>.txt (key=value), rewritten after every phase.
 * See README_ja.md. */
#include <kos.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include "g2b_core.h"
#include "../netgd-recv/src/crc32.h"

KOS_INIT_FLAGS(INIT_DEFAULT & ~INIT_CDROM);

#define STR_(x) #x
#define STR(x)  STR_(x)
#ifdef HOST_DIR_RAW
#define HOST_DIR STR(HOST_DIR_RAW)
#else
#define HOST_DIR "/pc"
#endif
#define DATA_PATH   HOST_DIR "/g2bench_data.bin"
#define DATA_BYTES  (32u << 20)
#define PASS_LIMIT_US  (30ull * 1000000ull)   /* abort one read pass after 30 s (rate is still computed) */
#ifndef G2B_MAX_BS
#define G2B_MAX_BS  65536u                    /* largest block size tried (build.ps1 -MaxBs / $env:G2BENCH_MAXBS) */
#endif
#define OV_MAX_BYTES (8u << 20)               /* phase C: up to 8 MiB, less when /pc/ is slow */
#define OV_TARGET_S  10u                      /* ... about this many seconds per C run */
#define OV_WAIT_MS  60000

static char g_text[6144];
static g2b_kv g_kv;
static char g_path[96];

static uint64_t now_us(void) { return timer_us_gettime64(); }

static void flush_result(void)
{
    FILE *f = fopen(g_path, "wb");
    if (!f) { printf("[g2bench] cannot write %s\n", g_path); return; }
    fwrite(g_text, 1, g_kv.len, f);
    fclose(f);
}

/* first g2bench_NNN.txt that does not exist yet */
static int pick_seq(void)
{
    for (int n = 1; n < 1000; n++) {
        snprintf(g_path, sizeof g_path, HOST_DIR "/g2bench_%03d.txt", n);
        FILE *f = fopen(g_path, "rb");
        if (!f) return n;
        fclose(f);
    }
    return -1;
}

static void rate_keys(const char *pfx, uint64_t bytes, uint64_t us)
{
    char k[64];
    snprintf(k, sizeof k, "%s_bytes", pfx); g2b_kv_u64(&g_kv, k, bytes);
    snprintf(k, sizeof k, "%s_us", pfx);    g2b_kv_u64(&g_kv, k, us);
    snprintf(k, sizeof k, "%s_kbps", pfx);  g2b_kv_u64(&g_kv, k, g2b_kbps(bytes, us));
}

/* ---- phase A ---- */
static unsigned g_best_bs, g_best_kbps;   /* best working block size, used by phase C */

static void phase_read(unsigned bs, int raw)
{
    char pfx[32], k[64];
    snprintf(pfx, sizeof pfx, "%s_%uk", raw ? "raw" : "read", bs / 1024);
    if (bs < 1024) snprintf(pfx, sizeof pfx, "%s_%ub", raw ? "raw" : "read", bs);
    /* written before the first call: a dc-tool-ip that dies in this pass leaves this line */
    snprintf(k, sizeof k, "next_pass"); g2b_kv_str(&g_kv, k, pfx);
    flush_result();
    printf("[g2bench] pass %s ...\n", pfx);

    uint8_t *buf = memalign(32, bs);
    FILE *f = raw ? NULL : fopen(DATA_PATH, "rb");
    int fd = raw ? open(DATA_PATH, O_RDONLY) : -1;
    if (f) setvbuf(f, NULL, _IONBF, 0);   /* every fread is exactly one dcload read */
    if (!buf || (raw ? fd < 0 : !f)) {
        snprintf(k, sizeof k, "%s_status", pfx); g2b_kv_str(&g_kv, k, buf ? "open_failed" : "nomem");
        if (f) fclose(f);
        if (fd >= 0) close(fd);
        free(buf);
        return;
    }
    uint64_t bytes = 0, io_us = 0, max_us = 0, calls = 0, wall0 = now_us();
    uint32_t crc = 0;
    const char *status = "ok";
    while (bytes < DATA_BYTES) {
        uint64_t t0 = now_us();
        long n = raw ? read(fd, buf, bs) : (long)fread(buf, 1, bs, f);
        uint64_t d = now_us() - t0;
        io_us += d;
        if (d > max_us) max_us = d;
        calls++;
        if (n > 0) {
            if (bs == 65536) crc = ng_crc32(crc, buf, (size_t)n);   /* outside the timed region */
            bytes += (uint64_t)n;
        }
        if (n < (long)bs) { if (bytes < DATA_BYTES) status = "short_read"; break; }
        if (now_us() - wall0 > PASS_LIMIT_US) { status = "time_limit"; break; }
    }
    if (f) fclose(f);
    if (fd >= 0) close(fd);
    free(buf);
    rate_keys(pfx, bytes, io_us);
    snprintf(k, sizeof k, "%s_calls", pfx);       g2b_kv_u64(&g_kv, k, calls);
    snprintf(k, sizeof k, "%s_us_per_call", pfx); g2b_kv_u64(&g_kv, k, calls ? io_us / calls : 0);
    snprintf(k, sizeof k, "%s_max_call_us", pfx); g2b_kv_u64(&g_kv, k, max_us);
    snprintf(k, sizeof k, "%s_wall_us", pfx);     g2b_kv_u64(&g_kv, k, now_us() - wall0);
    snprintf(k, sizeof k, "%s_status", pfx);      g2b_kv_str(&g_kv, k, status);
    if (bs == 65536 && !raw) {
        snprintf(k, sizeof k, "%s_crc32", pfx); g2b_kv_hex32(&g_kv, k, crc);
        snprintf(k, sizeof k, "%s_crc_ok", pfx);
        g2b_kv_u64(&g_kv, k, bytes == DATA_BYTES && crc == g2b_expected_crc(DATA_BYTES / G2B_BLOCK));
    }
    unsigned kb = (unsigned)g2b_kbps(bytes, io_us);
    if (!raw && bytes && kb >= g_best_kbps) { g_best_kbps = kb; g_best_bs = bs; }
    printf("[g2bench] %s: %u KB/s (%lu bytes, %lu us/call, %s)\n", pfx, kb, (unsigned long)bytes,
           (unsigned long)(calls ? io_us / calls : 0), status);
    flush_result();
}

/* ---- phase B ---- */
static void phase_cpu(void)
{
    uint8_t *buf = memalign(32, G2B_BLOCK);
    if (!buf) { g2b_kv_str(&g_kv, "cpu_status", "nomem"); return; }
    g2b_gen_block(3, buf);
    /* self check: standard test vectors */
    g2b_kv_u64(&g_kv, "selfcheck_ok", ng_crc32(0, "123456789", 9) == 0xCBF43926u && ng_fnv1a(2166136261u, "a", 1) == 0xE40C292Cu);

    uint64_t iters = 0, t0 = now_us(), el;
    uint32_t crc = 0;
    do { crc = ng_crc32(crc, buf, G2B_BLOCK); iters++; el = now_us() - t0; } while (el < 1500000);
    rate_keys("cpu_crc32_64k", iters * G2B_BLOCK, el);

    uint32_t h = 2166136261u;
    iters = 0; t0 = now_us();
    do { h = ng_fnv1a(h, buf, G2B_BLOCK); iters++; el = now_us() - t0; } while (el < 1500000);
    rate_keys("cpu_fnv1a_64k", iters * G2B_BLOCK, el);

    /* both together (what recv_core does per chunk) */
    iters = 0; t0 = now_us();
    do { crc = ng_crc32(crc, buf, G2B_BLOCK); h = ng_fnv1a(h, buf, G2B_BLOCK); iters++; el = now_us() - t0; } while (el < 1500000);
    rate_keys("cpu_both_64k", iters * G2B_BLOCK, el);
    g2b_kv_hex32(&g_kv, "cpu_sink", crc ^ h);   /* keeps the compiler from dropping the loops */
    free(buf);
    printf("[g2bench] cpu done\n");
    flush_result();
}

/* ---- phase C ---- */
#define RING 3
static uint8_t *g_ring[RING];
static size_t g_got[RING];
static semaphore_t g_full, g_empty;
static volatile int g_prod_blocks;
static FILE *g_ovf;
static unsigned g_cbs, g_cblocks;   /* phase C block size and block count */

static void *producer(void *arg)
{
    (void)arg;
    for (unsigned i = 0; i < g_cblocks; i++) {
        sem_wait(&g_empty);
        g_got[i % RING] = fread(g_ring[i % RING], 1, g_cbs, g_ovf);
        g_prod_blocks = (int)(i + 1);
        sem_signal(&g_full);
        if (g_got[i % RING] < g_cbs) break;
    }
    return NULL;
}

static FILE *open_nb(void)
{
    FILE *f = fopen(DATA_PATH, "rb");
    if (f) setvbuf(f, NULL, _IONBF, 0);
    return f;
}

static void phase_overlap(void)
{
    /* block size = the best one phase A found; amount = about OV_TARGET_S seconds of reading */
    g_cbs = g_best_bs ? g_best_bs : 4096;
    uint64_t want_bytes = g_best_kbps ? (uint64_t)g_best_kbps * 1024ull * OV_TARGET_S : 64ull * g_cbs;
    if (want_bytes > OV_MAX_BYTES) want_bytes = OV_MAX_BYTES;
    if (want_bytes < 16ull * g_cbs) want_bytes = 16ull * g_cbs;
    g_cblocks = (unsigned)(want_bytes / g_cbs);
    const uint64_t total = (uint64_t)g_cblocks * g_cbs;
    g2b_kv_u64(&g_kv, "ov_block_bytes", g_cbs);
    g2b_kv_u64(&g_kv, "ov_blocks", g_cblocks);
    g2b_kv_str(&g_kv, "next_pass", "overlap");

    uint8_t *buf = memalign(32, g_cbs);
    for (int i = 0; i < RING; i++) g_ring[i] = memalign(32, g_cbs);
    if (!buf || !g_ring[0] || !g_ring[1] || !g_ring[2]) { g2b_kv_str(&g_kv, "overlap_status", "nomem"); return; }
    uint32_t want = g2b_expected_crc_bytes(total);
    uint64_t t0, us = 0, got;
    uint32_t crc, h;
    FILE *f;

    /* C0: CPU only on the same amount of data */
    memset(buf, 0x5a, g_cbs);
    crc = 0; h = 2166136261u; t0 = now_us();
    for (unsigned i = 0; i < g_cblocks; i++) { crc = ng_crc32(crc, buf, g_cbs); h = ng_fnv1a(h, buf, g_cbs); }
    rate_keys("ov_cpu_only", total, now_us() - t0);
    g2b_kv_hex32(&g_kv, "ov_cpu_sink", crc ^ h);

    /* C1: fread only (main thread) */
    f = open_nb();
    if (!f) { g2b_kv_str(&g_kv, "overlap_status", "open_failed"); return; }
    got = 0; t0 = now_us();
    for (unsigned i = 0; i < g_cblocks; i++) { size_t n = fread(buf, 1, g_cbs, f); got += n; if (n < g_cbs) break; }
    us = now_us() - t0;
    fclose(f);
    rate_keys("ov_read_only", got, us);
    flush_result();

    /* C2: serial = fread then CRC + FNV, one block at a time */
    f = open_nb();
    if (!f) { g2b_kv_str(&g_kv, "overlap_status", "open_failed"); return; }
    crc = 0; h = 2166136261u; got = 0; t0 = now_us();
    for (unsigned i = 0; i < g_cblocks; i++) {
        size_t n = fread(buf, 1, g_cbs, f);
        got += n;
        crc = ng_crc32(crc, buf, n);
        h = ng_fnv1a(h, buf, n);
        if (n < g_cbs) break;
    }
    us = now_us() - t0;
    fclose(f);
    rate_keys("ov_serial", got, us);
    g2b_kv_u64(&g_kv, "ov_serial_crc_ok", crc == want && got == total);
    g2b_kv_str(&g_kv, "overlap_status", "thread_test_running");   /* stays if the thread call hangs */
    flush_result();
    printf("[g2bench] overlap: starting the second-thread fread\n");

    /* C3: overlap = a thread freads, the main thread does CRC + FNV */
    g_ovf = open_nb();
    if (!g_ovf) { g2b_kv_str(&g_kv, "overlap_status", "open_failed"); return; }
    sem_init(&g_empty, RING);
    sem_init(&g_full, 0);
    g_prod_blocks = 0;
    crc = 0; h = 2166136261u; got = 0;
    t0 = now_us();
    kthread_t *th = thd_create(0, producer, NULL);
    const char *st = "ok";
    if (!th) {
        st = "thd_create_failed";
    } else {
        for (unsigned i = 0; i < g_cblocks; i++) {
            if (sem_wait_timed(&g_full, OV_WAIT_MS) != 0) { st = "timeout"; break; }
            size_t n = g_got[i % RING];
            crc = ng_crc32(crc, g_ring[i % RING], n);
            h = ng_fnv1a(h, g_ring[i % RING], n);
            got += n;
            sem_signal(&g_empty);
            if (n < g_cbs) break;
        }
        us = now_us() - t0;
        if (strcmp(st, "ok") == 0) thd_join(th, NULL);
    }
    rate_keys("ov_overlap", got, us);
    g2b_kv_u64(&g_kv, "ov_overlap_crc_ok", crc == want && got == total);
    g2b_kv_u64(&g_kv, "ov_overlap_producer_blocks", (uint64_t)g_prod_blocks);
    g2b_kv_str(&g_kv, "overlap_status", st);
    if (strcmp(st, "ok") == 0) fclose(g_ovf);   /* a stuck producer still owns the FILE */
    printf("[g2bench] overlap: %s\n", st);
    flush_result();
    for (int i = 0; i < RING; i++) free(g_ring[i]);
    free(buf);
}

int main(int argc, char **argv)
{
    (void)argc; (void)argv;
    printf("[g2bench] start (no GD-ROM access; INIT_CDROM off)\n");
    int seq = pick_seq();
    if (seq < 0) { printf("[g2bench] no free g2bench_NNN.txt number\n"); return 1; }
    g2b_kv_init(&g_kv, g_text, sizeof g_text);
    g2b_kv_str(&g_kv, "tool", "g2bench");
    g2b_kv_u64(&g_kv, "seq", (uint64_t)seq);
    g2b_kv_u64(&g_kv, "data_bytes", DATA_BYTES);
    g2b_kv_u64(&g_kv, "max_block_bytes", G2B_MAX_BS);
    g2b_kv_str(&g_kv, "status", "running");
    flush_result();
    printf("[g2bench] result file: %s\n", g_path);

    FILE *f = fopen(DATA_PATH, "rb");
    int have_data = f != NULL;
    if (f) fclose(f);
    g2b_kv_u64(&g_kv, "data_present", (uint64_t)have_data);
    if (!have_data) printf("[g2bench] %s missing (run mkdata.py / build.ps1): phases A and C skipped\n", DATA_PATH);

    /* B first: no network, cannot be lost to a dc-tool-ip failure */
    phase_cpu();
    if (have_data) {
        static const unsigned sizes[] = { 1024, 4096, 16384, 65536 };
        for (unsigned i = 0; i < sizeof sizes / sizeof sizes[0]; i++) {
            if (sizes[i] > G2B_MAX_BS) break;
            phase_read(sizes[i], 0);
            if (sizes[i] == 4096) phase_read(4096, 1);   /* raw read(): same call without stdio */
        }
        phase_overlap();
    }

    g2b_kv_str(&g_kv, "next_pass", "none");
    g2b_kv_str(&g_kv, "status", "done");   /* later key wins in the parser */
    g2b_kv_u64(&g_kv, "kv_overflow", (uint64_t)g_kv.err);
    flush_result();
    printf("[g2bench] done: %s\n", g_path);
    return 0;
}
