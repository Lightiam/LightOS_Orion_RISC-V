//! Safe(ish) copies between kernel space and a process address space.
//!
//! Every transfer walks the process page table page-by-page and
//! rejects addresses outside the user VA window, so a hostile pointer
//! can never reach kernel memory.
#![allow(unsafe_code)] // copies through identity-mapped physical frames

use super::layout::PAGE_SIZE;
use super::mmu::PageTable;

/// User virtual address window (root slot 1).
pub const USER_VA_START: usize = 0x4000_0000;
pub const USER_VA_END: usize = 0x8000_0000;

fn check_range(uva: usize, len: usize) -> Result<(), &'static str> {
    let end = uva.checked_add(len).ok_or("uaccess: address overflow")?;
    if uva < USER_VA_START || end > USER_VA_END {
        return Err("uaccess: address outside user window");
    }
    Ok(())
}

/// Copy `src` into the process at `uva`.
pub fn copy_out(root: &mut PageTable, uva: usize, src: &[u8]) -> Result<(), &'static str> {
    check_range(uva, src.len())?;
    let mut done = 0;
    while done < src.len() {
        let va = uva + done;
        let pa = root
            .virt_to_phys(va)
            .ok_or("uaccess: unmapped user address")?;
        let n = core::cmp::min(PAGE_SIZE - (va % PAGE_SIZE), src.len() - done);
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr().add(done), pa as *mut u8, n);
        }
        done += n;
    }
    Ok(())
}

/// Copy `dst.len()` bytes from the process at `uva` into `dst`.
pub fn copy_in(root: &mut PageTable, uva: usize, dst: &mut [u8]) -> Result<(), &'static str> {
    check_range(uva, dst.len())?;
    let mut done = 0;
    while done < dst.len() {
        let va = uva + done;
        let pa = root
            .virt_to_phys(va)
            .ok_or("uaccess: unmapped user address")?;
        let n = core::cmp::min(PAGE_SIZE - (va % PAGE_SIZE), dst.len() - done);
        unsafe {
            core::ptr::copy_nonoverlapping(pa as *const u8, dst.as_mut_ptr().add(done), n);
        }
        done += n;
    }
    Ok(())
}

/// Copy a NUL-terminated string (max `buf.len() - 1` bytes) from the
/// process; returns it as `&str`.
pub fn copy_in_cstr<'a>(
    root: &mut PageTable,
    uva: usize,
    buf: &'a mut [u8],
) -> Result<&'a str, &'static str> {
    for i in 0..buf.len() - 1 {
        let mut byte = [0u8; 1];
        copy_in(root, uva + i, &mut byte)?;
        if byte[0] == 0 {
            return core::str::from_utf8(&buf[..i]).map_err(|_| "uaccess: invalid UTF-8");
        }
        buf[i] = byte[0];
    }
    Err("uaccess: string too long")
}
