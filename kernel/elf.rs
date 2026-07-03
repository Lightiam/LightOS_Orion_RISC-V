//! Minimal ELF64 loader for RISC-V EXEC images.
//!
//! Loads PT_LOAD segments into a process address space with U-mode
//! permissions derived from the segment flags. Segments must be
//! 4 KiB-aligned (userspace/user.ld guarantees this for LightOS
//! binaries).
#![allow(unsafe_code)] // copies segment bytes through identity-mapped frames

use crate::mem::layout::{page_down, page_up, PAGE_SIZE};
use crate::mem::mmu::{PageTable, PTE_R, PTE_U, PTE_W, PTE_X};
use crate::mem::page;

const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const CLASS_64: u8 = 2;
const MACHINE_RISCV: u16 = 243;
const ET_EXEC: u16 = 2;
const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_W: u32 = 2;

pub struct LoadedElf {
    pub entry: usize,
    /// One past the highest mapped virtual address (program break).
    pub brk: usize,
}

fn read_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn read_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn read_u64(b: &[u8], off: usize) -> u64 {
    let mut v = [0u8; 8];
    v.copy_from_slice(&b[off..off + 8]);
    u64::from_le_bytes(v)
}

/// Map and copy every PT_LOAD segment of `image` into `root`.
pub fn load(image: &[u8], root: &mut PageTable) -> Result<LoadedElf, &'static str> {
    if image.len() < 64 || image[..4] != ELF_MAGIC {
        return Err("elf: bad magic");
    }
    if image[4] != CLASS_64 {
        return Err("elf: not ELF64");
    }
    if read_u16(image, 18) != MACHINE_RISCV {
        return Err("elf: not RISC-V");
    }
    if read_u16(image, 16) != ET_EXEC {
        return Err("elf: not an EXEC image (PIE unsupported)");
    }

    let entry = read_u64(image, 24) as usize;
    let phoff = read_u64(image, 32) as usize;
    let phentsize = read_u16(image, 54) as usize;
    let phnum = read_u16(image, 56) as usize;
    let mut brk = 0usize;

    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        if ph + 56 > image.len() {
            return Err("elf: truncated program headers");
        }
        if read_u32(image, ph) != PT_LOAD {
            continue;
        }
        let flags = read_u32(image, ph + 4);
        let offset = read_u64(image, ph + 8) as usize;
        let vaddr = read_u64(image, ph + 16) as usize;
        let filesz = read_u64(image, ph + 32) as usize;
        let memsz = read_u64(image, ph + 40) as usize;

        if filesz > memsz || offset + filesz > image.len() {
            return Err("elf: malformed segment");
        }
        if !vaddr.is_multiple_of(PAGE_SIZE) {
            return Err("elf: unaligned segment (link with -zmax-page-size=4096)");
        }

        let mut pte_flags = PTE_U | PTE_R;
        if flags & PF_W != 0 {
            pte_flags |= PTE_W;
        }
        if flags & PF_X != 0 {
            pte_flags |= PTE_X;
        }

        let start = page_down(vaddr);
        let end = page_up(vaddr + memsz);
        let mut va = start;
        while va < end {
            let frame = page::page_alloc(1).ok_or("elf: out of memory")? as usize;
            // Copy this page's slice of the file image (frames are
            // pre-zeroed, so .bss tails need no explicit clearing).
            let seg_off = va - vaddr;
            if seg_off < filesz {
                let n = core::cmp::min(PAGE_SIZE, filesz - seg_off);
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        image.as_ptr().add(offset + seg_off),
                        frame as *mut u8,
                        n,
                    );
                }
            }
            root.map(va, frame, pte_flags)?;
            va += PAGE_SIZE;
        }
        brk = core::cmp::max(brk, end);
    }

    if brk == 0 {
        return Err("elf: no loadable segments");
    }
    Ok(LoadedElf { entry, brk })
}
