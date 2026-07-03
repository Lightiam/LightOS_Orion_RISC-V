//! Scheduler: preemptive round-robin over user processes.
//!
//! LightOS v1 switches *trap frames*, not kernel stacks: the kernel is
//! non-preemptible, every kernel entry runs to completion on the boot
//! hart's stack, and returning to user space goes through `__user_ret`
//! (sched/switch.S) which restores the chosen process's frame.
#![allow(unsafe_code)] // CSR access + the noreturn jump into switch.S

pub mod process;
pub mod round_robin;

use crate::mem::mmu;
use crate::trap;
use crate::trap::context::TrapFrame;
use core::sync::atomic::{AtomicUsize, Ordering};

core::arch::global_asm!(include_str!("switch.S"));

extern "C" {
    fn __user_ret(tf: *mut TrapFrame, satp: usize) -> !;
}

/// Slot index of the process currently in (or headed to) user mode.
pub static CURRENT: AtomicUsize = AtomicUsize::new(process::NO_PROC);

const SSTATUS_SPP: usize = 1 << 8;
const SSTATUS_SPIE: usize = 1 << 5;
const SSTATUS_SUM: usize = 1 << 18;

/// Pick the next Ready process and enter it; idle in wfi when nobody
/// is runnable. Never returns.
pub fn schedule() -> ! {
    loop {
        trap::disable_interrupts();
        if let Some((slot, root_pa)) = round_robin::pick_next() {
            CURRENT.store(slot, Ordering::Relaxed);
            enter_user(slot, root_pa);
        }
        // Nothing runnable: wait for a timer/console interrupt to wake
        // somebody up. Interrupts must be enabled around wfi or the
        // wake-up can never be delivered.
        trap::enable_interrupts();
        unsafe { core::arch::asm!("wfi") };
    }
}

/// Timer preemption: current process goes back to Ready, pick again.
/// Never returns.
pub fn yield_current() -> ! {
    let cur = CURRENT.load(Ordering::Relaxed);
    {
        let mut procs = process::PROCS.lock();
        if let Some(p) = procs.slots[cur].as_mut() {
            if p.state == process::ProcState::Running {
                p.state = process::ProcState::Ready;
            }
        }
    }
    CURRENT.store(process::NO_PROC, Ordering::Relaxed);
    schedule()
}

/// Re-enter the current process (non-blocking trap exit). Never returns.
pub fn resume_current() -> ! {
    let cur = CURRENT.load(Ordering::Relaxed);
    let root_pa = {
        let procs = process::PROCS.lock();
        procs.slots[cur]
            .as_ref()
            .expect("resume: no current process")
            .root_pa
    };
    enter_user(cur, root_pa);
}

fn enter_user(slot: usize, root_pa: usize) -> ! {
    let tf = process::tf_mut(slot);

    // sret target: U-mode (SPP=0), interrupts on after sret (SPIE),
    // and keep SUM so kernel copies to user pages keep working on the
    // next trap.
    let mut sstatus: usize;
    unsafe { core::arch::asm!("csrr {}, sstatus", out(reg) sstatus) };
    sstatus &= !SSTATUS_SPP;
    sstatus |= SSTATUS_SPIE | SSTATUS_SUM;
    tf.sstatus = sstatus;

    let satp = mmu::table_at(root_pa).satp();
    unsafe { __user_ret(tf, satp) }
}
