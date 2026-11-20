/* gdprobe: configuration file parser (/pc/.../gdprobe.cfg). Pure C.
 *
 * Plain "key=value" lines; '#' starts a comment. Unknown keys are ignored
 * (and counted), so a typo cannot enable something by accident.
 *
 *   label=stock            device name for the file name and the record (stock | netgd | ...)
 *   seq=1                  run number, used in the file name when the Dreamcast has no valid clock
 *   switch=off             on = run the image-switch loop (default off)
 *   images=3,4,5,6,7       folder numbers to visit, in order (only with switch=on); ranges 3-5 ok
 *   return_to=2            folder to switch back to at the end (0/absent = no switch back)
 *   switch_timeout_ms=15000
 *   identify=off           ATA IDENTIFY PACKET DEVICE (0xA1), default off
 *   throughput=on          timed CD_READ throughput + per-sector hashes
 *   tp_batches=64          batches
 *   tp_sectors=16          sectors per batch (1..16)
 *   seek=on                seek-time sampling
 *   seek_n=16              samples (2..64)
 *   seek_span=50000        sectors from the IP.BIN area to spread the samples over
 *   ip_fad=0               0 = auto (REQ_SES 2, else 45150)
 *   cdread_flags=0x24      CD_READ CDB byte 1 (bit 0 must stay 0 = FAD)
 *   timeout_ms=3000        per-command timeout
 */
#ifndef GDPROBE_CFG_H
#define GDPROBE_CFG_H
#include <stdint.h>

#define GP_MAX_IMAGES 32

typedef struct {
    char label[32];
    int seq;
    int do_switch;
    int images[GP_MAX_IMAGES];
    int n_images;
    int return_to;
    uint32_t switch_timeout_ms;
    int do_identify;
    int do_throughput;
    int tp_batches;
    int tp_sectors;
    int do_seek;
    int seek_n;
    uint32_t seek_span;
    uint32_t ip_fad;
    uint8_t cdread_flags;
    uint32_t timeout_ms;
    int unknown_keys;     /* lines that did not parse */
} gp_config;

void gp_config_defaults(gp_config *c);
/* Parse NUL-terminated text over the defaults already in *c. Always succeeds. */
void gp_config_parse(gp_config *c, const char *text);

#endif
