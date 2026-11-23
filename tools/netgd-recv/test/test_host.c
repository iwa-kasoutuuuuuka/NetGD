/* Host test driver for netgd-recv's core.  Driven by test/run_tests.py.
     test_host selftest
     test_host mkimage <img> <sectors> <cluster_bytes>
     test_host run <img> <sectors> <pcroot> <jobdir(/pc/...)> [options]
     test_host dump <img> <sectors> <outdir>
     test_host addfile <img> <sectors> <fspath> <localfile>
     test_host mkdirs <img> <sectors> <prefix> <count>
     test_host light <img> <sectors>      (light mount + ngfs_list + a refused write, then the full mount for comparison)
     test_host gdemu <n> [slots...] [--ua] [--notready N]  (fwctl menu commands against the firmware model, prints the packets)
     test_host hang                       (never returns: for the time-limit test of run_tests.py)
   options: --cut N --ckp N --read N --readback 0|1 --slow --flip K --caps C --frag --force
            --stopafter N --pollms N --quiet --rflip K --rflip-all --rflipn K --rflipn-all --mountms N
            --ua --failend2 --rflipn-from K --headroom BYTES */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifdef _WIN32
#include <direct.h>
#define ng_mkdir(p) _mkdir(p)
#else
#include <sys/stat.h>
#include <sys/types.h>
#define ng_mkdir(p) mkdir((p), 0777)
#endif
#include "ff.h"
#include "ng.h"
#include "ngfs.h"
#include "ngdev.h"
#include "crc32.h"
#include "manifest.h"
#include "xport_pc.h"
#include "ng_plat.h"
#include "simdisk.h"
#include "fwctl.h"

extern void (*ng_pc_cfg_hook)(void);
extern int ng_pc_pad_healthy;
extern int sim_b_stuck;
extern ngblk_stats_t ngblk_stats;

static struct { long ckp, read; int readback, slow, frag, force; long mountms; long headroom; } opt = { -1, -1, -1, 0, 0, 0, -1, -1 };

static void cfg_hook(void)
{
    if (opt.ckp > 0) ng_cfg.ckp_bytes = (uint32_t)opt.ckp;
    if (opt.read > 0) ng_cfg.read_bytes = (uint32_t)opt.read;
    if (opt.readback >= 0) ng_cfg.readback = opt.readback;
    if (opt.slow) ng_cfg.force_slow = 1;
    if (opt.frag) ng_cfg.allow_frag = 1;
    if (opt.force) ng_cfg.force = 1;
    if (opt.headroom >= 0) ng_cfg.headroom = (uint32_t)opt.headroom;
    if (opt.mountms >= 0) ng_cfg.mount_line_ms = (uint32_t)opt.mountms;
}

static void phase_hook(const char *p) { printf("PHASE %s %llu\n", p, (unsigned long long)sim_sector_writes); }

