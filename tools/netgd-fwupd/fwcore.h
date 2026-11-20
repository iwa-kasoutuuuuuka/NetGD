/* netgd-fwupd: the pure core. No KOS, no registers: image parsing and validation, the
 * 0xF9 update protocol as a state machine over an abstract device, result text.
 * Host-tested against a fake device (tests/test_all.c); main.c / g1dev.c are the KOS glue.
 *
 * Sources of truth (read these, not this file, when they disagree):
 *   mcu/opengdemu/src/fwupdate.rs    handover: F9/4F + key, F9/53 + key
 *   mcu/fwupdater/src/main.rs        updater wire protocol and status codes
 *   mcu/fwupdater/src/logic.rs       header / vector checks
 *   tools/fwimage/FORMAT.md          OGFW v2, "Sending"
 */
#ifndef NETGD_FWUPD_FWCORE_H
#define NETGD_FWUPD_FWCORE_H
#include <stddef.h>
#include <stdint.h>

/* ---- OGFW v2 (logic.rs:5-24, FORMAT.md) --------------------------------------- */
#define FW_BLOCK          512u
#define FW_HDR_WORDS      9u
#define FW_MAGIC          0x5746474Fu          /* "OGFW" */
#define FW_FORMAT         2u
#define FW_KIND_CODE      0u
#define FW_KIND_BITSTREAM 1u
#define FW_PLANE0_BASE    0x00080000u
#define FW_PLANE1_BASE    0x00100000u
#define FW_PLANE_LEN      0x00020000u
#define FW_MAX_IMAGES     2
#define FW_MAX_FILE       ((size_t)FW_MAX_IMAGES * (FW_BLOCK + FW_PLANE_LEN))
#define FW_CIDR_2E        0x281A0761u
#define FW_CIDR_4E        0x28100961u
#define FW_CIDR_VERSION_MASK 0x1Fu

enum { FW_PART_NONE = 0, FW_PART_2E = 1, FW_PART_4E = 2 };

/* ---- the 0xF9 protocol (fwupdate.rs:38-60, main.rs:108-117) ------------------- */
#define FW_CMD            0xF9
#define FW_FEAT_ARM       0x4F
#define FW_FEAT_ENTER     0x53
#define FW_FEAT_HEADER    0x55
#define FW_FEAT_DATA      0x56
#define FW_FEAT_VERIFY    0x57
#define FW_FEAT_RESET     0x58
/* There is deliberately no constant for 0x59 (force reset): this program never sends it. */
#define FW_MAGIC_ARM      0x4F4744555041ULL    /* fwupdate.rs:48 */
#define FW_MAGIC_ENTER    0x4744454D5521ULL    /* fwupdate.rs:50 */

/* ERROR register values with the ERR status bit (main.rs:132-151). The FPGA only keeps
 * ABRT and a 4-bit sense key, so each is 0x?4. Plain 0x04 is what the normal firmware
 * (and the updater, for an unknown step) answers when it refuses. */
enum {
    FWS_OK = 0x00, FWS_REFUSED = 0x04,
    FWS_BADHEADER = 0x14, FWS_BADDEST = 0x24, FWS_BADLENGTH = 0x34, FWS_NOHEADER = 0x44,
    FWS_OVERRUN = 0x54, FWS_FLASHERROR = 0x64, FWS_CRCMISMATCH = 0x74, FWS_BADTARGET = 0x84,
    FWS_LOCKED = 0x94, FWS_BADVECTOR = 0xA4, FWS_NOTVERIFIED = 0xB4
};
#define FW_ST_ERR 0x01
#define FW_ST_DRQ 0x08
#define FW_ST_BSY 0x80

/* ---- CRC-32 (zlib) ------------------------------------------------------------- */
uint32_t fw_crc32_update(uint32_t crc, const void *data, size_t n);   /* start with crc = 0 */
#define fw_crc32(p, n) fw_crc32_update(0, (p), (n))

/* ---- image files ---------------------------------------------------------------- */
typedef struct {
    uint32_t kind, target_cidr, dest, len, crc, version;
} fw_header;

typedef enum {
    FWV_OK = 0, FWV_EMPTY, FWV_TOO_BIG, FWV_SHORT, FWV_MAGIC, FWV_FORMAT, FWV_HDR_CRC, FWV_KIND,
    FWV_TARGET, FWV_DEST, FWV_LEN, FWV_TRUNC, FWV_PAYLOAD_CRC, FWV_VECTOR, FWV_MIXED, FWV_DUP,
    FWV__COUNT
} fw_verr;

