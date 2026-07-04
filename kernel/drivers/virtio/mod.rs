//! VirtIO MMIO transport (legacy, version 1 — what QEMU's
//! virtio-mmio devices speak by default).
//!
//! Probes the 8 transport slots on the QEMU virt machine and performs
//! the legacy init dance: status ACKNOWLEDGE→DRIVER, feature
//! negotiation, GuestPageSize, queue setup via QueuePFN, DRIVER_OK.
#![allow(unsafe_code)] // MMIO + shared DMA rings are raw memory by nature

pub mod blk;
pub mod net;

use crate::mem::layout::{PAGE_SIZE, VIRTIO_BASE};
use crate::uart_println;

// MMIO register offsets (legacy layout).
const MAGIC_VALUE: usize = 0x000;
const VERSION: usize = 0x004;
const DEVICE_ID: usize = 0x008;
pub(crate) const HOST_FEATURES: usize = 0x010;
pub(crate) const GUEST_FEATURES: usize = 0x020;
pub(crate) const GUEST_PAGE_SIZE: usize = 0x028;
pub(crate) const QUEUE_SEL: usize = 0x030;
const QUEUE_NUM_MAX: usize = 0x034;
const QUEUE_NUM: usize = 0x038;
const QUEUE_ALIGN: usize = 0x03c;
const QUEUE_PFN: usize = 0x040;
pub(crate) const QUEUE_NOTIFY: usize = 0x050;
pub(crate) const INTERRUPT_STATUS: usize = 0x060;
pub(crate) const INTERRUPT_ACK: usize = 0x064;
pub(crate) const STATUS: usize = 0x070;
pub(crate) const CONFIG: usize = 0x100;

pub(crate) const MAGIC: u32 = 0x7472_6976; // "virt"
const DEVICE_ID_BLOCK: u32 = 2;
pub(crate) const DEVICE_ID_NET: u32 = 1;
pub(crate) const MAGIC_VALUE_OFF: usize = MAGIC_VALUE;
pub(crate) const VERSION_OFF: usize = VERSION;
pub(crate) const DEVICE_ID_OFF: usize = DEVICE_ID;

pub(crate) const STATUS_ACKNOWLEDGE: u32 = 1;
pub(crate) const STATUS_DRIVER: u32 = 2;
pub(crate) const STATUS_DRIVER_OK: u32 = 4;

/// Number of descriptors per queue (power of two).
pub const QUEUE_SIZE: usize = 8;

pub fn reg_read(base: usize, off: usize) -> u32 {
    unsafe { ((base + off) as *const u32).read_volatile() }
}

pub fn reg_write(base: usize, off: usize, v: u32) {
    unsafe { ((base + off) as *mut u32).write_volatile(v) }
}

// Virtqueue structures (legacy layout, physically contiguous).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Descriptor {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

pub const DESC_F_NEXT: u16 = 1;
pub const DESC_F_WRITE: u16 = 2; // device writes this buffer

#[repr(C)]
pub struct AvailRing {
    pub flags: u16,
    pub idx: u16,
    pub ring: [u16; QUEUE_SIZE],
    pub used_event: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UsedElem {
    pub id: u32,
    pub len: u32,
}

#[repr(C)]
pub struct UsedRing {
    pub flags: u16,
    pub idx: u16,
    pub ring: [UsedElem; QUEUE_SIZE],
    pub avail_event: u16,
}

/// One initialized legacy virtqueue. The two pages backing it are
/// never freed (device holds the PFN for the machine's lifetime).
pub struct VirtQueue {
    pub base: usize, // MMIO base of the owning transport
    pub desc: *mut Descriptor,
    pub avail: *mut AvailRing,
    pub used: *mut UsedRing,
    pub last_used: u16,
}

unsafe impl Send for VirtQueue {}

impl VirtQueue {
    /// Legacy setup of queue `idx` on the transport at `base`.
    pub(crate) fn new(base: usize, idx: u32) -> Option<VirtQueue> {
        reg_write(base, QUEUE_SEL, idx);
        let max = reg_read(base, QUEUE_NUM_MAX);
        if max == 0 || (max as usize) < QUEUE_SIZE {
            return None;
        }
        reg_write(base, QUEUE_NUM, QUEUE_SIZE as u32);
        reg_write(base, QUEUE_ALIGN, PAGE_SIZE as u32);

        // Legacy layout in one physically-contiguous region:
        // page 0: descriptors + avail ring; page 1: used ring.
        let mem = crate::mem::page::page_alloc(2)? as usize;
        reg_write(base, QUEUE_PFN, (mem / PAGE_SIZE) as u32);

        let desc = mem as *mut Descriptor;
        let avail = (mem + QUEUE_SIZE * core::mem::size_of::<Descriptor>()) as *mut AvailRing;
        let used = (mem + PAGE_SIZE) as *mut UsedRing;
        Some(VirtQueue {
            base,
            desc,
            avail,
            used,
            last_used: 0,
        })
    }
}

/// Probe all 8 QEMU virt transports for a block device and initialize
/// it. Returns the MMIO base and its queue.
pub fn probe_block_device() -> Option<VirtQueue> {
    for slot in 0..8 {
        let base = VIRTIO_BASE + slot * 0x1000;
        if reg_read(base, MAGIC_VALUE) != MAGIC {
            continue;
        }
        let version = reg_read(base, VERSION);
        if reg_read(base, DEVICE_ID) != DEVICE_ID_BLOCK {
            continue;
        }
        if version != 1 {
            uart_println!(
                "virtio: block device at {:#x} speaks version {} (driver is legacy-only)",
                base,
                version
            );
            continue;
        }

        reg_write(base, STATUS, 0); // reset
        reg_write(base, STATUS, STATUS_ACKNOWLEDGE);
        reg_write(base, STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER);

        // Feature negotiation: accept none of the offered features —
        // the baseline block protocol is all we need.
        let _offered = reg_read(base, HOST_FEATURES);
        reg_write(base, GUEST_FEATURES, 0);
        reg_write(base, GUEST_PAGE_SIZE, PAGE_SIZE as u32);

        let queue = VirtQueue::new(base, 0)?;
        reg_write(
            base,
            STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK,
        );
        uart_println!("virtio: block device at {:#x} (legacy v1) up", base);
        return Some(queue);
    }
    None
}
