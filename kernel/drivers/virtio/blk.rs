//! VirtIO block device driver.
//!
//! Synchronous request path: build a 3-descriptor chain (header, data,
//! status), kick the queue, poll the used ring. Filesystem I/O in
//! LightOS v1 is synchronous by design (single scheduling hart, block
//! reads complete in microseconds on virtio), so polled completion is
//! simpler and race-free; the interrupt line is acknowledged to keep
//! the device state machine clean.
#![allow(unsafe_code)] // DMA descriptors and MMIO

use super::{
    reg_read, reg_write, VirtQueue, DESC_F_NEXT, DESC_F_WRITE, CONFIG, INTERRUPT_ACK,
    INTERRUPT_STATUS, QUEUE_NOTIFY,
};
use crate::lock::SpinLock;
use crate::uart_println;

pub const SECTOR_SIZE: usize = 512;

const REQ_TYPE_IN: u32 = 0; // device -> memory (read)
const REQ_TYPE_OUT: u32 = 1; // memory -> device (write)

#[repr(C)]
struct BlkReqHeader {
    req_type: u32,
    reserved: u32,
    sector: u64,
}

struct BlkDev {
    queue: VirtQueue,
    header: BlkReqHeader,
    status: u8,
    capacity_sectors: u64,
}

/// The (single) block device. Lock invariant: serializes the whole
/// request/poll cycle; DMA buffers inside are only touched while held.
static BLK: SpinLock<Option<BlkDev>> = SpinLock::new(None);

/// Probe and initialize the first virtio-blk transport.
pub fn init() -> bool {
    let Some(queue) = super::probe_block_device() else {
        uart_println!("virtio-blk: no block device found");
        return false;
    };
    let base = queue.base;
    let cap_lo = reg_read(base, CONFIG) as u64;
    let cap_hi = reg_read(base, CONFIG + 4) as u64;
    let capacity_sectors = (cap_hi << 32) | cap_lo;
    uart_println!(
        "virtio-blk: capacity {} sectors ({} KiB)",
        capacity_sectors,
        capacity_sectors * SECTOR_SIZE as u64 / 1024
    );
    *BLK.lock() = Some(BlkDev {
        queue,
        header: BlkReqHeader {
            req_type: 0,
            reserved: 0,
            sector: 0,
        },
        status: 0xff,
        capacity_sectors,
    });
    true
}

pub fn is_present() -> bool {
    BLK.lock().is_some()
}

fn transfer(sector: u64, buf: &mut [u8], write: bool) -> Result<(), &'static str> {
    debug_assert_eq!(buf.len() % SECTOR_SIZE, 0);
    let mut guard = BLK.lock();
    let dev = guard.as_mut().ok_or("virtio-blk: not initialized")?;

    let sectors = (buf.len() / SECTOR_SIZE) as u64;
    if sector + sectors > dev.capacity_sectors {
        return Err("virtio-blk: transfer beyond device capacity");
    }

    dev.header = BlkReqHeader {
        req_type: if write { REQ_TYPE_OUT } else { REQ_TYPE_IN },
        reserved: 0,
        sector,
    };
    dev.status = 0xff;

    unsafe {
        let q = &mut dev.queue;
        // Descriptor chain: 0 header (device reads), 1 data, 2 status
        // (device writes).
        *q.desc.add(0) = super::Descriptor {
            addr: &dev.header as *const _ as u64,
            len: core::mem::size_of::<BlkReqHeader>() as u32,
            flags: DESC_F_NEXT,
            next: 1,
        };
        *q.desc.add(1) = super::Descriptor {
            addr: buf.as_mut_ptr() as u64,
            len: buf.len() as u32,
            flags: DESC_F_NEXT | if write { 0 } else { DESC_F_WRITE },
            next: 2,
        };
        *q.desc.add(2) = super::Descriptor {
            addr: &dev.status as *const _ as u64,
            len: 1,
            flags: DESC_F_WRITE,
            next: 0,
        };

        let avail = &mut *q.avail;
        avail.ring[(avail.idx as usize) % super::QUEUE_SIZE] = 0;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        avail.idx = avail.idx.wrapping_add(1);
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        reg_write(q.base, QUEUE_NOTIFY, 0);

        // Poll completion (bounded; QEMU completes in microseconds).
        let used = &*q.used;
        let mut spins = 0u64;
        while core::ptr::addr_of!(used.idx).read_volatile() == q.last_used {
            core::hint::spin_loop();
            spins += 1;
            if spins > 1_000_000_000 {
                return Err("virtio-blk: request timed out");
            }
        }
        q.last_used = q.last_used.wrapping_add(1);

        // Acknowledge the interrupt the device raised for completion.
        let isr = reg_read(q.base, INTERRUPT_STATUS);
        if isr != 0 {
            reg_write(q.base, INTERRUPT_ACK, isr);
        }
    }

    if dev.status == 0 {
        Ok(())
    } else {
        Err("virtio-blk: device reported I/O error")
    }
}

/// Read `buf.len()` bytes starting at byte offset `sector * 512`.
pub fn read_sectors(sector: u64, buf: &mut [u8]) -> Result<(), &'static str> {
    transfer(sector, buf, false)
}

/// Write `buf` starting at byte offset `sector * 512`.
pub fn write_sectors(sector: u64, buf: &mut [u8]) -> Result<(), &'static str> {
    transfer(sector, buf, true)
}
