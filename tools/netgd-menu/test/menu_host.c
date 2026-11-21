/* Host test driver for netgd-menu's core.  Driven by test/run_tests.py.
     menu_host selftest                 pure functions (design 11.2 U1..U7; U8 is in the frame tests)
     menu_host ui <img> <sectors> --script "<events>" [--slots 2,3,5] [--ua] [--notready N] [--nofw] [--noimg]
                  [--rflip K] [--rflip-all] [--nocontroller] [--names A|B]
        events: U D L R A B X Y S LT RT (press and release), hold:A:1200, t:500, scan, step, dump, rflipall, ua,
                nocontroller, controller
   Nothing here runs on a Dreamcast. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "nm.h"
#include "nm_text.h"
#include "nm_slot.h"
#include "nm_cfgparse.h"
#include "nm_ipbin.h"
#include "nm_list.h"
#include "nm_input.h"
#include "ngtp.h"
#include "nm_ui.h"
#include "nm_disc.h"
#include "nm_art.h"
#include "crc32.h"
#include "nm_plat.h"
#include "plat_host.h"
#include "ngdev.h"
#include "simdisk.h"

static int bad;
#define CHECK(cond) do { if (!(cond)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #cond); bad++; } } while (0)
#define CHECK_STR(a, b) do { const char *a_ = (a), *b_ = (b); if (strcmp(a_, b_)) { printf("FAIL %s:%d: \"%s\" != \"%s\"\n", __FILE__, __LINE__, a_, b_); bad++; } } while (0)

/* U1: the cases of common/discimg/src/slot.rs */
static void u1_slot(void)
{
    CHECK(nm_slot_number("01") == 1);
    CHECK(nm_slot_number("002") == 2);
    CHECK(nm_slot_number("1000") == 1000);
    CHECK(nm_slot_number("99") == 99);
    CHECK(nm_slot_number("0") == 0);
    CHECK(nm_slot_number("00") == 0);
    CHECK(nm_slot_number("0000") == 0);
    CHECK(nm_slot_number("1") == 0);
    CHECK(nm_slot_number("12345") == 0);
    CHECK(nm_slot_number("0a") == 0);
    CHECK(nm_slot_number("02 X") == 0);
    CHECK(nm_slot_number("") == 0);
    CHECK(nm_slot_number("0 2") == 0);
    CHECK(nm_slot_number("01.") == 0);
}

/* U2: the inputs and expectations of common/gamecfg/src/ini.rs tests */
static void ini(nm_ini_t *i, const char *t) { nm_ini_parse(i, t, strlen(t)); }

static void u2_ini(void)
{
    nm_ini_t i;
    ini(&i, "");
    CHECK(i.open_time == 500 && i.detect_time == 250 && i.reset_goto == 0 && !i.open_time_set && !i.detect_time_set);
    CHECK(nm_ini_ready_limit_ms(&i) == 10000);
    ini(&i, "open_time=700\ndetect_time=300\nimage_tests=0\nhigh_speed=1\nreset_goto=12\nread_limit=900\n");
    CHECK(i.open_time == 700 && i.detect_time == 300 && i.reset_goto == 12 && i.open_time_set && i.detect_time_set);
    CHECK(nm_ini_ready_limit_ms(&i) == 11000);
    ini(&i, "open_time=10\ndetect_time=99999\n");
    CHECK(i.open_time == 150 && i.detect_time == 5000);
    ini(&i, "open_time=150\ndetect_time=5000\n");
    CHECK(i.open_time == 150 && i.detect_time == 5000);
    ini(&i, "open_time=-5\n");
    CHECK(i.open_time == 150 && i.open_time_set);
    CHECK(nm_ini_ready_limit_ms(&i) == 10150);
    ini(&i, "reset_goto=9999"); CHECK(i.reset_goto == 9999);
    ini(&i, "reset_goto=10000"); CHECK(i.reset_goto == 0);
    ini(&i, "reset_goto=-1"); CHECK(i.reset_goto == 0);
    ini(&i, "reset_goto=0"); CHECK(i.reset_goto == 0);
    ini(&i, "# a comment\n; another\n[GDEMU]\n  Open_Time = 600  # trailing\nRESET_GOTO\t=\t3 ; trailing\n");
    CHECK(i.open_time == 600 && i.reset_goto == 3);
    ini(&i, "open_time=800\r\ndetect_time=400\r\nreset_goto=2");
    CHECK(i.open_time == 800 && i.detect_time == 400 && i.reset_goto == 2);
    ini(&i, "open_time\n=5\nopen_time=abc\nreset_goto=\ndetect_time=1x\nhigh_speed=0x1\n");
    CHECK(i.open_time == 500 && i.detect_time == 250 && i.reset_goto == 0 && !i.open_time_set && !i.detect_time_set);
    ini(&i, "reset_goto=2\nreset_goto=\"5\"\n");
    CHECK(i.reset_goto == 5);
    ini(&i, "open_time=500"); CHECK(nm_ini_ready_limit_ms(&i) == 10500);
    ini(&i, "detect_time=250"); CHECK(nm_ini_ready_limit_ms(&i) == 10250);
    ini(&i, "\xEF\xBB\xBFopen_time=700\nreset_goto=3\n");
    CHECK(i.open_time == 700 && i.reset_goto == 3 && i.open_time_set);
    ini(&i, "# \x93\xfa\x96\x7b\x8c\xea\nreset_goto=12 # \x82\xa0\nopen_time=600\n");
    CHECK(i.reset_goto == 12 && i.open_time == 600);
    ini(&i, "open_time=\x82\xa0\nreset_goto=5\n");
    CHECK(i.reset_goto == 5 && !i.open_time_set);
    ini(&i, "open_time=800\rdetect_time=400\rreset_goto=2\r");
    CHECK(i.open_time == 800 && i.detect_time == 400 && i.reset_goto == 2);
    ini(&i, "open_time=800\r\n\r\ndetect_time=400\n\rreset_goto=2");
    CHECK(i.open_time == 800 && i.detect_time == 400 && i.reset_goto == 2);
    ini(&i, "reset_goto=+7"); CHECK(i.reset_goto == 7);
    ini(&i, "reset_goto=1234567890"); CHECK(i.reset_goto == 0);
}

