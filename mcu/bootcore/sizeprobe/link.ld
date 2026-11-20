MEMORY { FLASH : ORIGIN = 0x00080000, LENGTH = 8K
         RAM   : ORIGIN = 0x20000000, LENGTH = 16K }
ENTRY(Reset)
SECTIONS {
  .vector_table ORIGIN(FLASH) : { KEEP(*(.vector_table)) } > FLASH
  .text : { *(.text .text.*) } > FLASH
  .rodata : { *(.rodata .rodata.*) } > FLASH
  /DISCARD/ : { *(.ARM.exidx .ARM.exidx.*) *(.ARM.extab*) }
}
