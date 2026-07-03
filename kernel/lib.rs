//! LightOS kernel crate root.
//!
//! Royalty-free, UNIX-style RISC-V (RV64GC) kernel for LightRail AI NCE
//! hardware. `#![no_std]`, zero external dependencies. Unsafe code is
//! denied crate-wide and re-allowed per module with a justification.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

pub mod elf;
pub mod lock;
pub mod mem;
pub mod prog;
pub mod sched;
pub mod syscall;
pub mod trap;
pub mod uart;

use core::panic::PanicInfo;

/// Kernel panic — report on UART, then halt this hart forever.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    uart_println!("KERNEL PANIC: {}", info);
    halt()
}

/// Park the current hart: interrupts stay off at this point, so `wfi`
/// never wakes into handler code.
#[allow(unsafe_code)] // wfi is a privileged instruction, asm required
pub fn halt() -> ! {
    loop {
        unsafe { core::arch::asm!("wfi") }
    }
}

/// Print to the UART console (no trailing newline).
#[macro_export]
macro_rules! uart_print {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!($crate::uart::UART.lock(), $($arg)*);
    }};
}

/// Print to the UART console with a trailing newline.
#[macro_export]
macro_rules! uart_println {
    () => { $crate::uart_print!("\n") };
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut uart = $crate::uart::UART.lock();
        let _ = write!(uart, $($arg)*);
        let _ = write!(uart, "\n");
    }};
}
