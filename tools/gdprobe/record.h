/* gdprobe: the text record. Pure C.
 *
 * One line per probe, "key=value" fields separated by single spaces, stable
 * key order, so two records diff cleanly. Lines starting with '#' are header /
 * trailer comments (versions, dates, labels) that compare.py ignores.
 *
 *   img=3 probe=REQ_SES_0 cdb=150000000600000000000000 rc=ok status=0x50 err=0x00 irr=0x03 bytes=6 data=010002000000 us=1234
 *   img=3 probe=HASH fad=45150 crc=0badf00d
 *
 * img=0 means "the image that was mounted at start". Free-form extra fields
 * (fad=..., ssz=...) come right after probe=.
 */
#ifndef GDPROBE_RECORD_H
#define GDPROBE_RECORD_H
#include <stddef.h>
#include <stdint.h>
#include "gdtypes.h"

#define REC_LINE_MAX 2304   /* every formatted line fits in this (incl. NUL) */
#define REC_DATA_MAX 512    /* data= shows at most this many bytes; bytes= has the true size */

const char *rec_rc_name(int rc);              /* "ok" "check" "timeout" "busy" "proto" "refused" */

/* Hex-encode n bytes; out needs 2n+1 bytes. Returns 2n. */
size_t rec_hex(char *out, const uint8_t *in, size_t n);

typedef struct {
    int img;                 /* folder number tag, 0 = as mounted */
    const char *name;        /* probe name, no spaces */
    const char *extra;       /* "k=v k=v" or NULL */
    const uint8_t *cdb;      /* 12 bytes or NULL */
    const gd_result *res;
    const uint8_t *data;     /* res->stored bytes, may be NULL */
} rec_probe;

/* Each returns the line length (without NUL), including the trailing '\n';
 * output is truncated safely if cap is too small. */
size_t rec_probe_line(char *out, size_t cap, const rec_probe *p);
size_t rec_hash_line(char *out, size_t cap, int img, uint32_t fad, uint32_t crc, uint32_t ssz);
/* "# key=value ..." header/trailer comment. */
size_t rec_comment_line(char *out, size_t cap, const char *text);
/* Free-form event line: img=N probe=NAME k=v ... (no cdb/result). */
size_t rec_event_line(char *out, size_t cap, int img, const char *name, const char *fields);

#endif
