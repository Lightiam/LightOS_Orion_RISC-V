/*
 * LightOS linker script — RISC-V RV64GC, QEMU `virt` machine.
 *
 * With `-bios none` QEMU loads the kernel ELF at the start of DRAM
 * (0x8000_0000) and every hart begins executing there in machine mode.
 * The layout below keeps the boot entry first, then the usual sections,
 * then per-hart boot stacks. Everything from `_heap_start` to
 * `_memory_end` is handed to the page-frame allocator.
 */

OUTPUT_ARCH(riscv)
ENTRY(_start)

MEMORY {
    ram (wxa) : ORIGIN = 0x80000000, LENGTH = 128M
}

/* Must match NUM_HARTS/BOOT_STACK_SIZE in kernel/mem/layout.rs */
BOOT_STACK_SIZE = 64K;
NUM_HARTS = 4;

SECTIONS {
    .text : ALIGN(4K) {
        PROVIDE(_text_start = .);
        *(.text.entry)          /* reset vector must sit at 0x80000000 */
        *(.text .text.*)
        PROVIDE(_text_end = .);
    } > ram

    .rodata : ALIGN(4K) {
        PROVIDE(_rodata_start = .);
        *(.srodata .srodata.*)
        *(.rodata .rodata.*)
        PROVIDE(_rodata_end = .);
    } > ram

    .data : ALIGN(4K) {
        PROVIDE(_data_start = .);
        /* __global_pointer$ enables linker relaxation of gp-relative loads */
        PROVIDE(__global_pointer$ = . + 0x800);
        *(.sdata .sdata.*)
        *(.data .data.*)
        PROVIDE(_data_end = .);
    } > ram

    .bss : ALIGN(4K) {
        PROVIDE(_bss_start = .);
        *(.sbss .sbss.*)
        *(.bss .bss.*)
        . = ALIGN(8);
        PROVIDE(_bss_end = .);
    } > ram

    .stack (NOLOAD) : ALIGN(4K) {
        PROVIDE(_stack_start = .);
        . += BOOT_STACK_SIZE * NUM_HARTS;
        PROVIDE(_stack_end = .);
    } > ram

    PROVIDE(_memory_start = ORIGIN(ram));
    PROVIDE(_heap_start = ALIGN(_stack_end, 4K));
    PROVIDE(_memory_end = ORIGIN(ram) + LENGTH(ram));

    /DISCARD/ : {
        *(.eh_frame .eh_frame_hdr)
    }
}
