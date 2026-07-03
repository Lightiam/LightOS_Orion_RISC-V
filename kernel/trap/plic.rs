//! Platform-Level Interrupt Controller (PLIC) driver, QEMU virt.
//!
//! v1 routes everything to hart 0's supervisor context (context 1;
//! context 0 is hart 0 M-mode). SMP distribution is post-v1.
#![allow(unsafe_code)] // MMIO register access

use crate::mem::layout::PLIC_BASE;

/// UART0 interrupt source on QEMU virt.
pub const IRQ_UART0: u32 = 10;
/// First VirtIO MMIO transport (virtio-blk lands here).
pub const IRQ_VIRTIO0: u32 = 1;

const CONTEXT_HART0_S: usize = 1;

fn priority_reg(irq: u32) -> *mut u32 {
    (PLIC_BASE + 4 * irq as usize) as *mut u32
}

fn enable_reg(context: usize, irq: u32) -> *mut u32 {
    (PLIC_BASE + 0x2000 + context * 0x80 + (irq as usize / 32) * 4) as *mut u32
}

fn threshold_reg(context: usize) -> *mut u32 {
    (PLIC_BASE + 0x20_0000 + context * 0x1000) as *mut u32
}

fn claim_reg(context: usize) -> *mut u32 {
    (PLIC_BASE + 0x20_0004 + context * 0x1000) as *mut u32
}

/// Enable `irq` at priority 1 for hart 0 S-mode.
pub fn enable(irq: u32) {
    unsafe {
        priority_reg(irq).write_volatile(1);
        let reg = enable_reg(CONTEXT_HART0_S, irq);
        reg.write_volatile(reg.read_volatile() | (1 << (irq % 32)));
    }
}

/// Accept all enabled priorities on hart 0 S-mode.
pub fn init() {
    unsafe { threshold_reg(CONTEXT_HART0_S).write_volatile(0) };
}

/// Claim the highest-priority pending interrupt (0 = none).
pub fn claim() -> u32 {
    unsafe { claim_reg(CONTEXT_HART0_S).read_volatile() }
}

/// Signal completion of a claimed interrupt.
pub fn complete(irq: u32) {
    unsafe { claim_reg(CONTEXT_HART0_S).write_volatile(irq) };
}