static int selftest(void)
{
    int bad = 0;
    ng_manifest_t m;
    char err[96];
    static const char good[] =
        "NETGD-MANIFEST 1\nimage 02\ntype gdi\nfile 10 0000000A disc.gdi\nfile 20 000000FF track01.bin\n"
        "descriptor disc.gdi\nend\n";
    if (ng_crc32(0, "123456789", 9) != 0xCBF43926u) { printf("crc32 vector\n"); bad++; }
    if (ng_crc32(ng_crc32(0, "12345", 5), "6789", 4) != 0xCBF43926u) { printf("crc32 chain\n"); bad++; }
    if (ng_fnv1a(NG_FNV_INIT, "a", 1) != 0xE40C292Cu) { printf("fnv1a vector\n"); bad++; }
    if (ng_manifest_parse(good, strlen(good), &m, err, sizeof err) || m.nfiles != 2 || m.descriptor != 0 ||
        m.total_bytes != 30) { printf("parse good: %s\n", err); bad++; }
    {
        const char *rej[] = {
            "NETGD-MANIFEST 2\nimage 02\ntype gdi\nfile 1 00000000 a.gdi\ndescriptor a.gdi\nend\n",
            "NETGD-MANIFEST 1\nimage 02\ntype gdi\nfile 1 00000000 a.gdi\ndescriptor a.gdi\n",              /* no end */
            "NETGD-MANIFEST 1\nimage 02\ntype gdi\nfile 1 00000000 a.gdi\ndescriptor b.gdi\nend\n",
            "NETGD-MANIFEST 1\nimage 02\ntype gdi\nfile 1 00000000 averylongname.gdi\ndescriptor averylongname.gdi\nend\n",
            "NETGD-MANIFEST 1\nimage a/b/c/d\ntype gdi\nfile 1 00000000 a.gdi\ndescriptor a.gdi\nend\n",
            "NETGD-MANIFEST 1\nimage ..\ntype gdi\nfile 1 00000000 a.gdi\ndescriptor a.gdi\nend\n",
            "NETGD-MANIFEST 1\nimage 02\ntype gdi\nfile 1 00000000 a.gdi\nfile 1 00000000 A.GDI\ndescriptor a.gdi\nend\n",
            "NETGD-MANIFEST 1\nimage 02\ntype cdi\nfile 1 00000000 a.gdi\ndescriptor a.gdi\nend\n",
            "NETGD-MANIFEST 1\r\nimage 02\n",
            /* a data file named like the descriptor's .NGP placeholder */
            "NETGD-MANIFEST 1\nimage 02\ntype gdi\nfile 1 00000000 a.gdi\nfile 1 00000000 A.ngp\ndescriptor a.gdi\nend\n",
        };
        for (unsigned i = 0; i < sizeof rej / sizeof rej[0]; i++)
            if (!ng_manifest_parse(rej[i], strlen(rej[i]), &m, err, sizeof err)) { printf("reject %u accepted\n", i); bad++; }
    }
    {
        /* path validation shared by put / delete / verify / list / finish mount */
        const char *badp[] = { "/01", "01/", "01.", "01 ", "01//02", "a//b", "..", "01/..", "", "a/b/c/d", " 01", "a:b" };
        const char *okp[] = { "01", "GAMES/X", "Crazy Taxi (US)", "a.b/c" };
        for (unsigned i = 0; i < sizeof badp / sizeof badp[0]; i++)
            if (!ng_path_ok(badp[i], err, sizeof err)) { printf("path '%s' accepted\n", badp[i]); bad++; }
        for (unsigned i = 0; i < sizeof okp / sizeof okp[0]; i++)
            if (ng_path_ok(okp[i], err, sizeof err)) { printf("path '%s' rejected: %s\n", okp[i], err); bad++; }
        {
            char o[64];
            static const char *const in[] = { "/GAMES//X/", "01.", "01 ", "//", "a/./b ", "\\a\\b" };
            static const char *const want[] = { "GAMES/X", "01", "01", "", "a/b", "a/b" };
            for (unsigned i = 0; i < sizeof in / sizeof in[0]; i++) {
                ngfs_norm_path(in[i], o, sizeof o);
                if (strcmp(o, want[i])) { printf("norm '%s' -> '%s' (want '%s')\n", in[i], o, want[i]); bad++; }
            }
        }
    }
    printf(bad ? "SELFTEST FAIL\n" : "SELFTEST OK\n");
    return bad ? 1 : 0;
}

