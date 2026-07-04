//! VirtIO network device driver (legacy MMIO, version 1).
//!
//! Two virtqueues: RX (0) is pre-filled with device-writable buffers;
//! TX (1) carries one frame at a time. Every buffer is prefixed with a
//! 10-byte `virtio_net_hdr` (legacy, no mergeable RX buffers). RX is
//! polled — wiring the device's PLIC interrupt for async receive is a
//! follow-up; a poll loop is enough to bring the stack up.
#![allow(unsafe_code)] // DMA rings and MMIO

use super::{
    reg_read, reg_write, Descriptor, VirtQueue, CONFIG, DESC_F_WRITE, DEVICE_ID_NET, DEVICE_ID_OFF,
    GUEST_FEATURES, GUEST_PAGE_SIZE, HOST_FEATURES, INTERRUPT_ACK, INTERRUPT_STATUS, MAGIC,
    MAGIC_VALUE_OFF, QUEUE_NOTIFY, QUEUE_SIZE, STATUS, STATUS_ACKNOWLEDGE, STATUS_DRIVER,
    STATUS_DRIVER_OK, VERSION_OFF,
};
use crate::lock::SpinLock;
use crate::mem::layout::{PAGE_SIZE, VIRTIO_BASE};
use crate::mem::page;
use crate::uart_println;

/// Legacy `virtio_net_hdr` size (no VIRTIO_NET_F_MRG_RXBUF).
const NET_HDR_LEN: usize = 10;
/// Per-buffer size: header + a full 1514-byte Ethernet frame, rounded.
const BUF_SIZE: usize = 2048;
/// VIRTIO_NET_F_MAC — device provides a MAC in config space.
const F_MAC: u32 = 1 << 5;

const RXQ: u32 = 0;
const TXQ: u32 = 1;

struct NetDev {
    base: usize,
    rx: VirtQueue,
    tx: VirtQueue,
    mac: [u8; 6],
    /// Physical address of each RX buffer (indexed by descriptor id).
    rx_bufs: [usize; QUEUE_SIZE],
    tx_buf: usize,
}

unsafe impl Send for NetDev {}

/// The (single) network device. Lock invariant: serializes queue
/// manipulation and the shared DMA buffers.
static NET: SpinLock<Option<NetDev>> = SpinLock::new(None);

fn probe() -> Option<usize> {
    for slot in 0..8 {
        let base = VIRTIO_BASE + slot * 0x1000;
        if reg_read(base, MAGIC_VALUE_OFF) != MAGIC {
            continue;
        }
        if reg_read(base, VERSION_OFF) != 1 {
            continue;
        }
        if reg_read(base, DEVICE_ID_OFF) == DEVICE_ID_NET {
            return Some(base);
        }
    }
    None
}

/// Probe and initialize the first virtio-net device. Returns true on
/// success. Idempotent-ish: only the first call does anything useful.
pub fn init() -> bool {
    let Some(base) = probe() else {
        uart_println!("virtio-net: no network device found");
        return false;
    };

    reg_write(base, STATUS, 0); // reset
    reg_write(base, STATUS, STATUS_ACKNOWLEDGE);
    reg_write(base, STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER);

    // Accept VIRTIO_NET_F_MAC (if offered) so the config MAC is valid;
    // everything else stays off (no checksum offload, no MRG_RXBUF).
    let offered = reg_read(base, HOST_FEATURES);
    reg_write(base, GUEST_FEATURES, offered & F_MAC);
    reg_write(base, GUEST_PAGE_SIZE, PAGE_SIZE as u32);

    // MAC lives at config offset 0 as six individual bytes — read them
    // with byte-wide loads (a u32 read at CONFIG+1 would be unaligned).
    let mut mac = [0u8; 6];
    for (i, b) in mac.iter_mut().enumerate() {
        *b = unsafe { ((base + CONFIG + i) as *const u8).read_volatile() };
    }

    let Some(rx) = VirtQueue::new(base, RXQ) else {
        uart_println!("virtio-net: RX queue setup failed");
        return false;
    };
    let Some(tx) = VirtQueue::new(base, TXQ) else {
        uart_println!("virtio-net: TX queue setup failed");
        return false;
    };

    // RX buffers: 8 * 2048 = 16 KiB = 4 pages. TX: one page.
    let rx_mem = match page::page_alloc(QUEUE_SIZE * BUF_SIZE / PAGE_SIZE) {
        Some(p) => p as usize,
        None => return false,
    };
    let tx_buf = match page::page_alloc(1) {
        Some(p) => p as usize,
        None => return false,
    };

    let mut rx_bufs = [0usize; QUEUE_SIZE];
    for (i, slot) in rx_bufs.iter_mut().enumerate() {
        *slot = rx_mem + i * BUF_SIZE;
    }

    let dev = NetDev {
        base,
        rx,
        tx,
        mac,
        rx_bufs,
        tx_buf,
    };

    // Post every RX buffer to the device (device-writable).
    unsafe {
        for i in 0..QUEUE_SIZE {
            *dev.rx.desc.add(i) = Descriptor {
                addr: dev.rx_bufs[i] as u64,
                len: BUF_SIZE as u32,
                flags: DESC_F_WRITE,
                next: 0,
            };
            let avail = &mut *dev.rx.avail;
            avail.ring[i] = i as u16;
        }
        let avail = &mut *dev.rx.avail;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        avail.idx = QUEUE_SIZE as u16;
    }

    reg_write(
        base,
        STATUS,
        STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK,
    );
    reg_write(base, QUEUE_NOTIFY, RXQ); // let the device know RX is armed

    uart_println!(
        "virtio-net: {:#x} up, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        base,
        mac[0],
        mac[1],
        mac[2],
        mac[3],
        mac[4],
        mac[5]
    );
    *NET.lock() = Some(dev);
    true
}

