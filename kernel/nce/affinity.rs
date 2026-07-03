//! NCE-aware affinity hints.
//!
//! Userspace (e.g. a Ray worker) declares which NCE slots it wants to
//! sit near via a sched_setaffinity-style syscall; the mask is stored
//! per process. On the single-hart v1 scheduler it is a recorded hint;
//! the SMP scheduler upgrade consumes it to pin workers to
//! NCE-adjacent harts (see round_robin.rs upgrade note).

use crate::sched::process::PROCS;
use crate::sched::CURRENT;
use core::sync::atomic::Ordering;

/// Store the affinity mask for the current process (pid 0 = self is
/// the only supported target in v1).
pub fn set_current(mask: usize) -> isize {
    let cur = CURRENT.load(Ordering::Relaxed);
    let mut procs = PROCS.lock();
    match procs.slots[cur].as_mut() {
        Some(p) => {
            p.nce_affinity = mask;
            0
        }
        None => -3, // ESRCH
    }
}

pub fn get_current() -> usize {
    let cur = CURRENT.load(Ordering::Relaxed);
    let procs = PROCS.lock();
    procs.slots[cur].as_ref().map_or(0, |p| p.nce_affinity)
}