/* U3: GAME.CFG title and name.txt */
static void u3_titles(void)
{
    char t[80];
    const char *s;
    s = "# a game\ntitle = \"Skies of Arcadia (Disc 1)\"\nnext = \"Skies of Arcadia Disc 2\"\n";
    CHECK(nm_gamecfg_title(s, strlen(s), t, sizeof t) == 1); CHECK_STR(t, "Skies of Arcadia (Disc 1)");
    s = "[meta]\nnonsense\ntitle = \"ok\"\nnext = \n= 3\n";
    CHECK(nm_gamecfg_title(s, strlen(s), t, sizeof t) == 1); CHECK_STR(t, "ok");
    s = "title = \"Hash # Game\" # real comment\n";
    CHECK(nm_gamecfg_title(s, strlen(s), t, sizeof t) == 1); CHECK_STR(t, "Hash # Game");
    s = "title = Bare Words\n";
    CHECK(nm_gamecfg_title(s, strlen(s), t, sizeof t) == 1); CHECK_STR(t, "Bare Words");
    s = "title = \"unterminated\n";
    CHECK(nm_gamecfg_title(s, strlen(s), t, sizeof t) == 0);
    s = "Title = \"wrong case\"\n";
    CHECK(nm_gamecfg_title(s, strlen(s), t, sizeof t) == 0);
    s = "[disc]\ntitle=\"Section Is Skipped Only As A Line\"\n";
    CHECK(nm_gamecfg_title(s, strlen(s), t, sizeof t) == 1);
    s = "\xEF\xBB\xBFtitle = \"With BOM\"\r\nnext = \"x\"\r\n";
    CHECK(nm_gamecfg_title(s, strlen(s), t, sizeof t) == 1); CHECK_STR(t, "With BOM");
    s = "title = \"CRLF Title\"\r\n";
    CHECK(nm_gamecfg_title(s, strlen(s), t, sizeof t) == 1); CHECK_STR(t, "CRLF Title");
    s = "title = \"0123456789012345678901234567890123456789012345678901234567890123456789\"\n";
    CHECK(nm_gamecfg_title(s, strlen(s), t, sizeof t) == 1); CHECK(strlen(t) == 64);
    CHECK(nm_gamecfg_title("", 0, t, sizeof t) == 0);
    CHECK(nm_gamecfg_title("title = \"\"\n", 11, t, sizeof t) == 0);
    s = "title = \"Caf\xC3\xA9 \xE3\x82\xB2\xE3\x83\xBC\xE3\x83\xA0\"\n";
    CHECK(nm_gamecfg_title(s, strlen(s), t, sizeof t) == 1); CHECK_STR(t, "Caf? ???");
    /* name.txt */
    s = "Crazy Taxi\r\nsecond line\r\n";
    CHECK(nm_nametxt_title(s, strlen(s), t, sizeof t) == 1); CHECK_STR(t, "Crazy Taxi");
    s = "\xEF\xBB\xBF  Spaced   Name  \n";
    CHECK(nm_nametxt_title(s, strlen(s), t, sizeof t) == 1); CHECK_STR(t, "Spaced Name");
    CHECK(nm_nametxt_title("", 0, t, sizeof t) == 0);
    CHECK(nm_nametxt_title("\n\nlater\n", 8, t, sizeof t) == 0);      /* only the first line counts */
}

