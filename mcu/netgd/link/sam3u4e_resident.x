/* RESIDENT LAYOUT (cargo feature `resident`): the application sits behind
 * the 8 KB resident boot region, at APP_BASE = 0x00082000, length at most
 * 0x1E000 (docs/design/fw-bootloader.md 2-5, tools/fwimage/FORMAT.md
 * "Format 3", mcu/bootcore/src/app.rs). Everything else is link/sam3u4e.x.
 *
 * What is added on top of that script:
 *   - _stext moves to ORIGIN(FLASH) + 0x140 (cortex-m-rt's .text starts at
 *     _stext), `.app_desc` takes the 32 bytes at ORIGIN(FLASH) + 0x100,
 *     right behind the vector table, and `.app_ax` (the "NGAX" extension
 *     block, docs/design/resident-app-contract.md 3-5) the 32 bytes at
 *     ORIGIN(FLASH) + 0x120. src/resident.rs fills them in (magic, version,
 *     bitstream fields; min_gen); `fwimage` fills in length, CRC and stamp.
 *   - `.image_pad` + `.image_crc` close the image: the 4-byte trailer is the
 *     last thing in plane 0 and ends on a 512-byte boundary (ALIGN works on
 *     absolute addresses, and APP_BASE is a multiple of 512).
 *   - the vector table must fit in the 0x100 bytes before the descriptor.
 */
MEMORY
{
	FLASH : ORIGIN = 0x00082000, LENGTH = 0x0001E000 /* Flash0 behind the 8K resident, 120K */
	flash1 (W!RX) : ORIGIN = 0x00100000, LENGTH = 0x00020000 /* Flash1, 128K */
	RAM   : ORIGIN = 0x20000000, LENGTH = 0x00008000 /* Sram0, 32K */
	sram1  (W!RX) : ORIGIN = 0x20080000, LENGTH = 0x00004000 /* Sram1, 16K */
	nfc    (W!RX) : ORIGIN = 0x20100000, LENGTH = 0x00001080 /* NFC SRAM, 4224 */
}

/* SRAM1 is a second, non-contiguous 16K bank (SRAM0 ends at 0x20008000,
 * SRAM1 starts at 0x20080000), so the RAM region cannot simply be made
 * longer. Giving it its own output section lets the big staging buffers move
 * off SRAM0, where they were crowding the stack: statics had grown to 19,904
 * of the 32K bank, leaving under 13K for the stack, and a 4K buffer in a deep
 * FAT call chain was enough to run off the end of it -- which shows up as a
 * HardFault on an undefined instruction inside memset.
 *
 * NOLOAD: nothing zero-initialises this, so only put buffers here that are
 * always written before they are read.
 */
SECTIONS
{
  /* The FPGA bitstream is ~64K of a 128K flash bank, and the bank is not
   * where the code has to live: SAM3U4E has a second 128K plane at 0x00100000
   * that nothing used, so parking the bitstream there roughly halves flash
   * pressure on the code. Not contiguous with FLASH (which ends at 0x000A0000)
   * so, as with SRAM1, it needs its own region rather than a longer one. */
  .flash1 : ALIGN(4)
  {
    __bitstream_addr = .;
    KEEP(*(.flash1 .flash1.*));
    . = ALIGN(4);
  } > flash1

  /* The bitstream image ends with its own CRC-32 (docs/design/
   * resident-app-contract.md 4-3): padded so that the 4-byte `.bs_crc` ends
   * plane 1's image on a 512-byte boundary. `fwimage` fills the word in and
   * copies it into the "NGAX" block's bs_stamp; the app compares the two. */
  .bs_pad : ALIGN(4)
  {
    FILL(0xFFFFFFFF);
    . = ALIGN(. + 4, 512) - 4;
  } > flash1

  .bs_crc : ALIGN(4)
  {
    __bs_crc = .;
    KEEP(*(.bs_crc));
  } > flash1

  .sram1 (NOLOAD) : ALIGN(4)
  {
    *(.sram1 .sram1.*);
    . = ALIGN(4);
  } > sram1

  /* The NAND flash controller's SRAM, unused by this firmware. Empty on 4E,
   * where SRAM0 is not tight enough to be worth the NOLOAD caveat; it is
   * declared here only so both linker scripts stay the same shape. */
  .nfc (NOLOAD) : ALIGN(4)
  {
    *(.nfc .nfc.*);
    . = ALIGN(4);
  } > nfc

  /* Closes plane 0. Inserted after .bss: .data's load image (in FLASH) is
   * behind us in the FLASH location counter by then. */
  .image_pad : ALIGN(4)
  {
    FILL(0xFFFFFFFF);
    . = ALIGN(. + 4, 512) - 4;
  } > FLASH

  .image_crc : ALIGN(4)
  {
    KEEP(*(.image_crc));
  } > FLASH
} INSERT AFTER .bss;


/* The descriptor lives right behind the vector table, the extension block
 * behind the descriptor; .text starts behind the extension block. (Assigned
 * here, before cortex-m-rt's own PROVIDE.) */
_stext = ORIGIN(FLASH) + 0x140;

SECTIONS
{
  .app_desc ORIGIN(FLASH) + 0x100 : { KEEP(*(.app_desc)); } > FLASH
} INSERT AFTER .vector_table;

SECTIONS
{
  .app_ax ORIGIN(FLASH) + 0x120 : { KEEP(*(.app_ax)); } > FLASH
} INSERT AFTER .app_desc;

ASSERT(SIZEOF(.vector_table) <= 0x100, "the vector table does not fit in the 0x100 bytes before .app_desc");
ASSERT(SIZEOF(.app_desc) == 32, ".app_desc must be exactly 32 bytes (src/resident.rs)");
ASSERT(SIZEOF(.app_ax) == 32, ".app_ax must be exactly 32 bytes (src/resident.rs)");
ASSERT(SIZEOF(.bs_crc) == 4, ".bs_crc must be exactly 4 bytes (src/resident.rs)");
ASSERT((ADDR(.bs_crc) + 4) % 512 == 0, "the bitstream image (trailer included) must be a multiple of 512 bytes");
ASSERT(SIZEOF(.image_crc) == 4, ".image_crc must be exactly 4 bytes (src/resident.rs)");
ASSERT((ADDR(.image_crc) + 4) % 512 == 0, "the image (trailer included) must be a multiple of 512 bytes");
