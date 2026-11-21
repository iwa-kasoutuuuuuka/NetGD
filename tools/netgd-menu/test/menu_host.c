/* Host test driver for netgd-menu's core.  Driven by test/run_tests.py.
     menu_host selftest                 pure functions (design 11.2 U1..U7; U8 is in the frame tests)
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

static int selftest(void)
{
    u1_slot();
    u2_ini();
    u3_titles();
    u4_ipbin();
    u5_text();
    u6_list();
    u7_input();
    printf(bad ? "SELFTEST FAIL (%d)\n" : "SELFTEST OK\n", bad);
    return bad ? 1 : 0;
}

int main(int argc, char **argv)
{
    if (argc >= 2 && !strcmp(argv[1], "selftest")) return selftest();
    fprintf(stderr, "usage: menu_host selftest\n");
    return 2;
}
