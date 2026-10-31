ENTRY(_start)
SECTIONS {
  . = 0x80010000;
  .text : { *(.text.start) *(.text*) }
  .rodata : { *(.rodata*) }
  .data : { *(.data*) *(.sdata*) }
  _gp = ALIGN(16) + 0x7FF0;
  __bss_start = .;
  .bss : { *(.sbss*) *(.bss*) *(COMMON) }
  __bss_end = ALIGN(4);
  /DISCARD/ : { *(.MIPS.*) *(.comment) *(.reginfo) *(.pdr) }
}
