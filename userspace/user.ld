/* LightOS userspace link layout: flat ELF EXEC images at USER_BASE.
 * Segments are page-aligned so the kernel loader can map each LOAD
 * with exact permissions. */
OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS {
    . = 0x40000000;

    .text : ALIGN(4K) {
        *(.text.start)
        *(.text .text.*)
    }

    .rodata : ALIGN(4K) {
        *(.srodata .srodata.*)
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4K) {
        PROVIDE(__global_pointer$ = . + 0x800);
        *(.sdata .sdata.*)
        *(.data .data.*)
    }

    .bss : ALIGN(16) {
        *(.sbss .sbss.*)
        *(.bss .bss.*)
    }

    /DISCARD/ : { *(.eh_frame .eh_frame_hdr) }
}