/* U4: .gdi parsing and IP.BIN titles */
static void u4_ipbin(void)
{
    char name[64];
    unsigned ss;
    const char *g;
    uint8_t sec[2352];

    g = "3\r\n1 0 4 2352 track01.bin 0\r\n2 450 0 2352 track02.raw 0\r\n3 45000 4 2048 track03.bin 0\r\n";
    CHECK(nm_gdi_track3(g, strlen(g), name, sizeof name, &ss) == 1); CHECK_STR(name, "track03.bin"); CHECK(ss == 2048);
    g = "\xEF\xBB\xBF" "3\n1 0 4 2352 \"track 01.bin\" 0\n2 450 0 2352 track02.raw 0\n3 45000 4 2352 \"track 03.bin\" 0\n";
    CHECK(nm_gdi_track3(g, strlen(g), name, sizeof name, &ss) == 1); CHECK_STR(name, "track 03.bin"); CHECK(ss == 2352);
    g = "2\n1 0 4 2352 track01.bin 0\n2 450 0 2352 track02.raw 0\n";
    CHECK(nm_gdi_track3(g, strlen(g), name, sizeof name, &ss) == 0);
    g = "4\n1 0 4 2352 a.bin 0\n2 450 0 2352 b.raw 0\n4 45000 4 2048 d.bin 0\n";    /* no track 3: the LBA 45000 line */
    CHECK(nm_gdi_track3(g, strlen(g), name, sizeof name, &ss) == 1); CHECK_STR(name, "d.bin"); CHECK(ss == 2048);
    g = "3\n1 0 4 2352 a.bin 0\ngarbage line\n3 45000 4 1234 c.bin 0\n";           /* bad sector size */
    CHECK(nm_gdi_track3(g, strlen(g), name, sizeof name, &ss) == 0);
    g = "3\n3 45000 4 2048\n";                                                     /* too few fields */
    CHECK(nm_gdi_track3(g, strlen(g), name, sizeof name, &ss) == 0);
    CHECK(nm_gdi_track3("", 0, name, sizeof name, &ss) == 0);

    memset(sec, 0, sizeof sec);
    memcpy(sec, "SEGA SEGAKATANA ", 16);
    memset(sec + 0x80, ' ', 128);
    memcpy(sec + 0x80, "  CRAZY   TAXI", 14);
    CHECK(nm_ipbin_title(sec, 2048, 2048, name, sizeof name) == 1); CHECK_STR(name, "CRAZY TAXI");
    memset(sec, 0, sizeof sec);
    memcpy(sec + 16, "SEGA SEGAKATANA ", 16);
    memset(sec + 16 + 0x80, ' ', 128);
    memcpy(sec + 16 + 0x80, "SONIC ADVENTURE", 15);
    CHECK(nm_ipbin_title(sec, 2352, 2352, name, sizeof name) == 1); CHECK_STR(name, "SONIC ADVENTURE");
    CHECK(nm_ipbin_title(sec, 2352, 2048, name, sizeof name) == 0);                 /* read as 2048: no magic at 0 */
    memcpy(sec + 16, "SEGA SEGAKATANX ", 16);
    CHECK(nm_ipbin_title(sec, 2352, 2352, name, sizeof name) == 0);                 /* wrong magic */
    memcpy(sec + 16, "SEGA SEGAKATANA ", 16);
    memset(sec + 16 + 0x80, ' ', 128);
    CHECK(nm_ipbin_title(sec, 2352, 2352, name, sizeof name) == 0);                 /* blank title */
    CHECK(nm_ipbin_title(sec, 100, 2352, name, sizeof name) == 0);                  /* short read */
    memset(sec + 16 + 0x80, 0xA4, 128);
    CHECK(nm_ipbin_title(sec, 2352, 2352, name, sizeof name) == 1);                 /* non-ASCII -> '?' */
    CHECK(name[0] == '?');
}

/* U5 */
static void u5_text(void)
{
    char o[100];
    nm_text_clean(o, sizeof o, "  a \t b\r\nc  ", 12, 64); CHECK_STR(o, "a b c");
    nm_text_clean(o, sizeof o, "ab\xC3\xA9" "cd", 7, 64); CHECK_STR(o, "ab?cd");
    nm_text_clean(o, sizeof o, "\x82\xa0\x82\xa2", 4, 64); CHECK_STR(o, "????");   /* Shift-JIS: no UTF-8 lead byte, one '?' per byte */
    nm_text_clean(o, sizeof o, "abcdefghij", 10, 5); CHECK_STR(o, "abcde");
    nm_text_clean(o, sizeof o, "abcd efgh", 9, 5); CHECK_STR(o, "abcd");           /* no trailing space */
    nm_text_clean(o, sizeof o, "ab\0cd", 5, 64); CHECK_STR(o, "ab");                  /* stops at NUL */
    nm_text_clean(o, 4, "abcdefgh", 8, 64); CHECK_STR(o, "abc");                       /* dst size wins */
    CHECK(nm_text_clean(o, sizeof o, "   ", 3, 64) == 0);
    nm_text_fit(o, sizeof o, "short", 10); CHECK_STR(o, "short");
    nm_text_fit(o, sizeof o, "exactly10!", 10); CHECK_STR(o, "exactly10!");
    nm_text_fit(o, sizeof o, "this is too long", 10); CHECK_STR(o, "this is ..");
    CHECK(nm_text_fit(o, sizeof o, "this is too long", 10) == 10);
    nm_text_fit(o, sizeof o, "abc", 1); CHECK_STR(o, ".");
    nm_text_fit(o, sizeof o, "abcdef", 1); CHECK_STR(o, ".");
}

