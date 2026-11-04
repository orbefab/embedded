/* Code and its literals in SRAM through the instruction bus, data and the stack through the
 * data bus, on bytes of SRAM the code does not use. */
ENTRY(_start)

MEMORY
{
    iram (rx) : ORIGIN = 0x40380000, LENGTH = 0x10000
    dram (rw) : ORIGIN = 0x3fca0000, LENGTH = 0x10000
}

SECTIONS
{
    .vectors : ALIGN(1024) { KEEP(*(.vectors)) } > iram
    /* l32r reaches back, so every literal comes before every instruction. */
    .text : { *(.literal .literal.*) *(.text .text.*) } > iram
    .rodata : { *(.rodata .rodata.*) } > dram
    .data : { *(.data .data.*) } > dram
    .bss (NOLOAD) : ALIGN(4) {
        _bss_start = .;
        *(.bss .bss.* COMMON)
        . = ALIGN(4);
        _bss_end = .;
    } > dram
    _stack_top = ORIGIN(dram) + LENGTH(dram);
    /DISCARD/ : { *(.comment) *(.note.*) *(.xtensa.info) *(.xt.*) }
}