static int mkimage(const char *img, uint32_t sectors, uint32_t cluster)
{
    static BYTE work[FF_MAX_SS * 4];
    MKFS_PARM p = { FM_FAT32 | FM_SFD, 2, 0, 0, cluster };
    FRESULT r;
    uint8_t mbr[512] __attribute__((aligned(32)));
    remove(img);
    if (sim_open_ex(img, sectors, 1)) return 1;
    ngdev_init();
    sim_part_off = 2048;
    r = f_mkfs("0:", &p, work, sizeof work);
    sim_part_off = 0;
    if (r != FR_OK) { printf("f_mkfs failed %d\n", (int)r); return 1; }
    memset(mbr, 0, sizeof mbr);
    mbr[446 + 0] = 0x00; mbr[446 + 1] = 0xFE; mbr[446 + 2] = 0xFF; mbr[446 + 3] = 0xFF; mbr[446 + 4] = 0x0C;
    mbr[446 + 5] = 0xFE; mbr[446 + 6] = 0xFF; mbr[446 + 7] = 0xFF;
    {
        uint32_t st = 2048, sz = sectors - 2048;
        for (int i = 0; i < 4; i++) { mbr[446 + 8 + i] = (uint8_t)(st >> (8 * i)); mbr[446 + 12 + i] = (uint8_t)(sz >> (8 * i)); }
    }
    mbr[510] = 0x55; mbr[511] = 0xAA;
    if (ngdev_write(0, 1, mbr)) return 1;
    sim_close();
    printf("mkimage ok\n");
    return 0;
}

static void walk(const char *fsdir, const char *outdir)
{
    DIR d;
    FILINFO fi;
    if (f_opendir(&d, fsdir) != FR_OK) return;
    while ((fi.fname[0] = 0, f_readdir(&d, &fi)) == FR_OK && fi.fname[0]) {
        char fp[400], op[400];
        snprintf(fp, sizeof fp, "%s%s%s", fsdir, fsdir[strlen(fsdir) - 1] == '/' ? "" : "/", fi.fname);
        snprintf(op, sizeof op, "%s%s%s", outdir, fsdir[2] ? "" : "", fp + 2);
        if (fi.fattrib & AM_DIR) {
            ng_mkdir(op);
            printf("DIR %s\n", fp + 2);
            walk(fp, outdir);
        } else {
            FIL f;
            FILE *o = fopen(op, "wb");
            static uint8_t buf[65536];
            UINT got;
            uint32_t crc = 0;
            printf("FILE %s %lu\n", fp + 2, (unsigned long)fi.fsize);
            if (f_open(&f, fp, FA_READ) == FR_OK) {
                while (f_read(&f, buf, sizeof buf, &got) == FR_OK && got) {
                    if (o) fwrite(buf, 1, got, o);
                    crc = ng_crc32(crc, buf, got);
                }
                f_close(&f);
            }
            printf("CRC %s %08X\n", fp + 2, (unsigned)crc);
            if (o) fclose(o);
        }
    }
    f_closedir(&d);
}

/* fill the volume with `fsz`-byte files, then delete every other one: free space only in holes */
static int fragment(const char *img, uint32_t sectors, uint32_t fsz)
{
    static uint8_t buf[65536];
    char why[96], name[32];
    int n = 0;
    if (fsz > sizeof buf) return 1;
    if (sim_open(img, sectors)) return 1;
    if (ngfs_mount(why, sizeof why, NULL, NULL)) { printf("MOUNT FAILED %s\n", why); return 1; }
    for (;;) {
        FIL f;
        UINT bw = 0;
        FRESULT r;
        snprintf(name, sizeof name, "0:/F%04d", n);
        if (f_open(&f, name, FA_CREATE_ALWAYS | FA_WRITE) != FR_OK) break;
        r = f_write(&f, buf, fsz, &bw);
        f_close(&f);
        if (r != FR_OK || bw < fsz) { f_unlink(name); break; }
        n++;
    }
    for (int i = 1; i < n; i += 2) {
        snprintf(name, sizeof name, "0:/F%04d", i);
        f_unlink(name);
    }
    ngfs_unmount();
    sim_close();
    printf("fragment ok (%d files)\n", n);
    return 0;
}