/// True once a device is initialized.
pub fn is_present() -> bool {
    NET.lock().is_some()
}

/// This interface's MAC address.
pub fn mac() -> Option<[u8; 6]> {
    NET.lock().as_ref().map(|d| d.mac)
}

/// Transmit one Ethernet frame (a `virtio_net_hdr` is prepended).
pub fn send(frame: &[u8]) -> Result<(), &'static str> {
    if frame.len() > BUF_SIZE - NET_HDR_LEN {
        return Err("virtio-net: frame too large");
    }
    let mut guard = NET.lock();
    let dev = guard.as_mut().ok_or("virtio-net: not initialized")?;

    unsafe {
        // Zeroed header, then the frame.
        core::ptr::write_bytes(dev.tx_buf as *mut u8, 0, NET_HDR_LEN);
        core::ptr::copy_nonoverlapping(
            frame.as_ptr(),
            (dev.tx_buf + NET_HDR_LEN) as *mut u8,
            frame.len(),
        );
        *dev.tx.desc.add(0) = Descriptor {
            addr: dev.tx_buf as u64,
            len: (NET_HDR_LEN + frame.len()) as u32,
            flags: 0, // device reads
            next: 0,
        };
        let avail = &mut *dev.tx.avail;
        avail.ring[(avail.idx as usize) % QUEUE_SIZE] = 0;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        avail.idx = avail.idx.wrapping_add(1);
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        reg_write(dev.base, QUEUE_NOTIFY, TXQ);

        // Wait for the device to consume the TX descriptor.
        let used = &*dev.tx.used;
        let mut spins = 0u64;
        while core::ptr::addr_of!(used.idx).read_volatile() == dev.tx.last_used {
            core::hint::spin_loop();
            spins += 1;
            if spins > 100_000_000 {
                return Err("virtio-net: TX timed out");
            }
        }
        dev.tx.last_used = dev.tx.last_used.wrapping_add(1);
        ack_irq(dev.base);
    }
    Ok(())
}

/// Drain the RX ring, invoking `f` on each received Ethernet frame
/// (the `virtio_net_hdr` is stripped). Returns the number of frames
/// delivered. Non-blocking.
pub fn poll_rx(mut f: impl FnMut(&[u8])) -> usize {
    let mut guard = NET.lock();
    let Some(dev) = guard.as_mut() else {
        return 0;
    };
    let mut count = 0;
    unsafe {
        loop {
            let used = &*dev.rx.used;
            if core::ptr::addr_of!(used.idx).read_volatile() == dev.rx.last_used {
                break;
            }
            let slot = (dev.rx.last_used as usize) % QUEUE_SIZE;
            let elem = used.ring[slot];
            let id = elem.id as usize;
            let total = elem.len as usize;

            if total > NET_HDR_LEN && id < QUEUE_SIZE {
                let frame_ptr = (dev.rx_bufs[id] + NET_HDR_LEN) as *const u8;
                let frame_len = total - NET_HDR_LEN;
                let frame = core::slice::from_raw_parts(frame_ptr, frame_len);
                f(frame);
                count += 1;
            }

            dev.rx.last_used = dev.rx.last_used.wrapping_add(1);

            // Re-post the buffer for the device to fill again.
            let avail = &mut *dev.rx.avail;
            let ai = (avail.idx as usize) % QUEUE_SIZE;
            avail.ring[ai] = id as u16;
            core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
            avail.idx = avail.idx.wrapping_add(1);
        }
        if count > 0 {
            reg_write(dev.base, QUEUE_NOTIFY, RXQ);
            ack_irq(dev.base);
        }
    }
    count
}

fn ack_irq(base: usize) {
    let isr = reg_read(base, INTERRUPT_STATUS);
    if isr != 0 {
        reg_write(base, INTERRUPT_ACK, isr);
    }
}
