//! LightOS boot path: `_start` (boot/entry.S, M-mode) → `mstart`
//! (M-mode Rust, below) → `kinit` (S-mode, the kernel proper).
#![no_std]
#![no_main]
#![allow(unsafe_code)] // boot CSR programming is inherently unsafe asm

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use lightos::{fs, mem, nce, sched, trap, uart, uart_println};

core::arch::global_asm!(include_str!("../boot/entry.S"));

// mstatus.MPP: privilege level restored by mret.
const MSTATUS_MPP_MASK: usize = 3 << 11;
const MSTATUS_MPP_S: usize = 1 << 11;

// sie: supervisor interrupt enables (external / timer / software).
const SIE_SEIE: usize = 1 << 9;
const SIE_STIE: usize = 1 << 5;
const SIE_SSIE: usize = 1 << 1;

/// Machine-mode early init, hart 0 only. Configures delegation and
/// physical memory protection, then `mret`s into supervisor-mode
/// `kinit()`. Never returns.
#[no_mangle]
extern "C" fn mstart(hartid: usize, dtb: usize) -> ! {
    unsafe {
        // Next privilege level after mret: supervisor.
        let mut mstatus: usize;
        core::arch::asm!("csrr {}, mstatus", out(reg) mstatus);
        mstatus &= !MSTATUS_MPP_MASK;
        mstatus |= MSTATUS_MPP_S;
        core::arch::asm!("csrw mstatus, {}", in(reg) mstatus);

        // Land in kinit() with paging off.
        core::arch::asm!("csrw mepc, {}", in(reg) kinit as *const () as usize);
        core::arch::asm!("csrw satp, zero");

        // Delegate all exceptions and interrupts to S-mode; the kernel
        // proper never re-enters M-mode after this point.
        let all: usize = 0xffff;
        core::arch::asm!("csrw medeleg, {}", in(reg) all);
        core::arch::asm!("csrw mideleg, {}", in(reg) all);
        core::arch::asm!("csrw sie, {}", in(reg) SIE_SEIE | SIE_STIE | SIE_SSIE);

        // Let S-mode read cycle/time/instret, and enable the Sstc
        // extension (menvcfg.STCE, bit 63) so S-mode can program
        // stimecmp directly — no M-mode timer trampoline needed.
        core::arch::asm!("csrw mcounteren, {}", in(reg) 7_usize);
        core::arch::asm!("csrs 0x30a, {}", in(reg) 1_usize << 63);

        // PMP entry 0: allow S-mode access to all physical memory
        // (TOR, RWX, top = 2^56). Without this, the first S-mode fetch
        // faults back into M-mode.
        let pmp_top: usize = 0x3f_ffff_ffff_ffff;
        core::arch::asm!("csrw pmpaddr0, {}", in(reg) pmp_top);
        core::arch::asm!("csrw pmpcfg0, {}", in(reg) 0xf_usize);

        // Enter kinit(hartid, dtb) in S-mode.
        core::arch::asm!(
            "mv a0, {h}",
            "mv a1, {d}",
            "mret",
            h = in(reg) hartid,
            d = in(reg) dtb,
            options(noreturn),
        );
    }
}

/// Boot splash: ASCII rendering of the Orion.mp4 title card
/// (regenerate with scripts/make_splash.sh).
const SPLASH: &str = include_str!("../assets/splash.txt");

/// Supervisor-mode kernel entry point.
#[no_mangle]
extern "C" fn kinit(hartid: usize, dtb: usize) -> ! {
    uart::init();
    uart_println!("{}", SPLASH);
    uart_println!("LightOS booting...");
    uart_println!(
        "LightOS v{} — RISC-V RV64GC, hart {} in S-mode",
        env!("CARGO_PKG_VERSION"),
        hartid
    );
    uart_println!("device tree blob at {:#x}", dtb);
    uart_println!("[phase 0] milestone: bare boot + UART OK");

    mem::init();
    heap_smoke_test();
    uart_println!("[phase 1] milestone: MMU on, kernel heap OK");

    trap::init();
    fs::mount_root();
    nce::init(dtb);

    let pid = sched::process::spawn("init").expect("failed to spawn init");
    uart_println!("proc: spawned init as pid {}", pid);
    sched::schedule()
}

/// Exercise the global allocator and the live Sv39 translation.
fn heap_smoke_test() {
    let boxed = Box::new(0xdead_beef_u64);
    let mut v: Vec<usize> = Vec::new();
    for i in 0..1024 {
        v.push(i * 3);
    }
    assert_eq!(*boxed, 0xdead_beef);
    assert_eq!(v[1023], 1023 * 3);
    uart_println!(
        "heap: Box at {:p}, Vec[1024] at {:p} — allocations verified",
        &*boxed,
        v.as_ptr(),
    );
}