/* addfile <img> <sectors> <fspath> <localfile>: copy a host file into the FAT volume (folders are created) */
static int addfile(const char *img, uint32_t sectors, const char *fspath, const char *local)
{
    char why[96], path[300];
    FIL f;
    FILE *in = fopen(local, "rb");
    static uint8_t buf[4096];
    size_t n;
    UINT bw;
    if (!in) return 1;
    if (sim_open(img, sectors)) return 1;
    if (ngfs_mount(why, sizeof why, NULL, NULL)) { printf("MOUNT FAILED %s\n", why); return 1; }
    snprintf(path, sizeof path, "0:%s", fspath);
    if (strrchr(path, '/') > path + 2) {
        char dir[300];
        snprintf(dir, sizeof dir, "%.*s", (int)(strrchr(path, '/') - path), path);
        ngfs_mkdir_p(dir);
    }
    if (f_open(&f, path, FA_CREATE_ALWAYS | FA_WRITE) != FR_OK) { printf("cannot create %s\n", path); return 1; }
    while ((n = fread(buf, 1, sizeof buf, in)) > 0) f_write(&f, buf, (UINT)n, &bw);
    f_close(&f);
    fclose(in);
    ngfs_unmount();
    sim_close();
    printf("addfile ok\n");
    return 0;
}

/* wr256 <img> <sectors>: one 256-sector ngblk_write with the firmware check on.  The KOS driver may split
   such a write (255 + 1) and the firmware STATUS only describes the last command, so the app must issue
   128-sector commands itself (R89 C-A): two commands, no FNV mismatch. */
static int wr256(const char *img, uint32_t sectors)
{
    static uint8_t buf[256 * 512];
    int rc;
    if (sim_open(img, sectors)) return 1;
    for (size_t i = 0; i < sizeof buf; i++) buf[i] = (uint8_t)(i * 7 + 3);
    ngblk_check_fnv = 1;
    sim_session = 1;
    rc = ngblk_write(4000, 256, buf);
    printf("WR256 rc=%d cmds=%llu fnvbad=%u\n", rc, (unsigned long long)sim_cmd_writes, (unsigned)ngblk_stats.fnv_mismatches);
    sim_close();
    return rc ? 1 : 0;
}

/* mkdirs <img> <sectors> <prefix> <count>: folders <prefix>000 .. */
static int mkdirs(const char *img, uint32_t sectors, const char *prefix, int count)
{
    char why[96], path[300];
    if (sim_open(img, sectors)) return 1;
    if (ngfs_mount(why, sizeof why, NULL, NULL)) { printf("MOUNT FAILED %s\n", why); return 1; }
    for (int i = 0; i < count; i++) {
        snprintf(path, sizeof path, "0:/%s%03d", prefix, i);
        if (f_mkdir(path) != FR_OK) { printf("mkdir %s failed\n", path); return 1; }
    }
    ngfs_unmount();
    sim_close();
    printf("mkdirs ok\n");
    return 0;
}

static void light_cb(void *ctx, const char *state, const char *path)
{
    (void)state; (void)path;
    ++*(int *)ctx;
}

/* light <img> <sectors>: the menu's read-only mount.  Prints the read commands of each phase. */
static int light(const char *img, uint32_t sectors)
{
    char why[96];
    int n = 0, rc;
    uint64_t r0, r1, r2;
    static uint8_t buf[512];
    if (sim_open(img, sectors)) return 1;
    ngdev_init();
    r0 = sim_cmd_reads;
    if (ngfs_mount_light(why, sizeof why)) { printf("LIGHT MOUNT FAILED %s\n", why); return 1; }
    r1 = sim_cmd_reads;
    rc = (int)ngfs_list("/", light_cb, &n);
    printf("LIGHT mount_rcmds=%llu list_rc=%d folders=%d list_rcmds=%llu readonly=%d\n", (unsigned long long)(r1 - r0),
           rc, n, (unsigned long long)(sim_cmd_reads - r1), ngblk_readonly);
    rc = ngblk_write(5000, 1, buf);
    printf("LIGHT write rc=%d cmd_writes=%llu sector_writes=%llu unsessioned=%d\n", rc, (unsigned long long)sim_cmd_writes,
           (unsigned long long)sim_sector_writes, sim_unsessioned_writes);
    ngfs_unmount();
    printf("LIGHT after_unmount readonly=%d\n", ngblk_readonly);
    r2 = sim_cmd_reads;
    if (ngfs_mount(why, sizeof why, NULL, NULL)) { printf("FULL MOUNT FAILED %s\n", why); return 1; }
    printf("FULL mount_rcmds=%llu readonly=%d\n", (unsigned long long)(sim_cmd_reads - r2), ngblk_readonly);
    ngfs_unmount();
    sim_close();
    return 0;
}

