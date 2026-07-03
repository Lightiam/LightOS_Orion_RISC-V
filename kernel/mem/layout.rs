//! Physical/virtual memory map constants for the QEMU `virt` machine
//! and accessors for the linker-provided section symbols.
#![allow(unsafe_code)] // taking addresses of extern linker symbols

/// 4 KiB pages everywhere (Sv39 leaf granularity).
pub const PAGE_SIZE: usize = 4096;

/// Start of DRAM on QEMU virt; the kernel is loaded here.
pub const RAM_START: usize = 0x8000_0000;
/// 128 MiB of RAM (`-m 128M`). Post-v1: parse from the device tree.
pub const RAM_SIZE: usize = 128 * 1024 * 1024;
pub const RAM_END: usize = RAM_START + RAM_SIZE;

/// Kernel virtual base == physical base (identity map for v1).
pub const KERNEL_BASE: usize = RAM_START;

// MMIO regions (QEMU virt machine).
pub const UART0_BASE: usize = 0x1000_0000;
pub const UART0_SIZE: usize = PAGE_SIZE;
pub const VIRTIO_BASE: usize = 0x1000_1000;
pub const VIRTIO_SIZE: usize = 8 * PAGE_SIZE; // 8 MMIO transports
pub const CLINT_BASE: usize = 0x0200_0000;
pub const CLINT_SIZE: usize = 0x1_0000;
pub const PLIC_BASE: usize = 0x0c00_0000;
pub const PLIC_SIZE: usize = 0x40_0000;

/// Number of harts the boot code stacks are sized for (see linker.ld).
pub const NUM_HARTS: usize = 4;

extern "C" {
    static _text_start: u8;
    static _text_end: u8;
    static _rodata_start: u8;
    static _rodata_end: u8;
    static _data_start: u8;
    static _bss_end: u8;
    static _stack_start: u8;
    static _stack_end: u8;
    static _heap_start: u8;
}

macro_rules! symbol_fn {
    ($fn_name:ident, $sym:ident) => {
        /// Address of the corresponding linker symbol.
        pub fn $fn_name() -> usize {
            unsafe { &$sym as *const u8 as usize }
        }
    };
}

symbol_fn!(text_start, _text_start);
symbol_fn!(text_end, _text_end);
symbol_fn!(rodata_start, _rodata_start);
symbol_fn!(rodata_end, _rodata_end);
symbol_fn!(data_start, _data_start);
symbol_fn!(bss_end, _bss_end);
symbol_fn!(stack_start, _stack_start);
symbol_fn!(stack_end, _stack_end);
symbol_fn!(heap_start, _heap_start);

/// Round `addr` down to a page boundary.
pub const fn page_down(addr: usize) -> usize {
    addr & !(PAGE_SIZE - 1)
}

/// Round `addr` up to a page boundary.
pub const fn page_up(addr: usize) -> usize {
    (addr + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}
