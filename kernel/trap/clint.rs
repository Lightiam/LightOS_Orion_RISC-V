//! Timer programming.
//!
//! The wall clock is the CLINT's `mtime` (MMIO, readable from S-mode
//! on QEMU virt). Timer interrupts use the Sstc extension: S-mode
//! writes `stimecmp` directly, so no machine-mode trampoline is needed.
//! `mstart()` enables this by setting menvcfg.STCE and mcounteren.TM.
#![allow(unsafe_code)] // MMIO read + stimecmp CSR write

use crate::mem::layout::CLINT_BASE;

const CLINT_MTIME: usize = CLINT_BASE + 0xbff8;

/// QEMU virt timebase: 10 MHz.
pub const TIMEBASE_FREQ: u64 = 10_000_000;
/// 10 ms scheduler tick.
pub const TICK_INTERVAL: u64 = TIMEBASE_FREQ / 100;

/// Current time in timebase ticks.
pub fn time() -> u64 {
    unsafe { (CLINT_MTIME as *const u64).read_volatile() }
}

/// Arm the next supervisor timer interrupt `interval` ticks from now.
pub fn set_next(interval: u64) {
    let next = time().wrapping_add(interval);
    unsafe {
        // stimecmp is CSR 0x14d (Sstc); numeric form keeps older
        // assemblers happy.
        core::arch::asm!("csrw 0x14d, {}", in(reg) next);
    }
}

/// Start the periodic tick.
pub fn init() {
    set_next(TICK_INTERVAL);
}
