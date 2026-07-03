//! Machine power control via the QEMU `virt` sifive_test finisher.
//!
//! Writing a magic word to the finisher MMIO register asks QEMU to
//! power off or reset the machine. We jumped straight to S-mode at boot
//! (no SBI firmware), so SBI shutdown is unavailable — this device is
//! the clean way to halt the emulator.
#![allow(unsafe_code)] // MMIO write to the finisher register

use crate::mem::layout::TEST_BASE;
use crate::{halt, uart_println};

const FINISHER_PASS: u32 = 0x5555; // power off (exit code 0)
const FINISHER_RESET: u32 = 0x7777; // reboot

/// Power the machine off. Never returns.
pub fn poweroff() -> ! {
    uart_println!("LightOS: powering off.");
    unsafe { (TEST_BASE as *mut u32).write_volatile(FINISHER_PASS) };
    halt()
}

/// Reset the machine. Never returns.
pub fn reboot() -> ! {
    uart_println!("LightOS: rebooting.");
    unsafe { (TEST_BASE as *mut u32).write_volatile(FINISHER_RESET) };
    halt()
}