/* gdemu <n> [slot...] [--ua] [--notready N]: every menu command of fwctl.c against the model, packets traced */
static int gdemu(int n, int argc, char **argv)
{
    uint8_t sense = 0xEE;
    uint32_t boot = 0, fw = 0;
    int rc, k = 0;
    for (int i = 0; i < argc; i++) {
        if (!strcmp(argv[i], "--ua")) { sim_ua_mode = 1; sim_ua_pending = 1; }
        else if (!strcmp(argv[i], "--notready") && i + 1 < argc) sim_notready_polls = atoi(argv[++i]);
        else if (k < 63) sim_slots[k++] = atoi(argv[i]);
    }
    sim_trace = 1;
    rc = fw_gdemu_mount((uint16_t)n, &sense);
    printf("MOUNT rc=%d sense=%u mounted=%d\n", rc, sense, sim_mounted_number);
    rc = fw_gdemu_step(0, &sense);
    printf("STEP_NEXT rc=%d sense=%u mounted=%d\n", rc, sense, sim_mounted_number);
    rc = fw_gdemu_step(1, &sense);
    printf("STEP_PREV rc=%d sense=%u mounted=%d\n", rc, sense, sim_mounted_number);
    rc = fw_gdemu_version(&boot, &fw);
    printf("VERSION rc=%d boot=%08X fw=%08X\n", rc, (unsigned)boot, (unsigned)fw);
    rc = fw_mount_named("ABCDEFGHIJKLM", &sense);
    printf("NAMED rc=%d sense=%u named_mounts=%d\n", rc, sense, sim_named_mounts);
    rc = fw_mount_default(&sense);
    printf("DEFAULT rc=%d sense=%u default_mounts=%d\n", rc, sense, sim_default_mounts);
    for (int i = 0; i < 4; i++) {
        rc = fw_test_unit_ready(&sense);
        printf("TUR rc=%d sense=%u\n", rc, sense);
    }
    printf("COUNTS packets=%d ua_served=%d mount_in_session=%d\n", sim_packets, sim_ua_served, sim_mount_in_session);
    return 0;
}

static int dump(const char *img, uint32_t sectors, const char *outdir)
{
    char why[96];
    if (sim_open(img, sectors)) return 1;
    if (ngfs_mount(why, sizeof why, NULL, NULL)) { printf("MOUNT FAILED %s\n", why); return 1; }
    ng_mkdir(outdir);
    walk("0:/", outdir);
    ngfs_unmount();
    sim_close();
    return 0;
}

