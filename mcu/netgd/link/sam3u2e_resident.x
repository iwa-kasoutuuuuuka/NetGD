/* RESIDENT LAYOUT (cargo feature `resident`): the application sits behind
 * the 8 KB resident boot region, at APP_BASE = 0x00082000, length at most
 * 0x1E000 (docs/design/fw-bootloader.md 2-5, tools/fwimage/FORMAT.md
 * "Format 3", mcu/bootcore/src/app.rs). Everything else is link/sam3u2e.x.
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
	/* SAM3U2E has a single 128K flash plane -- there is no 0x00100000
	 * plane to park the bitstream in, so it shares FLASH with the code.
	 * That is the whole reason this part is tight: 4E spends 128K on code
	 * and a second 128K on the bitstream; 2E has to fit both in one bank. */
	FLASH : ORIGIN = 0x00082000, LENGTH = 0x0001E000 /* Flash0 behind the 8K resident, 120K */
	RAM   : ORIGIN = 0x20000000, LENGTH = 0x00004000 /* Sram0, 16K */
	sram1 (W!RX) : ORIGIN = 0x20080000, LENGTH = 0x00004000 /* Sram1, 16K */
	nfc   (W!RX) : ORIGIN = 0x20100000, LENGTH = 0x00001080 /* NFC SRAM, 4224 */
}

/* See link/sam3u4e.x for what each of these sections is for; the only
 * difference here is that .flash1 has no second plane to go to. */
SECTIONS
{
  .flash1 : ALIGN(4)
  {
    __bitstream_addr = .;
    KEEP(*(.flash1 .flash1.*));
    . = ALIGN(4);
  } > FLASH

  /* Closes plane 0, after the bitstream: on 2E the bitstream is part of the
   * image, and the trailer is its last four bytes. */
  .image_pad : ALIGN(4)
  {
    FILL(0xFFFFFFFF);
    . = ALIGN(. + 4, 512) - 4;
  } > FLASH

  .image_crc : ALIGN(4)
  {
    KEEP(*(.image_crc));
  } > FLASH

  .sram1 (NOLOAD) : ALIGN(4)
  {
    *(.sram1 .sram1.*);
    . = ALIGN(4);
  } > sram1

  /* The NAND flash controller's SRAM. Nothing in this firmware drives the
   * NFC, so on 2E its 4,224 bytes are the one block of RAM available for
   * free -- which is exactly enough for cdda::RING. NOLOAD, so anything
   * put here must be written before it is read (cdda::init zeroes it). */
  .nfc (NOLOAD) : ALIGN(4)
  {
    *(.nfc .nfc.*);
    . = ALIGN(4);
  } > nfc
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
ASSERT(SIZEOF(.image_crc) == 4, ".image_crc must be exactly 4 bytes (src/resident.rs)");
ASSERT((ADDR(.image_crc) + 4) % 512 == 0, "the image (trailer included) must be a multiple of 512 bytes");