/* U6 */
static void u6_list(void)
{
    static nm_list_t l;
    nm_list_init(&l);
    nm_list_add(&l, "Zelda", 0);
    nm_list_add(&l, "1000", 1000);
    nm_list_add(&l, "03", 3);
    nm_list_add(&l, "alpha", 0);
    nm_list_add(&l, "002", 2);
    nm_list_add(&l, "02", 2);        /* duplicate number: "02" has fewer digits and stays */
    nm_list_add(&l, "10", 10);
    nm_list_sort(&l);
    CHECK(l.count == 6);
    CHECK_STR(l.items[0].folder, "02"); CHECK(l.items[0].flags & NM_F_DUPLICATE);
    CHECK_STR(l.items[1].folder, "03");
    CHECK_STR(l.items[2].folder, "10");
    CHECK_STR(l.items[3].folder, "1000");
    CHECK_STR(l.items[4].folder, "alpha");           /* named folders after, case-insensitive */
    CHECK_STR(l.items[5].folder, "Zelda");
    CHECK(l.duplicate == 2);
    CHECK_STR(l.items[0].title, "02");                /* the title starts as the folder name */
    CHECK(nm_list_find_folder(&l, "1000") == 3 && nm_list_find_folder(&l, "002") == -1);
    CHECK(nm_list_find_number(&l, 10) == 2 && nm_list_find_number(&l, 11) == -1);
    /* the limit */
    nm_list_init(&l);
    for (int i = 0; i < NM_MAX_GAMES + 5; i++) {
        char n[16];
        snprintf(n, sizeof n, "G%d", i);
        nm_list_add(&l, n, 0);
    }
    CHECK(l.count == NM_MAX_GAMES && l.truncated);
    CHECK(nm_list_add(&l, "", 0) == -1);
    {
        char longname[200];
        memset(longname, 'x', sizeof longname - 1);
        longname[sizeof longname - 1] = 0;
        nm_list_init(&l);
        CHECK(nm_list_add(&l, longname, 0) == -1 && l.count == 0);
    }
}

/* U7 */
static void u7_input(void)
{
    nm_input_t in;
    uint32_t t, ev, ups = 0, rights = 0;
    nm_input_init(&in);
    CHECK(nm_input_update(&in, 0, 0) == 0);
    CHECK(nm_input_update(&in, NM_BTN_A, 10) == NM_BTN_A);                          /* the rising edge */
    CHECK(nm_input_update(&in, NM_BTN_A, 20) == 0);                                 /* held: nothing more */
    CHECK(nm_input_update(&in, 0, 30) == 0);
    CHECK(nm_input_update(&in, NM_BTN_A, 40) == NM_BTN_A);                          /* again after a release */
    /* two buttons at once */
    nm_input_init(&in);
    CHECK(nm_input_update(&in, NM_BTN_A | NM_BTN_B, 0) == (NM_BTN_A | NM_BTN_B));
    CHECK(nm_input_update(&in, NM_BTN_A | NM_BTN_B | NM_BTN_X, 10) == NM_BTN_X);
    /* UP held: edge at 1000, then 1300, 1380, 1460 ... (10 ms frames) */
    nm_input_init(&in);
    for (t = 1000; t <= 1500; t += 10) {
        ev = nm_input_update(&in, NM_BTN_UP | NM_BTN_RIGHT, t);
        if (ev & NM_BTN_UP) {
            ups++;
            CHECK(t == 1000 || t == 1300 || t == 1380 || t == 1460);
        }
        if (ev & NM_BTN_RIGHT) rights++;
    }
    CHECK(ups == 4);
    CHECK(rights == 1);                                                              /* RIGHT does not repeat */
    CHECK(nm_input_held_ms(&in, NM_BTN_UP, 1500) == 500);
    nm_input_update(&in, 0, 1510);
    CHECK(nm_input_held_ms(&in, NM_BTN_UP, 1510) == 0);
    /* a slow frame rate does not fire a burst */
    nm_input_init(&in);
    nm_input_update(&in, NM_BTN_DOWN, 0);
    CHECK(nm_input_update(&in, NM_BTN_DOWN, 1000) == NM_BTN_DOWN);
    CHECK(nm_input_update(&in, NM_BTN_DOWN, 1010) == 0);
    /* a clock that wraps */
    nm_input_init(&in);
    nm_input_update(&in, NM_BTN_UP, 0xFFFFFF00u);
    CHECK(nm_input_update(&in, NM_BTN_UP, 0x100u) == NM_BTN_UP);                    /* 0x200 ms later */
}

/* ---- menu_host ui: the UI core on a virtual clock, driven by a script ---- */

static void sim_summary(void)
{
    printf("SIM cmd_writes=%llu sector_writes=%llu begin=%d end=%d unsess=%d violations=%llu rcmds=%llu rsecs=%llu "
           "gdemu_mounts=%d named_mounts=%d mount_in_session=%d packets=%d endmode=%d endname=%s session=%d\n",
           (unsigned long long)sim_cmd_writes, (unsigned long long)sim_sector_writes, sim_begin_calls, sim_end_calls,
           sim_unsessioned_writes, (unsigned long long)sim_violations, (unsigned long long)sim_cmd_reads,
           (unsigned long long)sim_sector_reads, sim_gdemu_mounts, sim_named_mounts, sim_mount_in_session, sim_packets, sim_end_mode,
           sim_end_name[0] ? sim_end_name : "-", sim_session);
}

static nm_ui_t ui;

