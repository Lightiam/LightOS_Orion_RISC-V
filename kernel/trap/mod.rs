//! Trap vector registration and dispatch.
#![allow(unsafe_code)] // trap trampoline asm, CSR access, no_mangle ABI entry

pub mod clint;
pub mod context;
pub mod plic;

use crate::{uart, uart_println};
use context::TrapFrame;
use core::sync::atomic::{AtomicUsize, Ordering};

core::arch::global_asm!(include_str!("vector.S"));

const SCAUSE_INTERRUPT: usize = 1 << 63;
const IRQ_S_SOFT: usize = 1;
const IRQ_S_TIMER: usize = 5;
const IRQ_S_EXTERNAL: usize = 9;

/// Monotonic 10 ms tick counter (diagnostics + scheduler heartbeat).
pub static TICKS: AtomicUsize = AtomicUsize::new(0);

extern "C" {
    fn __trap_vector();
}

/// Install the trap vector and unmask the interrupt sources.
#[allow(unsafe_code)] // stvec/sstatus CSR programming
pub fn init() {
    unsafe {
        // Direct mode: all traps to __trap_vector (4-byte aligned).
        core::arch::asm!(
            "csrw stvec, {}",
            in(reg) __trap_vector as *const () as usize,
        );
    }
    plic::init();
    plic::enable(plic::IRQ_UART0);
    uart::UART.lock().enable_rx_interrupt();
    clint::init();
    enable_interrupts();
    uart_println!("trap: stvec installed, PLIC + timer armed");
}

/// Set sstatus.SIE — supervisor interrupts on.
#[allow(unsafe_code)]
pub fn enable_interrupts() {
    unsafe { core::arch::asm!("csrsi sstatus, 2") };
}

/// Clear sstatus.SIE — supervisor interrupts off.
#[allow(unsafe_code)]
pub fn disable_interrupts() {
    unsafe { core::arch::asm!("csrci sstatus, 2") };
}

fn read_csr_scause() -> usize {
    let v: usize;
    #[allow(unsafe_code)]
    unsafe {
        core::arch::asm!("csrr {}, scause", out(reg) v)
    };
    v
}

fn read_csr_stval() -> usize {
    let v: usize;
    #[allow(unsafe_code)]
    unsafe {
        core::arch::asm!("csrr {}, stval", out(reg) v)
    };
    v
}

/// Central trap dispatch, called from the assembly trampoline with the
/// saved context. Interrupts are handled; unexpected exceptions panic
/// (Phase 4 adds the ecall path).
#[no_mangle]
extern "C" fn trap_handler(tf: &mut TrapFrame) {
    let scause = read_csr_scause();

    if scause & SCAUSE_INTERRUPT != 0 {
        match scause & !SCAUSE_INTERRUPT {
            IRQ_S_TIMER => timer_tick(),
            IRQ_S_EXTERNAL => external_interrupt(),
            IRQ_S_SOFT => {
                // No IPIs yet (single-hart scheduling until SMP work).
                uart_println!("trap: spurious software interrupt");
            }
            other => uart_println!("trap: unknown interrupt {}", other),
        }
        return;
    }

    // Synchronous exception: fatal until the syscall path lands.
    panic!(
        "unhandled exception: scause={:#x} ({}) sepc={:#x} stval={:#x}",
        scause,
        exception_name(scause),
        tf.sepc,
        read_csr_stval(),
    );
}

fn timer_tick() {
    let ticks = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    if ticks == 100 {
        uart_println!("timer: 100 ticks (1.0 s at 10 ms/tick)");
        uart_println!("[phase 2] milestone: timer + IRQ-driven UART live");
    }
    clint::set_next(clint::TICK_INTERVAL);
}

fn external_interrupt() {
    let irq = plic::claim();
    match irq {
        0 => {} // spurious / already claimed
        plic::IRQ_UART0 => {
            uart::handle_interrupt();
            plic::complete(irq);
        }
        other => {
            uart_println!("plic: unexpected irq {}", other);
            plic::complete(other);
        }
    }
}

fn exception_name(scause: usize) -> &'static str {
    match scause {
        0 => "instruction address misaligned",
        1 => "instruction access fault",
        2 => "illegal instruction",
        3 => "breakpoint",
        5 => "load access fault",
        7 => "store access fault",
        8 => "ecall from U-mode",
        9 => "ecall from S-mode",
        12 => "instruction page fault",
        13 => "load page fault",
        15 => "store page fault",
        _ => "unknown",
    }
}