int main(int argc, char **argv)
{
    int rc, i;
    uint32_t sectors;
    if (argc < 2) return 2;
    ng_cfg_defaults();
    if (!strcmp(argv[1], "selftest")) return selftest();
    if (!strcmp(argv[1], "gdemu") && argc >= 3) return gdemu(atoi(argv[2]), argc - 3, argv + 3);
    if (!strcmp(argv[1], "hang")) {
        volatile unsigned spin = 0;
        printf("hanging\n");
        fflush(stdout);
        for (;;) spin++;
    }
    if (!strcmp(argv[1], "light") && argc >= 4) return light(argv[2], (uint32_t)strtoul(argv[3], NULL, 10));
    if (argc < 5) return 2;
    sectors = (uint32_t)strtoul(argv[3], NULL, 10);
    if (!strcmp(argv[1], "mkimage")) return mkimage(argv[2], sectors, (uint32_t)strtoul(argv[4], NULL, 10));
    if (!strcmp(argv[1], "fragment")) return fragment(argv[2], sectors, (uint32_t)strtoul(argv[4], NULL, 10));
    if (!strcmp(argv[1], "addfile") && argc >= 6) return addfile(argv[2], sectors, argv[4], argv[5]);
    if (!strcmp(argv[1], "mkdirs") && argc >= 6) return mkdirs(argv[2], sectors, argv[4], atoi(argv[5]));
    if (!strcmp(argv[1], "wr256")) return wr256(argv[2], sectors);
    if (!strcmp(argv[1], "dump")) return dump(argv[2], sectors, argv[4]);
    if (strcmp(argv[1], "run") || argc < 6) return 2;
    snprintf(sim_pc_root, sizeof sim_pc_root, "%s", argv[4]);
    for (i = 6; i < argc; i++) {
        if (!strcmp(argv[i], "--cut")) sim_cut_after = atoll(argv[++i]);
        else if (!strcmp(argv[i], "--ckp")) opt.ckp = atol(argv[++i]);
        else if (!strcmp(argv[i], "--read")) opt.read = atol(argv[++i]);
        else if (!strcmp(argv[i], "--readback")) opt.readback = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--slow")) opt.slow = 1;
        else if (!strcmp(argv[i], "--flip")) sim_flip_cmd = atoll(argv[++i]);
        else if (!strcmp(argv[i], "--caps")) sim_caps = (uint8_t)atoi(argv[++i]);
        else if (!strcmp(argv[i], "--frag")) opt.frag = 1;
        else if (!strcmp(argv[i], "--force")) opt.force = 1;
        else if (!strcmp(argv[i], "--stopafter")) sim_stop_after_polls = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--b-stuck")) sim_b_stuck = 1;
        else if (!strcmp(argv[i], "--pollms")) ng_pc_poll_ms = (uint32_t)atoi(argv[++i]);
        else if (!strcmp(argv[i], "--quiet")) sim_quiet = 1;
        else if (!strcmp(argv[i], "--rflip")) sim_rflip_cmd = atoll(argv[++i]);
        else if (!strcmp(argv[i], "--rflip-all")) sim_rflip_every = 1;
        else if (!strcmp(argv[i], "--rflipn")) sim_rflipn_cmd = atoll(argv[++i]);
        else if (!strcmp(argv[i], "--rflipn-all")) sim_rflipn_every = 1;
        else if (!strcmp(argv[i], "--rflipn-from")) sim_rflipn_from = atoll(argv[++i]);
        else if (!strcmp(argv[i], "--headroom")) opt.headroom = atol(argv[++i]);
        else if (!strcmp(argv[i], "--mountms")) opt.mountms = atol(argv[++i]);
        else if (!strcmp(argv[i], "--ua")) { sim_ua_mode = 1; sim_ua_pending = 1; }
        else if (!strcmp(argv[i], "--failend2")) sim_fail_end2 = 1;
    }
    if (sim_open(argv[2], sectors)) { printf("cannot open image\n"); return 2; }
    sim_protect_lba = 2048;
    ng_pc_cfg_hook = cfg_hook;
    ng_phase_hook = phase_hook;
    ng_pc_pad_healthy = !sim_b_stuck;   /* --stopafter is a press on a healthy pad */
    rc = ng_pc_run(argv[5], NULL);
    printf("SIM rc=%d writes=%llu cmds=%llu violations=%llu unsess=%d fnvbad=%u retries=%u unverified=%u begin=%d end=%d "
           "endmode=%d endname=%s dead=%d rdmis=%u ua=%d defmounts=%d namedmounts=%d polls=%d rcmds=%llu rsecs=%llu\n",
           rc, (unsigned long long)sim_sector_writes, (unsigned long long)sim_cmd_writes,
           (unsigned long long)sim_violations, sim_unsessioned_writes, (unsigned)ngblk_stats.fnv_mismatches,
           (unsigned)ngblk_stats.io_retries, (unsigned)ngblk_stats.unverified, sim_begin_calls, sim_end_calls,
           sim_end_mode, sim_end_name, sim_dead, (unsigned)ngblk_stats.read_mismatches, sim_ua_served,
           sim_default_mounts, sim_named_mounts, sim_polls, (unsigned long long)sim_cmd_reads,
           (unsigned long long)sim_sector_reads);
    sim_close();
    return rc;
}