#define FRAME_MS 16

static void frame(void)
{
    nm_ui_step(&ui);
    nm_plat_sleep(FRAME_MS);
}

static void run_ms(uint32_t ms)
{
    for (uint32_t t = 0; t < ms; t += FRAME_MS) frame();
}

static void dump_screen(void)
{
    char row[NM_COLS + 1];
    printf("SCREEN state=%d cursor=%d top=%d count=%d scan=%d\n", ui.state, ui.cursor, ui.top, ui.list.count, ui.scan.state);
    for (int r = 0; r < NM_ROWS; r++) {
        nm_screen_row_text(&nm_host_screen, r, row);
        printf("S%02d|%s\n", r, row);
    }
    printf("END\n");
    {
        uint8_t le[NM_ART_BYTES];
        uint32_t crc = 0;
        if (nm_host_art) {
            for (int i = 0; i < NM_ART_PW * NM_ART_PH; i++) { le[i * 2] = (uint8_t)nm_host_art[i]; le[i * 2 + 1] = (uint8_t)(nm_host_art[i] >> 8); }
            crc = ng_crc32(0, le, sizeof le);
        }
        printf("ART shown=%d crc=%08X dirty=%d layout=%d state=%d\n", nm_host_art ? 1 : 0, (unsigned)crc, nm_host_art_dirty, ui.art_layout, ui.art.state);
    }
}

static uint32_t button_by_name(const char *n)
{
    static const struct { const char *name; uint32_t bit; } tab[] = {
        { "U", NM_BTN_UP }, { "D", NM_BTN_DOWN }, { "L", NM_BTN_LEFT }, { "R", NM_BTN_RIGHT }, { "A", NM_BTN_A },
        { "B", NM_BTN_B }, { "X", NM_BTN_X }, { "Y", NM_BTN_Y }, { "S", NM_BTN_START }, { "LT", NM_BTN_LT }, { "RT", NM_BTN_RT },
    };
    for (unsigned i = 0; i < sizeof tab / sizeof tab[0]; i++) if (!strcmp(tab[i].name, n)) return tab[i].bit;
    return 0;
}

static void run_event(char *ev)
{
    uint32_t bit;
    if (!strcmp(ev, "dump")) { dump_screen(); return; }
    if (!strcmp(ev, "step")) { frame(); return; }
    if (!strcmp(ev, "scan")) {
        for (int i = 0; i < 200000 && (ui.state == NM_S_BOOT || ui.scan.state == NM_SCAN_RUNNING || ui.scan.state == NM_SCAN_IDLE); i++) {
            frame();
            if (ui.state == NM_S_NOFW || ui.state == NM_S_NOSD) break;
        }
        return;
    }
    if (!strcmp(ev, "rflipall")) { sim_rflip_every = 1; return; }
    if (!strcmp(ev, "rflipboth")) { sim_rflip_every = 1; sim_rflipn_every = 1; return; }
    if (!strcmp(ev, "rflipoff")) { sim_rflip_every = 0; sim_rflipn_every = 0; return; }
    if (!strcmp(ev, "ua")) { sim_ua_mode = 1; sim_ua_pending = 1; return; }
    if (!strcmp(ev, "nocontroller")) { nm_host_controller = 0; return; }
    if (!strcmp(ev, "controller")) { nm_host_controller = 1; return; }
    if (!strncmp(ev, "t:", 2)) { run_ms((uint32_t)atol(ev + 2)); return; }
    if (!strncmp(ev, "hold:", 5)) {
        char *colon = strchr(ev + 5, ':');
        if (!colon) return;
        *colon = 0;
        bit = button_by_name(ev + 5);
        nm_host_buttons = bit;
        run_ms((uint32_t)atol(colon + 1));
        nm_host_buttons = 0;
        frame();
        return;
    }
    bit = button_by_name(ev);
    if (!bit) { printf("unknown event '%s'\n", ev); exit(2); }
    nm_host_buttons = bit;
    frame();
    nm_host_buttons = 0;
    frame();
}

/* menu_host ui <img> <sectors> --script "..." [--slots 2,3,5] [--ua] [--notready N] [--nofw] [--noimg] [--rflip K] [--rflip-all] */
static int ui_cmd(int argc, char **argv)
{
    const char *script = "";
    const char *img = argv[2];
    uint32_t sectors = (uint32_t)strtoul(argv[3], NULL, 10);
    int noimg = 0, k = 0;
    char *copy, *tok;
    for (int i = 4; i < argc; i++) {
        if (!strcmp(argv[i], "--script") && i + 1 < argc) script = argv[++i];
        else if (!strcmp(argv[i], "--slots") && i + 1 < argc) {
            char *s = argv[++i], *p;
            for (p = strtok(s, ","); p && k < 63; p = strtok(NULL, ",")) sim_slots[k++] = atoi(p);
        }
        else if (!strcmp(argv[i], "--ua")) { sim_ua_mode = 1; sim_ua_pending = 1; }
        else if (!strcmp(argv[i], "--notready") && i + 1 < argc) sim_notready_polls = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--nofw")) sim_fw_dead = 1;
        else if (!strcmp(argv[i], "--noimg")) noimg = 1;
        else if (!strcmp(argv[i], "--rflip") && i + 1 < argc) sim_rflip_cmd = atoll(argv[++i]);
        else if (!strcmp(argv[i], "--rflip-all")) sim_rflip_every = 1;
        else if (!strcmp(argv[i], "--nocontroller")) nm_host_controller = 0;
        else if (!strcmp(argv[i], "--nobba")) nm_host_net_nodev = 1;
        else if (!strcmp(argv[i], "--noaddr") && i + 1 < argc) nm_host_net_noaddr = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--names") && i + 1 < argc) snprintf(sim_names, sizeof sim_names, "%s", argv[++i]);
    }
    if (!noimg && sim_open(img, sectors)) { printf("cannot open image\n"); return 2; }
    sim_trace = 1;
    atexit(sim_summary);
    nm_ui_init(&ui);
    copy = malloc(strlen(script) + 1);
    strcpy(copy, script);
    for (tok = strtok(copy, " ,"); tok; tok = strtok(NULL, " ,")) {
        char ev[64];
        snprintf(ev, sizeof ev, "%s", tok);
        run_event(ev);
    }
    free(copy);
    return 0;
}

