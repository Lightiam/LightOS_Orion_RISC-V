//! NS16550A UART driver (MMIO) for the QEMU `virt` machine.
//!
//! Polling transmit/receive for Phase 0–1; Phase 2 enables the receive
//! interrupt (IER) and routes input through the PLIC instead of polling.
#![allow(unsafe_code)] // raw MMIO register access requires volatile pointer ops

use crate::lock::SpinLock;
use core::fmt;

/// MMIO base of UART0 on QEMU `virt`.
pub const UART_BASE: usize = 0x1000_0000;

// Register offsets (byte-wide registers, reg-shift = 0 on QEMU virt).
const RBR: usize = 0; // receive buffer (read)
const THR: usize = 0; // transmit holding (write)
const IER: usize = 1; // interrupt enable
const FCR: usize = 2; // FIFO control (write)
const LCR: usize = 3; // line control
const LSR: usize = 5; // line status

const LSR_RX_READY: u8 = 1 << 0;
const LSR_TX_IDLE: u8 = 1 << 5;

const IER_RX_ENABLE: u8 = 1 << 0;

/// Global UART instance. The lock's invariant: holders may touch UART
/// MMIO registers; it is held only for the duration of one register
/// sequence (never across a wait for *input*), so it cannot deadlock
/// against the RX path.
pub static UART: SpinLock<Uart> = SpinLock::new(Uart { base: UART_BASE });

pub struct Uart {
    base: usize,
}

impl Uart {
    fn reg(&self, offset: usize) -> *mut u8 {
        (self.base + offset) as *mut u8
    }

    fn read(&self, offset: usize) -> u8 {
        unsafe { self.reg(offset).read_volatile() }
    }

    fn write(&mut self, offset: usize, value: u8) {
        unsafe { self.reg(offset).write_volatile(value) }
    }

    /// One-time hardware init: 8N1, FIFO on, divisor for 38.4 kbaud
    /// (QEMU ignores the baud rate but real NS16550A parts will not).
    pub fn init(&mut self) {
        self.write(IER, 0x00); // all UART interrupts off until Phase 2
        self.write(LCR, 0x80); // divisor latch access
        self.write(0, 0x03); // DLL: divisor lo
        self.write(1, 0x00); // DLM: divisor hi
        self.write(LCR, 0x03); // 8 data bits, no parity, 1 stop; latch off
        self.write(FCR, 0x07); // enable + clear FIFOs
    }

    /// Enable the receive-data interrupt (Phase 2, IRQ-driven input).
    pub fn enable_rx_interrupt(&mut self) {
        self.write(IER, IER_RX_ENABLE);
    }

    /// Blocking transmit of one byte (polls LSR.THRE).
    pub fn put(&mut self, byte: u8) {
        while self.read(LSR) & LSR_TX_IDLE == 0 {
            core::hint::spin_loop();
        }
        self.write(THR, byte);
    }

    /// Non-blocking receive; `None` when the FIFO is empty.
    pub fn get(&mut self) -> Option<u8> {
        if self.read(LSR) & LSR_RX_READY != 0 {
            Some(self.read(RBR))
        } else {
            None
        }
    }
}

impl fmt::Write for Uart {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                self.put(b'\r');
            }
            self.put(byte);
        }
        Ok(())
    }
}

/// Initialize UART0. Called once by `kinit` before the first print.
pub fn init() {
    UART.lock().init();
}

const RX_BUF_SIZE: usize = 256;

/// Console input ring buffer, filled by the UART RX interrupt.
/// Lock invariant: independent of the UART register lock; the IRQ path
/// takes UART then RX_BUF, and readers take only RX_BUF, so there is
/// no ordering cycle.
static RX_BUF: SpinLock<RxRing> = SpinLock::new(RxRing {
    buf: [0; RX_BUF_SIZE],
    head: 0,
    tail: 0,
});

struct RxRing {
    buf: [u8; RX_BUF_SIZE],
    head: usize, // next write
    tail: usize, // next read
}

/// UART interrupt service: drain the RX FIFO into the ring buffer,
/// echo, then hand buffered input to any process blocked in read(0).
/// Called from the PLIC external-interrupt path.
pub fn handle_interrupt() {
    let mut got_input = false;
    {
        let mut uart = UART.lock();
        while let Some(byte) = uart.get() {
            // Echo (translate CR from terminals to NL).
            let byte = if byte == b'\r' { b'\n' } else { byte };
            if byte == b'\n' {
                uart.put(b'\r');
            }
            uart.put(byte);
            let mut rx = RX_BUF.lock();
            let next = (rx.head + 1) % RX_BUF_SIZE;
            if next != rx.tail {
                let head = rx.head;
                rx.buf[head] = byte;
                rx.head = next;
                got_input = true;
            } // else: buffer full, drop input
        }
    }
    if got_input {
        crate::sched::process::wake_console_reader();
    }
}

/// Raw console write for the sys_write path (LF -> CRLF).
pub fn write_bytes(bytes: &[u8]) {
    let mut uart = UART.lock();
    for &byte in bytes {
        if byte == b'\n' {
            uart.put(b'\r');
        }
        uart.put(byte);
    }
}

/// Pop one byte of console input, if any.
pub fn read_byte() -> Option<u8> {
    let mut rx = RX_BUF.lock();
    if rx.head == rx.tail {
        None
    } else {
        let byte = rx.buf[rx.tail];
        rx.tail = (rx.tail + 1) % RX_BUF_SIZE;
        Some(byte)
    }
}
