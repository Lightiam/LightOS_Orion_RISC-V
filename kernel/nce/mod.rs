//! NCE (Neural Compute Element) hardware abstraction layer.
//!
//! Enumerates NCE accelerator slots from the device tree blob
//! (compatible = "lightrail,nce"). QEMU's virt machine carries no such
//! nodes, so when none are found the HAL registers emulated slots —
//! same descriptors, same power state machine, no MMIO side effects —
//! keeping the whole /dev/nce* surface exercisable before silicon.

pub mod affinity;
pub mod power;

use crate::lock::SpinLock;
use crate::uart_println;
use alloc::vec::Vec;
use power::PowerState;

#[derive(Clone, Copy)]
pub struct NceDescriptor {
    pub slot: usize,
    /// Control register window (doorbells, status).
    pub mmio_base: usize,
    /// Dedicated NCE-adjacent memory region.
    pub mem_base: usize,
    pub mem_size: usize,
}

struct Nce {
    desc: NceDescriptor,
    state: PowerState,
    /// False for emulated slots: skip real doorbell writes.
    hardware: bool,
}

/// NCE table. Lock invariant: guards slot state; doorbell MMIO writes
/// happen under it so state and hardware can never diverge.
static NCES: SpinLock<Vec<Nce>> = SpinLock::new(Vec::new());

/// Number of emulated slots registered when the DT has no NCE nodes.
const EMULATED_SLOTS: usize = 2;
/// Placeholder bases for emulated descriptors (documented fiction).
const EMULATED_MMIO_BASE: usize = 0x6000_0000;
const EMULATED_MEM_BASE: usize = 0x6100_0000;
const EMULATED_MEM_SIZE: usize = 0x10_0000; // 1 MiB per slot

/// Scan the flattened device tree for "lightrail,nce" compatibles.
/// v1 detects presence (full property parsing lands with real
/// hardware bring-up); absence selects the emulated fallback.
fn dtb_has_nce_nodes(dtb: usize) -> bool {
    if dtb == 0 {
        return false;
    }
    #[allow(unsafe_code)] // bounded read of the firmware-provided DTB
    let (magic, total) = unsafe {
        let magic = u32::from_be((dtb as *const u32).read_volatile());
        let total = u32::from_be(((dtb + 4) as *const u32).read_volatile());
        (magic, total as usize)
    };
    if magic != 0xd00d_feed || total == 0 || total > 4 * 1024 * 1024 {
        uart_println!("nce: no valid device tree at {:#x}", dtb);
        return false;
    }
    #[allow(unsafe_code)]
    let blob = unsafe { core::slice::from_raw_parts(dtb as *const u8, total) };
    blob.windows(b"lightrail,nce".len())
        .any(|w| w == b"lightrail,nce")
}

/// Enumerate NCEs (device tree first, emulated fallback) at boot.
pub fn init(dtb: usize) {
    let mut nces = NCES.lock();
    if dtb_has_nce_nodes(dtb) {
        // Real-hardware path: full FDT property parse is a follow-up;
        // no LightRail DT will lack it when silicon arrives.
        uart_println!("nce: device tree reports NCE nodes (hardware parse TODO)");
    }
    if nces.is_empty() {
        for slot in 0..EMULATED_SLOTS {
            nces.push(Nce {
                desc: NceDescriptor {
                    slot,
                    mmio_base: EMULATED_MMIO_BASE + slot * 0x1000,
                    mem_base: EMULATED_MEM_BASE + slot * EMULATED_MEM_SIZE,
                    mem_size: EMULATED_MEM_SIZE,
                },
                state: PowerState::Idle,
                hardware: false,
            });
        }
        uart_println!(
            "nce: no NCE nodes in device tree; {} emulated slots registered",
            EMULATED_SLOTS
        );
    }
}

pub fn count() -> usize {
    NCES.lock().len()
}

/// One-line descriptor for /dev/nceN reads.
pub fn describe(slot: usize) -> Option<alloc::string::String> {
    let nces = NCES.lock();
    let nce = nces.get(slot)?;
    Some(alloc::format!(
        "nce{}: state={} mmio={:#x} mem={:#x}+{:#x} {}\n",
        nce.desc.slot,
        nce.state.as_str(),
        nce.desc.mmio_base,
        nce.desc.mem_base,
        nce.desc.mem_size,
        if nce.hardware { "hw" } else { "emulated" },
    ))
}

/// Request a power transition; enforces the stepwise state machine.
pub fn set_power(slot: usize, next: PowerState) -> Result<(), &'static str> {
    let mut nces = NCES.lock();
    let nce = nces.get_mut(slot).ok_or("nce: no such slot")?;
    if nce.state == next {
        return Ok(());
    }
    if !nce.state.can_transition(next) {
        return Err("nce: invalid power transition");
    }
    if nce.hardware {
        #[allow(unsafe_code)] // doorbell write into the NCE MMIO window
        unsafe {
            ((nce.desc.mmio_base + power::REG_POWER) as *mut u32)
                .write_volatile(power::doorbell_value(next));
        }
    }
    nce.state = next;
    Ok(())
}