/* U8: the NGTP frame layer */
static void u8_frame(void)
{
    static uint8_t f[NGTP_MAX_FRAME + 16], big[NGTP_MAX_PAYLOAD + 1];
    uint8_t pay[40];
    ngtp_hdr_t h;
    size_t n;
    for (int i = 0; i < 40; i++) pay[i] = (uint8_t)(i * 5 + 1);
    /* a round trip */
    n = ngtp_frame_build(f, sizeof f, NGTP_LINE, NGTP_F_THROTTLED, 0x01020304u, pay, 40);
    CHECK(n == 16 + 40 + 4);
    CHECK(!memcmp(f, "NGT1", 4) && f[4] == NGTP_LINE && f[5] == 1 && f[6] == 0 && f[7] == 0);
    CHECK(f[8] == 4 && f[9] == 3 && f[10] == 2 && f[11] == 1);                       /* little endian sequence */
    CHECK(f[12] == 40 && f[13] == 0);
    CHECK(ngtp_hdr_parse(f, &h) == NGTP_OK && h.type == NGTP_LINE && h.flags == 1 && h.seq == 0x01020304u && h.len == 40);
    CHECK(ngtp_frame_check(f, f + 16, 40, f + 16 + 40) == NGTP_OK);
    /* the CRC is the zlib CRC-32 of header + payload: pin it with a known vector */
    {
        uint8_t e[20];
        size_t en = ngtp_frame_build(e, sizeof e, NGTP_PING, 0, 0, NULL, 0);
        CHECK(en == 20);
        CHECK(ngtp_frame_crc(e, NULL, 0) == ngtp_get32(e + 16));
    }
    /* a flipped bit anywhere in header or payload */
    for (int pos = 0; pos < 16 + 40; pos++) {
        uint8_t c[80];
        ngtp_hdr_t hh;
        memcpy(c, f, n);
        c[pos] ^= 0x04;
        if (ngtp_hdr_parse(c, &hh) == NGTP_OK && hh.len == 40) CHECK(ngtp_frame_check(c, c + 16, 40, c + 16 + 40) == NGTP_E_CRC);
    }
    {
        uint8_t c[80];
        memcpy(c, f, n);
        c[n - 1] ^= 0x80;                                                              /* in the CRC itself */
        CHECK(ngtp_frame_check(c, c + 16, 40, c + 16 + 40) == NGTP_E_CRC);
    }
    /* bad magic, bad zero field, too long */
    {
        uint8_t c[16];
        memcpy(c, f, 16);
        c[0] = 'X';
        CHECK(ngtp_hdr_parse(c, &h) == NGTP_E_MAGIC);
        memcpy(c, f, 16);
        c[6] = 1;
        CHECK(ngtp_hdr_parse(c, &h) == NGTP_E_MAGIC);
        memcpy(c, f, 16);
        ngtp_put32(c + 12, NGTP_MAX_PAYLOAD);
        CHECK(ngtp_hdr_parse(c, &h) == NGTP_OK && h.len == NGTP_MAX_PAYLOAD);
        ngtp_put32(c + 12, NGTP_MAX_PAYLOAD + 1);
        CHECK(ngtp_hdr_parse(c, &h) == NGTP_E_LENGTH);
        ngtp_put32(c + 12, 0xFFFFFFFFu);
        CHECK(ngtp_hdr_parse(c, &h) == NGTP_E_LENGTH);
    }
    /* sizes: the largest frame, and refusals */
    memset(big, 0x5A, sizeof big);
    CHECK(ngtp_frame_build(f, sizeof f, NGTP_DATA, 0, 1, big, NGTP_MAX_PAYLOAD) == NGTP_MAX_FRAME);
    CHECK(ngtp_frame_build(f, sizeof f, NGTP_DATA, 0, 1, big, NGTP_MAX_PAYLOAD + 1) == 0);
    CHECK(ngtp_frame_build(f, 19, NGTP_PING, 0, 1, NULL, 0) == 0);                    /* no room for the CRC */
    CHECK(ngtp_frame_build(f, 20, NGTP_PING, 0, 1, NULL, 0) == 20);
    /* building in place (the payload already at out+16) is allowed */
    memcpy(f + 16, pay, 40);
    CHECK(ngtp_frame_build(f, sizeof f, NGTP_LINE, 0, 7, f + 16, 40) == 60);
    CHECK(ngtp_frame_check(f, f + 16, 40, f + 56) == NGTP_OK && !memcmp(f + 16, pay, 40));
    /* the request types */
    for (int t = 0; t < 256; t++) CHECK(ngtp_is_request((uint8_t)t) == (t >= 1 && t <= 9));
    /* little-endian helpers */
    {
        uint8_t b[8];
        ngtp_put64(b, 0x0102030405060708ull);
        CHECK(b[0] == 8 && b[7] == 1 && ngtp_get64(b) == 0x0102030405060708ull);
    }
    /* the pass code is always four digits, and a seed maps the same way every time */
    for (uint32_t s = 0; s < 1000; s++) CHECK(nm_ui_code_from_seed(s) < 10000 && nm_ui_code_from_seed(s) == nm_ui_code_from_seed(s));
}

