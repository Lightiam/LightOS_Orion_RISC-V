//! POSIX-flavoured syscall implementations.

use crate::mem::layout::PAGE_SIZE;
use crate::mem::mmu::{self, PTE_R, PTE_U, PTE_W};
use crate::mem::{page, uaccess};
use crate::sched::process;
use crate::trap::context::TrapFrame;
use crate::uart;

const EBADF: isize = -9;
const ENOMEM: isize = -12;
const EFAULT: isize = -14;
const EINVAL: isize = -22;

/// write(fd, buf, len): fd 1/2 go to the UART console. File-backed
/// descriptors arrive with the VFS in Phase 5.
pub fn sys_write(tf: &mut TrapFrame) -> isize {
    let fd = tf.a0() as isize;
    let uva = tf.a1();
    let len = tf.a2();
    match fd {
        1 | 2 => {
            let (root_pa, _) = process::current_info();
            let root = mmu::table_at(root_pa);
            let mut chunk = [0u8; 256];
            let mut done = 0;
            while done < len {
                let n = core::cmp::min(chunk.len(), len - done);
                if uaccess::copy_in(root, uva + done, &mut chunk[..n]).is_err() {
                    return EFAULT;
                }
                uart::write_bytes(&chunk[..n]);
                done += n;
            }
            len as isize
        }
        _ => EBADF,
    }
}

/// read(fd, buf, len): fd 0 reads the console; returns as soon as at
/// least one byte is available, blocking the caller otherwise.
/// The blocking path never returns from this function.
pub fn sys_read(tf: &mut TrapFrame) -> isize {
    let fd = tf.a0() as isize;
    let uva = tf.a1();
    let len = tf.a2();
    if fd != 0 {
        return EBADF;
    }
    if len == 0 {
        return 0;
    }

    let mut tmp = [0u8; 128];
    let mut n = 0;
    while n < len.min(tmp.len()) {
        match uart::read_byte() {
            Some(b) => {
                tmp[n] = b;
                n += 1;
            }
            None => break,
        }
    }

    if n == 0 {
        // Nothing buffered: park until the UART interrupt delivers.
        process::block_on_console(uva, len);
    }

    let (root_pa, _) = process::current_info();
    if uaccess::copy_out(mmu::table_at(root_pa), uva, &tmp[..n]).is_err() {
        return EFAULT;
    }
    n as isize
}

/// openat/close: no VFS until Phase 5.
pub fn sys_openat(_tf: &mut TrapFrame) -> isize {
    super::ENOSYS
}

pub fn sys_close(_tf: &mut TrapFrame) -> isize {
    super::ENOSYS
}

pub fn sys_exit(tf: &mut TrapFrame) -> isize {
    process::exit_current(tf.a0() as i32)
}

pub fn sys_getpid() -> isize {
    let (_, pid) = process::current_info();
    pid as isize
}

pub fn sys_fork(tf: &mut TrapFrame) -> isize {
    process::fork_current(tf)
}

pub fn sys_execve(tf: &mut TrapFrame) -> isize {
    let (root_pa, _) = process::current_info();
    let mut buf = [0u8; 128];
    match uaccess::copy_in_cstr(mmu::table_at(root_pa), tf.a0(), &mut buf) {
        Ok(name) => process::exec_current(tf, name),
        Err(_) => EFAULT,
    }
}

pub fn sys_wait4(tf: &mut TrapFrame) -> isize {
    // a0 = pid (only -1 "any child" is supported), a1 = *wstatus.
    process::wait_current(tf.a1())
}

/// mmap(addr=0, len, ...): anonymous RW mappings only, kernel-chosen
/// address (bump region per process).
pub fn sys_mmap(tf: &mut TrapFrame) -> isize {
    let len = tf.a1();
    if len == 0 {
        return EINVAL;
    }
    let pages = len.div_ceil(PAGE_SIZE);
    let (root_pa, _) = process::current_info();

    let va = {
        let cur = crate::sched::CURRENT.load(core::sync::atomic::Ordering::Relaxed);
        let mut procs = process::PROCS.lock();
        let p = procs.slots[cur].as_mut().expect("mmap: no current process");
        let va = p.next_mmap;
        p.next_mmap += pages * PAGE_SIZE;
        va
    };

    let root = mmu::table_at(root_pa);
    for i in 0..pages {
        let Some(frame) = page::page_alloc(1) else {
            return ENOMEM;
        };
        if root.map(va + i * PAGE_SIZE, frame as usize, PTE_U | PTE_R | PTE_W).is_err() {
            return ENOMEM;
        }
    }
    va as isize
}

/// munmap(addr, len): unmap and free anonymous pages.
pub fn sys_munmap(tf: &mut TrapFrame) -> isize {
    let va = tf.a0();
    let len = tf.a1();
    if va % PAGE_SIZE != 0 || len == 0 {
        return EINVAL;
    }
    let (root_pa, _) = process::current_info();
    let root = mmu::table_at(root_pa);
    for i in 0..len.div_ceil(PAGE_SIZE) {
        if let Some(pa) = root.unmap(va + i * PAGE_SIZE) {
            page::page_free(pa as *mut u8, 1);
        }
    }
    0
}
