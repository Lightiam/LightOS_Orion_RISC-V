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