/* U10: the header of a box art file (step 11) */
static void u10_art(void)
{
    uint8_t h[16] = { 'N', 'G', 'A', 'R', 1, 0, 128, 0, 128, 0, 0, 0, 0, 0, 0, 0 };
    CHECK(nm_art_check_header(h) == 0);
    h[10] = 0xAA; h[15] = 0x55;
    CHECK(nm_art_check_header(h) == 0);                 /* the reserved bytes are not looked at */
    h[0] = 'X'; CHECK(nm_art_check_header(h) != 0); h[0] = 'N';
    h[3] = 'r'; CHECK(nm_art_check_header(h) != 0); h[3] = 'R';
    h[4] = 0; CHECK(nm_art_check_header(h) != 0);       /* version 0 */
    h[4] = 2; CHECK(nm_art_check_header(h) != 0);       /* version 2: a newer file than this menu knows */
    h[4] = 1; h[5] = 1; CHECK(nm_art_check_header(h) != 0); h[5] = 0;
    h[6] = 64; CHECK(nm_art_check_header(h) != 0); h[6] = 128;
    h[7] = 1; CHECK(nm_art_check_header(h) != 0); h[7] = 0;
    h[8] = 127; CHECK(nm_art_check_header(h) != 0); h[8] = 128;
    h[9] = 1; CHECK(nm_art_check_header(h) != 0); h[9] = 0;
    CHECK(nm_art_check_header(h) == 0);
    CHECK(NM_ART_COL + NM_ART_COLS == NM_COLS && NM_ART_COLS * 12 >= NM_ART_PW && NM_ART_ROWS * 24 >= NM_ART_PH);
}

/* U9: the discovery request and reply (step 9) */
static void u9_disc(void)
{
    char out[NM_DISC_MAX];
    size_t l;
    static const char ok[] = "NGTP-DISCOVER 1";
    CHECK(nm_disc_is_request(ok, sizeof ok - 1));
    CHECK(nm_disc_is_request("NGTP-DISCOVER 1\n", 16));
    CHECK(nm_disc_is_request("NGTP-DISCOVER 1 from=pc\r\n", 26));
    CHECK(!nm_disc_is_request("NGTP-DISCOVER 2", 15));
    CHECK(!nm_disc_is_request("NGTP-DISCOVER 10", 16));
    CHECK(!nm_disc_is_request("NGTP-DISCOVER", 13));
    CHECK(!nm_disc_is_request("NGTP-DISCOVEX 1", 15));
    CHECK(!nm_disc_is_request("ngtp-discover 1", 15));
    CHECK(!nm_disc_is_request("", 0));
    CHECK(!nm_disc_is_request("NGTP-HERE 1 name=NetGD", 22));            /* another Dreamcast's reply is not a request */
    l = nm_disc_reply(ok, sizeof ok - 1, 0, out, sizeof out);
    CHECK(l && l == strlen(out));
    CHECK_STR(out, "NGTP-HERE 1 name=NetGD app=" NM_VERSION " state=idle");
    l = nm_disc_reply(ok, sizeof ok - 1, 1, out, sizeof out);
    CHECK_STR(out, "NGTP-HERE 1 name=NetGD app=" NM_VERSION " state=busy");
    CHECK(nm_disc_reply("junk", 4, 0, out, sizeof out) == 0);
    CHECK(nm_disc_reply(ok, sizeof ok - 1, 0, out, 10) == 0);               /* too small: nothing half-written is sent */
}

/* ---- menu_host serve: the transfer screen on a real 127.0.0.1 socket, real time ---- */

static int serve_confirm;       /* 1 = A is held while the confirm screen shows, 2 = B */
static uint32_t serve_btn;

static uint32_t serve_buttons(void)
{
    if (ui.state == NM_S_NET_CONFIRM) return serve_confirm == 1 ? NM_BTN_A : serve_confirm == 2 ? NM_BTN_B : 0;
    return serve_btn;
}

