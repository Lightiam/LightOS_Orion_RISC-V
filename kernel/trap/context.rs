//! Trap context: the register state captured on every trap entry.

/// Full CPU context saved by the trap trampoline (`vector.S`).
///
/// Layout contract with the assembly: `regs[i]` is register `x{i}` at
/// byte offset `i * 8`; `sepc` at 256; `sstatus` at 264. Total 272
/// bytes, 16-byte aligned. Do not reorder fields.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TrapFrame {
    /// x0..x31. x0 slot is never written (hardwired zero); x2 slot
    /// holds the interrupted stack pointer.
    pub regs: [usize; 32],
    pub sepc: usize,
    pub sstatus: usize,
}

impl TrapFrame {
    pub const fn zeroed() -> Self {
        Self {
            regs: [0; 32],
            sepc: 0,
            sstatus: 0,
        }
    }
}

// Named accessors for the registers the kernel actually inspects.
impl TrapFrame {
    pub fn sp(&self) -> usize {
        self.regs[2]
    }
    pub fn a0(&self) -> usize {
        self.regs[10]
    }
    pub fn set_a0(&mut self, v: usize) {
        self.regs[10] = v;
    }
    pub fn a1(&self) -> usize {
        self.regs[11]
    }
    pub fn a2(&self) -> usize {
        self.regs[12]
    }
    pub fn a3(&self) -> usize {
        self.regs[13]
    }
    pub fn a7(&self) -> usize {
        self.regs[17]
    }
}
