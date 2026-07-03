//! Sv39 three-level page tables: map / unmap / walk, plus construction
//! and activation of the kernel's identity-mapped address space.
#![allow(unsafe_code)] // page-table walks and satp writes are raw memory ops

use super::layout::{self, page_down, page_up, PAGE_SIZE};
use super::page;

// PTE permission/state bits.
pub const PTE_V: u64 = 1 << 0;
pub const PTE_R: u64 = 1 << 1;
pub const PTE_W: u64 = 1 << 2;
pub const PTE_X: u64 = 1 << 3;
pub const PTE_U: u64 = 1 << 4;
pub const PTE_G: u64 = 1 << 5;
pub const PTE_A: u64 = 1 << 6;
pub const PTE_D: u64 = 1 << 7;

const SATP_MODE_SV39: usize = 8 << 60;

/// Root-table slot for user mappings: VAs 0x4000_0000..0x8000_0000
/// (1 GiB). Kernel RAM sits in slot 2, MMIO in slot 0, so every
/// process root shares the kernel's slot-0/slot-2 subtrees and owns
/// slot 1 exclusively.
pub const USER_ROOT_SLOT: usize = 1;

use core::sync::atomic::{AtomicUsize, Ordering};

/// Physical address of the kernel's root table (set once at boot).
static KERNEL_ROOT_PA: AtomicUsize = AtomicUsize::new(0);

/// One page table: 512 eight-byte entries, exactly one 4 KiB page.
#[repr(C, align(4096))]
pub struct PageTable {
    entries: [u64; 512],
}

impl PageTable {
    /// Allocate a zeroed table from the page-frame allocator.
    pub fn alloc() -> Option<&'static mut PageTable> {
        let ptr = page::page_alloc(1)? as *mut PageTable;
        Some(unsafe { &mut *ptr })
    }

    fn pa(&self) -> usize {
        self as *const _ as usize
    }

    /// satp value that activates this table as the root.
    pub fn satp(&self) -> usize {
        SATP_MODE_SV39 | (self.pa() >> 12)
    }

    /// VPN slice for `level` (2 = root) of a Sv39 virtual address.
    fn vpn(va: usize, level: usize) -> usize {
        (va >> (12 + 9 * level)) & 0x1ff
    }

    /// Walk to the leaf PTE for `va`, allocating intermediate tables
    /// when `create` is set. Returns a raw PTE pointer (level 0).
    fn walk(&mut self, va: usize, create: bool) -> Option<*mut u64> {
        let mut table = self;
        for level in (1..=2).rev() {
            let pte = &mut table.entries[Self::vpn(va, level)];
            if *pte & PTE_V == 0 {
                if !create {
                    return None;
                }
                let next = PageTable::alloc()?;
                *pte = ((next.pa() as u64) >> 12 << 10) | PTE_V;
            } else if *pte & (PTE_R | PTE_W | PTE_X) != 0 {
                // Superpage leaf in the middle of the walk — LightOS v1
                // maps 4 KiB pages only, so treat this as a bug.
                panic!("mmu: unexpected superpage at va {:#x}", va);
            }
            let next_pa = ((*pte >> 10) << 12) as usize;
            table = unsafe { &mut *(next_pa as *mut PageTable) };
        }
        Some(&mut table.entries[Self::vpn(va, 0)] as *mut u64)
    }

    /// Map the 4 KiB page at `va` to physical `pa` with `flags`
    /// (PTE_R/W/X/U). A and D are set eagerly so implementations that
    /// trap on hardware A/D updates never fault on kernel mappings.
    pub fn map(&mut self, va: usize, pa: usize, flags: u64) -> Result<(), &'static str> {
        debug_assert_eq!(va % PAGE_SIZE, 0, "map: unaligned va");
        debug_assert_eq!(pa % PAGE_SIZE, 0, "map: unaligned pa");
        let pte = self
            .walk(va, true)
            .ok_or("mmu: out of memory for page table")?;
        unsafe {
            if *pte & PTE_V != 0 {
                return Err("mmu: va already mapped");
            }
            *pte = ((pa as u64) >> 12 << 10) | flags | PTE_A | PTE_D | PTE_V;
        }
        Ok(())
    }

    /// Identity-map every page in `[start, end)`.
    pub fn map_range(&mut self, start: usize, end: usize, flags: u64) -> Result<(), &'static str> {
        let mut va = page_down(start);
        while va < page_up(end) {
            self.map(va, va, flags)?;
            va += PAGE_SIZE;
        }
        Ok(())
    }

    /// Remove the mapping for `va`. Returns the physical address that
    /// was mapped, so callers can free the frame if they own it.
    pub fn unmap(&mut self, va: usize) -> Option<usize> {
        let pte = self.walk(va, false)?;
        unsafe {
            if *pte & PTE_V == 0 {
                return None;
            }
            let pa = ((*pte >> 10) << 12) as usize;
            *pte = 0;
            Some(pa)
        }
    }

    /// Software walk: translate `va` to its physical address.
    pub fn virt_to_phys(&mut self, va: usize) -> Option<usize> {
        let pte = self.walk(va, false)?;
        unsafe {
            if *pte & PTE_V == 0 {
                return None;
            }
            let pa = ((*pte >> 10) << 12) as usize;
            Some(pa + (va & (PAGE_SIZE - 1)))
        }
    }
}