/* menu_host serve <img> <sectors> --port P --code C [--cut N] [--idle-ms N] [--stopafter N] [--confirm allow|deny]
                   [--slots ..] [--ua] [--failend2] [--caps C] [--nobba] [--no-udp] [--timeout-ms T] [--dump] [--trace] */
static int serve_cmd(int argc, char **argv)
{
    const char *img = argv[2];
    uint32_t sectors = (uint32_t)strtoul(argv[3], NULL, 10), seed = 0, timeout_ms = 120000;
    unsigned code = 4821;
    int dump = 0, entered = 0, k = 0;
    uint64_t t0;
    for (int i = 4; i < argc; i++) {
        if (!strcmp(argv[i], "--port") && i + 1 < argc) nm_host_net_port = (uint16_t)atoi(argv[++i]);
        else if (!strcmp(argv[i], "--code") && i + 1 < argc) code = (unsigned)atoi(argv[++i]);
        else if (!strcmp(argv[i], "--cut") && i + 1 < argc) sim_cut_after = atoll(argv[++i]);
        else if (!strcmp(argv[i], "--idle-ms") && i + 1 < argc) ui.idle_ms = (uint32_t)atol(argv[++i]);
        else if (!strcmp(argv[i], "--stopafter") && i + 1 < argc) nm_host_stop_after_polls = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--confirm") && i + 1 < argc) serve_confirm = !strcmp(argv[++i], "allow") ? 1 : 2;
        else if (!strcmp(argv[i], "--ua")) { sim_ua_mode = 1; sim_ua_pending = 1; }
        else if (!strcmp(argv[i], "--failend2")) sim_fail_end2 = 1;
        else if (!strcmp(argv[i], "--caps") && i + 1 < argc) sim_caps = (uint8_t)atoi(argv[++i]);
        else if (!strcmp(argv[i], "--nobba")) nm_host_net_nodev = 1;
        else if (!strcmp(argv[i], "--no-udp")) nm_host_no_udp = 1;
        else if (!strcmp(argv[i], "--timeout-ms") && i + 1 < argc) timeout_ms = (uint32_t)atol(argv[++i]);
        else if (!strcmp(argv[i], "--dump")) dump = 1;
        else if (!strcmp(argv[i], "--trace")) sim_trace = 1;
        else if (!strcmp(argv[i], "--slots") && i + 1 < argc) {
            char *p;
            for (p = strtok(argv[++i], ","); p && k < 63; p = strtok(NULL, ",")) sim_slots[k++] = atoi(p);
        }
    }
    if (sim_open(img, sectors)) { printf("cannot open image\n"); return 2; }
    sim_protect_lba = 2048;
    nm_host_realtime = 1;
    nm_host_buttons_hook = serve_buttons;
    while (nm_ui_code_from_seed(seed) != (uint16_t)code) seed++;
    nm_host_entropy = seed;
    atexit(sim_summary);
    {
        uint32_t keep_idle = ui.idle_ms;
        nm_ui_init(&ui);
        if (keep_idle) ui.idle_ms = keep_idle;
    }
    for (int i = 0; i < 2000 && ui.state != NM_S_LIST; i++) {
        frame();
        if (ui.state == NM_S_NOFW || ui.state == NM_S_NOSD) { printf("SERVE-END state=%d\n", ui.state); return 0; }
    }
    serve_btn = NM_BTN_Y;
    frame();
    serve_btn = 0;
    frame();
    if (ui.state != NM_S_NET_IDLE) { printf("SERVE-END state=%d (the transfer screen did not open)\n", ui.state); if (dump) dump_screen(); return 0; }
    printf("LISTENING %u code %04u\n", (unsigned)nm_host_net_port, code);
    fflush(stdout);
    t0 = nm_host_real_ms();
    for (;;) {
        frame();
        if (ui.state == NM_S_NET_IDLE || ui.state == NM_S_NET_BUSY || ui.state == NM_S_NET_CONFIRM) entered = 1;
        else if (entered) break;
        if (sim_dead) { printf("POWERCUT\n"); fflush(stdout); return 0; }
        if (nm_host_real_ms() - t0 > timeout_ms) { printf("SERVE-TIMEOUT\n"); break; }
    }
    for (int i = 0; i < 100000 && (ui.state == NM_S_LIST) && ui.scan.state == NM_SCAN_RUNNING; i++) frame();
    printf("SERVE-END state=%d\n", ui.state);
    if (dump) dump_screen();
    return 0;
}

static int selftest(void)
{
    u1_slot();
    u2_ini();
    u3_titles();
    u4_ipbin();
    u5_text();
    u6_list();
    u7_input();
    u8_frame();
    u9_disc();
    u10_art();
    printf(bad ? "SELFTEST FAIL (%d)\n" : "SELFTEST OK\n", bad);
    return bad ? 1 : 0;
}

int main(int argc, char **argv)
{
    if (argc >= 2 && !strcmp(argv[1], "selftest")) return selftest();
    if (argc >= 4 && !strcmp(argv[1], "ui")) return ui_cmd(argc, argv);
    if (argc >= 4 && !strcmp(argv[1], "serve")) return serve_cmd(argc, argv);
    fprintf(stderr, "usage: menu_host selftest | ui <img> <sectors> --script \"<events>\" [options]\n");
    return 2;
}