typedef struct {
    fw_header h;
    const uint8_t *header_block;   /* the 512 bytes as they are sent */
    const uint8_t *payload;        /* h.len bytes */
} fw_image;

typedef struct {
    int n;
    fw_image img[FW_MAX_IMAGES];
    int part;                      /* FW_PART_* the images are for */
} fw_file;

typedef struct { fw_verr err; int index; } fw_vres;

/* Same checks as fwimage.iter_images + validate_file (and the updater's before it
 * erases anything). Returns 0 and fills *f, or -1 and *res. `d` must outlive *f. */
int fw_parse_file(const uint8_t *d, size_t n, fw_file *f, fw_vres *res);
fw_verr fw_header_parse(const uint8_t *block512, fw_header *h);   /* magic, format, CRC, kind */
fw_verr fw_header_check(const fw_header *h, int part);           /* target, dest, length */
int fw_part_from_cidr(uint32_t cidr);
const char *fw_part_name(int part);                 /* "SAM3U2E" / "SAM3U4E" */
uint32_t fw_part_sram0_end(int part);
int fw_check_vectors(const uint32_t v[4], uint32_t dest, uint32_t len, int part);   /* 0 ok */

/* Reading the image from /pc/ in modest chunks (a long single read over dcload-ip is not
 * something to rely on). Reads at most cap bytes. 0 ok, -1 cannot open, -2 read error. */
typedef struct fw_rd {
    void *(*open)(const char *path);
    size_t (*read)(void *f, void *buf, size_t n);
    int (*error)(void *f);
    void (*close)(void *f);
} fw_rd;
#define FW_READ_CHUNK 8192u
int fw_read_all(const fw_rd *io, const char *path, uint8_t *buf, size_t cap, size_t *len);

/* ---- the device, as the protocol sees it ------------------------------------------- */
enum { FWIO_OK = 0, FWIO_TIMEOUT = 1, FWIO_BUSY = 2, FWIO_PROTO = 3 };

typedef struct {
    int rc;               /* FWIO_*: OK = the command ended (BSY and DRQ clear) */
    uint8_t status;       /* ATA status at the end (FW_ST_ERR = the board refused) */
    uint8_t error;        /* ERROR register, valid when status has FW_ST_ERR */
    uint32_t blocks;      /* data blocks actually written */
} fw_reply;

typedef struct {
    int image, nimages;
    uint32_t blocks_done, blocks_total;    /* of this image, payload only */
    int msgid;                              /* FWM_STEP_* */
    int attempt;                            /* 0 = first try */
} fw_progress;

typedef struct fw_env {
    void *ctx;
    /* One vendor 0xF9 command on the secondary device. `count` goes in the sector-count
     * register (a data command's block count, 0 = 256). If use_key, `key` is written to
     * the six LBA bytes in LBA48 order. If nblocks > 0, PIO-write nblocks x 512 bytes
     * from `data` as the device raises DRQ. `timeout_ms` bounds every single wait.
     * Always fills *r; returns r->rc. */
    int (*f9)(void *ctx, uint8_t feat, uint8_t count, int use_key, uint64_t key,
              const uint8_t *data, uint32_t nblocks, uint32_t timeout_ms, fw_reply *r);
    /* ATA IDENTIFY DEVICE of the secondary, 512 bytes. May be NULL (then not checked). */
    int (*identify)(void *ctx, uint8_t *buf512, uint32_t timeout_ms, fw_reply *r);
    /* Called once after the device was recognised, before the first F9: PIO timing. May be NULL. */
    int (*prepare)(void *ctx);
    uint32_t (*now_ms)(void *ctx);                    /* may be NULL */
    void (*sleep_ms)(void *ctx, uint32_t ms);         /* may be NULL */
    void (*progress)(void *ctx, const fw_progress *p);/* may be NULL */
    void (*log)(void *ctx, const char *line);         /* one line, no '\n'; may be NULL */
} fw_env;

typedef struct {
    uint32_t chunk_blocks;     /* data blocks per F9/56 command, 1..256 (default 64) */
    int max_retries;           /* resends of one image after a retryable error (default 2) */
    uint32_t t_cmd_ms;         /* arm, enter, header, reset (default 5000) */
    uint32_t t_data_ms;        /* each wait inside a data command (default 10000) */
    uint32_t t_verify_ms;      /* verify, which also programs page 0 (default 60000) */
} fw_options;
void fw_options_default(fw_options *o);

