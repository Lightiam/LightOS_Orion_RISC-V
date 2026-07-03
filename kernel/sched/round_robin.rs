//! Round-robin pick policy: scan the process table starting after the
//! last-run slot, first Ready process wins. Upgrade path: CFS-style
//! virtual runtime once NCE affinity hints (Phase 7+) need weighting.

use super::process::{ProcState, MAX_PROCS, PROCS};

/// Choose the next Ready slot and mark it Running. Returns
/// `(slot, root_pa)` or `None` when everyone is blocked.
pub fn pick_next() -> Option<(usize, usize)> {
    let mut procs = PROCS.lock();
    let start = procs.last_run;
    for i in 1..=MAX_PROCS {
        let slot = (start + i) % MAX_PROCS;
        if let Some(p) = procs.slots[slot].as_mut() {
            if p.state == ProcState::Ready {
                p.state = ProcState::Running;
                let root_pa = p.root_pa;
                procs.last_run = slot;
                return Some((slot, root_pa));
            }
        }
    }
    None
}
