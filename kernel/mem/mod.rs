//! Memory subsystem: physical page frames, kernel heap, Sv39 paging.

pub mod kalloc;
pub mod layout;
pub mod mmu;
pub mod page;

use crate::uart_println;

/// Bring up the whole memory subsystem on the boot hart:
/// frame allocator → kernel heap → kernel page table → paging on.
pub fn init() {
    page::init(layout::heap_start());
    kalloc::init();
    let root = mmu::kernel_page_table();
    mmu::enable(root);
    uart_println!(
        "mem: {} KiB free, heap at {:#x}, Sv39 paging on",
        page::free_frames() * layout::PAGE_SIZE / 1024,
        layout::heap_start(),
    );
}