/// Build the kernel address space: identity map with per-section
/// permissions (text RX, rodata R, data/stacks/RAM RW) plus MMIO.
pub fn kernel_page_table() -> &'static mut PageTable {
    let root = PageTable::alloc().expect("mmu: no memory for kernel root table");

    root.map_range(layout::text_start(), layout::text_end(), PTE_R | PTE_X)
        .expect("map kernel text");
    root.map_range(layout::rodata_start(), layout::rodata_end(), PTE_R)
        .expect("map kernel rodata");
    // data + bss + boot stacks + all allocatable RAM: read/write.
    root.map_range(layout::data_start(), layout::RAM_END, PTE_R | PTE_W)
        .expect("map kernel data/heap");

    // MMIO for the devices the kernel drives.
    root.map_range(
        layout::UART0_BASE,
        layout::UART0_BASE + layout::UART0_SIZE,
        PTE_R | PTE_W,
    )
    .expect("map UART0");
    root.map_range(
        layout::VIRTIO_BASE,
        layout::VIRTIO_BASE + layout::VIRTIO_SIZE,
        PTE_R | PTE_W,
    )
    .expect("map VirtIO");
    root.map_range(
        layout::CLINT_BASE,
        layout::CLINT_BASE + layout::CLINT_SIZE,
        PTE_R | PTE_W,
    )
    .expect("map CLINT");
    root.map_range(
        layout::PLIC_BASE,
        layout::PLIC_BASE + layout::PLIC_SIZE,
        PTE_R | PTE_W,
    )
    .expect("map PLIC");

    root
}

/// Reconstitute a `&mut PageTable` from a physical address (valid
/// because all of RAM is identity-mapped for the kernel).
pub fn table_at(pa: usize) -> &'static mut PageTable {
    unsafe { &mut *(pa as *mut PageTable) }
}

/// Physical address of the kernel root table.
pub fn kernel_root_pa() -> usize {
    KERNEL_ROOT_PA.load(Ordering::Relaxed)
}

/// Create a fresh process root: kernel subtrees shared by reference
/// (no PTE_U, so user code cannot touch them), user slot empty.
pub fn new_user_root() -> Option<&'static mut PageTable> {
    let root = PageTable::alloc()?;
    let kernel = table_at(kernel_root_pa());
    for i in 0..512 {
        if i != USER_ROOT_SLOT {
            root.entries[i] = kernel.entries[i];
        }
    }
    Some(root)
}

/// Free every frame and table under the user slot of `root`. Shared
/// kernel subtrees are left untouched.
pub fn free_user_space(root: &mut PageTable) {
    let root_entry = root.entries[USER_ROOT_SLOT];
    if root_entry & PTE_V == 0 {
        return;
    }
    let l1 = table_at(((root_entry >> 10) << 12) as usize);
    for e1 in l1.entries.iter() {
        if e1 & PTE_V == 0 {
            continue;
        }
        let l0 = table_at(((e1 >> 10) << 12) as usize);
        for e0 in l0.entries.iter() {
            if e0 & PTE_V != 0 {
                page::page_free(((e0 >> 10) << 12) as *mut u8, 1);
            }
        }
        page::page_free(l0 as *mut PageTable as *mut u8, 1);
    }
    page::page_free(l1 as *mut PageTable as *mut u8, 1);
    root.entries[USER_ROOT_SLOT] = 0;
}

/// Deep-copy the user slot of `src` into `dst` (fork). Every mapped
/// page is duplicated frame-by-frame with identical permissions.
pub fn clone_user_space(src: &mut PageTable, dst: &mut PageTable) -> Result<(), &'static str> {
    let root_entry = src.entries[USER_ROOT_SLOT];
    if root_entry & PTE_V == 0 {
        return Ok(());
    }
    let l1 = table_at(((root_entry >> 10) << 12) as usize);
    for (i1, e1) in l1.entries.iter().enumerate() {
        if e1 & PTE_V == 0 {
            continue;
        }
        let l0 = table_at(((e1 >> 10) << 12) as usize);
        for (i0, e0) in l0.entries.iter().enumerate() {
            if e0 & PTE_V == 0 {
                continue;
            }
            let va = (USER_ROOT_SLOT << 30) | (i1 << 21) | (i0 << 12);
            let src_pa = ((e0 >> 10) << 12) as usize;
            let flags = e0 & (PTE_R | PTE_W | PTE_X | PTE_U);
            let frame = page::page_alloc(1).ok_or("fork: out of memory")? as usize;
            unsafe {
                core::ptr::copy_nonoverlapping(src_pa as *const u8, frame as *mut u8, PAGE_SIZE);
            }
            dst.map(va, frame, flags)?;
        }
    }
    Ok(())
}

/// Point satp at `root` and flush the TLB. Caller must ensure the
/// currently-executing code is identity-mapped in `root`.
pub fn enable(root: &PageTable) {
    KERNEL_ROOT_PA.store(root.pa(), Ordering::Relaxed);
    unsafe {
        core::arch::asm!(
            "sfence.vma zero, zero",
            "csrw satp, {satp}",
            "sfence.vma zero, zero",
            satp = in(reg) root.satp(),
        );
    }
}