typedef enum {
    FWR_OK = 0,
    FWR_OK_UNCONFIRMED,    /* everything verified, reset requested, no clean answer */
    FWR_NO_RESET,          /* everything verified, the board refused the reset */
    FWR_NOT_NETGD,         /* IDENTIFY says it is not NetGD: nothing sent */
    FWR_IDENT_FAIL,        /* IDENTIFY failed: nothing sent */
    FWR_REFUSED_START,     /* arm refused and the updater is not running: nothing sent */
    FWR_ENTER_FAILED,      /* armed, but the updater did not start */
    FWR_REJECTED,          /* the updater refused an image before writing it */
    FWR_FAILED,            /* a write/verify failed (flash may be half-written) */
    FWR_IO,                /* the board stopped answering */
    FWR_CANCELLED          /* the user said no on the confirmation screen: nothing sent */
} fw_outcome;

typedef struct {
    fw_outcome outcome;
    int handed_over;          /* the RAM updater is (or was) running */
    int resumed;              /* ... and was already running when we started */
    int flash_touched;        /* flash may have been modified (conservative) */
    int images_verified;
    int failed_image;         /* index, or -1 */
    int fail_msgid;           /* FWM_STEP_* of the failing step, or 0 */
    int io_rc;                /* FWIO_* of the failing step */
    uint8_t dev_status, dev_error;   /* what the board answered at the failing step */
    int retries_used;
} fw_report;

/* Identify check: 0 = NetGD (model string says OpenGDEMU/NetGD), 1 = some other device,
 * -1 = IDENTIFY failed. `model` receives the trimmed model string (>= 41 bytes). */
int fw_check_device(const fw_env *env, char *model, size_t cap);
int fw_model_is_netgd(const char *model);

/* Perform the whole update. 0 only if every image verified (rep->outcome FWR_OK or
 * FWR_OK_UNCONFIRMED); the caller must have shown the confirmation first. */
int fw_update(const fw_env *env, const fw_file *f, const fw_options *o, fw_report *rep);

/* ---- status decoding and result text --------------------------------------------- */
int fw_status_msgid(uint8_t error);        /* FWM_ST_* for an ERROR register value */
const char *fw_status_name(uint8_t error); /* "BadTarget", ... "unknown" */
int fw_verr_msgid(fw_verr e);              /* FWM_V_* */
int fw_io_msgid(int rc);                   /* FWM_IO_* */
int fw_outcome_msgid(const fw_report *rep);/* the FWM_OUT_* to show last */

/* ---- messages and text layout (fwmsg.c) ------------------------------------------- */
typedef enum {
#define FWMSG(id, en, ja8, jaeuc) id,
#include "fwmsg_gen.h"
#undef FWMSG
    FWM__COUNT
} fw_msgid;
const char *fw_msg_en(int id);
const char *fw_msg_ja(int id);        /* UTF-8: the result file and the dcload console */
const char *fw_msg_euc(int id);       /* EUC-JP: the BIOS font on the Dreamcast screen */
/* Display width of an EUC-JP string in half-width cells (wide = 2, SS2 kana = 1; '\n' ends). */
int fw_text_cells(const char *euc, size_t len);
/* Break `euc` into lines of at most `cells` cells (and at '\n'). Writes NUL-terminated
 * lines of up to linecap-1 bytes into out[0..maxlines-1]; returns the number of lines. */
int fw_text_wrap(const char *euc, int cells, char *out, size_t linecap, int maxlines);

/* ---- result file text -------------------------------------------------------------- */
/* The two ends of the result file (UTF-8, key=value lines plus the Japanese message).
 * The lines of the run itself arrive through env->log in between. Both return the length
 * written (always NUL-terminated, truncated rather than overflowing). */
size_t fw_describe_text(char *buf, size_t cap, const char *image_path, size_t file_size,
                        const fw_file *f, const char *model);
size_t fw_summary_text(char *buf, size_t cap, const fw_report *rep);

/* ---- small config file: "image=<path>" lines (# comments) ---------------------------- */
int fw_cfg_image_path(const char *text, char *out, size_t cap);   /* 1 if found */

#endif
