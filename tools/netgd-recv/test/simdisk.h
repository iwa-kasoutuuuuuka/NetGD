/* Host test doubles: an image file as the "secondary IDE disk" with power-cut and
   bit-flip injection, plus a model of the firmware's write session. */
#ifndef NG_SIMDISK_H
#define NG_SIMDISK_H
#include <stdint.h>

/* disk (dev_file.c) */
int sim_open(const char *path, uint32_t sectors);      /* an existing image; -1 if it cannot be opened */
int sim_open_ex(const char *path, uint32_t sectors, int create);    /* create != 0: a new, empty one */
void sim_close(void);
extern int64_t sim_cut_after;       /* sector writes allowed before the power goes; <0 = never */
extern int sim_dead;
extern uint64_t sim_sector_writes;  /* sector writes performed */
extern uint64_t sim_cmd_writes;     /* write commands */
extern int64_t sim_flip_cmd;        /* corrupt one bit in this (1-based) write command; <0 = never */
extern uint32_t sim_part_off;       /* added to every LBA (used only while creating the file system) */
extern uint32_t sim_protect_lba;    /* firmware F7: writes below this are refused during the session */
extern int64_t sim_rflip_cmd;        /* flip one bit in this (1-based) SINGLE-sector read; <0 = never */
extern int sim_rflip_every;         /* every single-sector read returns a different flipped bit (never stable) */
extern int64_t sim_rflipn_cmd;      /* the same for the (1-based) MULTI-sector read command */
extern int sim_rflipn_every;        /* every multi-sector read returns a different flipped bit */
extern int64_t sim_rflipn_from;     /* the same, from this (1-based) multi-sector read on; <0 = never */
extern uint64_t sim_cmd_reads, sim_sector_reads;   /* read commands / sectors read, of any size */
extern uint64_t sim_violations;     /* refused writes */
extern uint32_t sim_last_lba, sim_last_n, sim_last_fnv;
extern int sim_last_ok;
extern int sim_session;             /* write session active (fwctl_sim) */
extern int sim_unsessioned_writes;  /* writes while no session was active */

/* firmware (fwctl_sim.c) */
extern uint8_t sim_caps;            /* VERSION feature byte; default 7 */
extern int sim_end_calls, sim_end_mode;
extern char sim_end_name[128];
extern int sim_begin_calls;
extern int sim_ua_mode;             /* UNIT ATTENTION at start and after every END / MOUNT */
extern int sim_ua_pending, sim_ua_served;
extern int sim_fail_end;            /* END fails: 1 = the session ends anyway, 2 = it stays open */
extern int sim_fail_end2;           /* END mode 2 fails (the session ends anyway) */
extern int sim_default_mounts, sim_named_mounts, sim_packets;
/* menu commands (0x52 / TEST UNIT READY / named MOUNT) */
extern int sim_slots[64];           /* mountable folder numbers, 0-terminated; a 52 82 for another number: ILLEGAL REQUEST */
extern int sim_mounted_number;      /* last number mounted by 52 82 / 52 81 (0 = default / none) */
extern int sim_notready_polls;      /* TEST UNIT READY answers NOT READY this many times after a mount */
extern int sim_gdemu_mounts;        /* successful 52 82 / 52 81 */
extern char sim_names[256];         /* '|'-separated names a named MOUNT accepts (upper case); empty = any */
extern int sim_fw_dead;             /* the firmware does not answer any packet (no OGDM, a stock GDEMU, no board) */
extern int sim_trace;               /* print "FW <12 hex bytes>" for every packet */
extern int sim_mount_in_session;    /* mount-type commands seen while a write session was open */
extern int sim_tur_calls;           /* TEST UNIT READY packets seen */
extern uint32_t sim_fw_version;     /* 52 00 reply (boot and firmware word) */
extern int sim_stock;               /* a stock GDEMU 6.0 (record 08): 52 00 answers 5.20.5 / 2.5.0, anything else ILLEGAL */
extern int sim_e0_packets;          /* 0xE0 packets seen */
extern int sim_ua_on_e0;            /* the next 0xE0 packet gets a UNIT ATTENTION */
extern uint32_t sim_probe_limit_max;

/* platform (plat_host.c) */
extern char sim_pc_root[260];       /* what "/pc" maps to */
extern int sim_quiet;
extern int sim_polls;               /* polls of the stop request so far */
extern int sim_stop_after_polls;    /* ng_plat_user_stop returns 1 from this poll on; <0 never */
extern uint32_t ng_pc_poll_ms;      /* xport_pc.c */

#endif
