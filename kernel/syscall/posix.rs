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
    if fd >= 3 {
        return file_read(fd as usize, uva, len);
    }
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

const ENOENT: isize = -2;
const ENOTDIR: isize = -20;
const EISDIR: isize = -21;

/// LightOS accepts this openat flag (matches Linux riscv64 value).
const O_DIRECTORY: usize = 0o200000;

/// openat(dirfd, path, flags): dirfd is ignored — all paths resolve
/// from the root (processes track their own cwd in userspace for v1).
pub fn sys_openat(tf: &mut TrapFrame) -> isize {
    let (root_pa, _) = process::current_info();
    let mut buf = [0u8; 128];
    let Ok(path) = uaccess::copy_in_cstr(mmu::table_at(root_pa), tf.a1(), &mut buf) else {
        return EFAULT;
    };
    let flags = tf.a2();

    let inode = match crate::fs::lookup(path) {
        Ok(Some(inode)) => inode,
        Ok(None) => return ENOENT,
        Err(_) => return ENOENT,
    };
    if flags & O_DIRECTORY != 0 && !inode.is_dir() {
        return ENOTDIR;
    }

    let cur = crate::sched::CURRENT.load(core::sync::atomic::Ordering::Relaxed);
    let mut procs = process::PROCS.lock();
    let p = procs.slots[cur].as_mut().expect("open: no current process");
    let file = process::OpenFile {
        ino: inode.ino,
        pos: 0,
    };
    // Reuse a closed slot or append.
    let idx = match p.files.iter().position(|f| f.is_none()) {
        Some(i) => {
            p.files[i] = Some(file);
            i
        }
        None => {
            p.files.push(Some(file));
            p.files.len() - 1
        }
    };
    (idx + 3) as isize
}

pub fn sys_close(tf: &mut TrapFrame) -> isize {
    let fd = tf.a0() as isize;
    if fd < 3 {
        return 0; // console fds are never really closed
    }
    let cur = crate::sched::CURRENT.load(core::sync::atomic::Ordering::Relaxed);
    let mut procs = process::PROCS.lock();
    let p = procs.slots[cur].as_mut().expect("close: no current process");
    match p.files.get_mut(fd as usize - 3) {
        Some(slot @ Some(_)) => {
            *slot = None;
            0
        }
        _ => EBADF,
    }
}

/// Read from a file-backed fd. Returns bytes read (0 at EOF).
fn file_read(fd: usize, uva: usize, len: usize) -> isize {
    let cur = crate::sched::CURRENT.load(core::sync::atomic::Ordering::Relaxed);
    let (ino, pos) = {
        let procs = process::PROCS.lock();
        let p = procs.slots[cur].as_ref().expect("read: no current process");
        match p.files.get(fd - 3).copied().flatten() {
            Some(f) => (f.ino, f.pos),
            None => return EBADF,
        }
    };
    let Ok(inode) = crate::fs::inode(ino) else {
        return EBADF;
    };
    if inode.is_dir() {
        return EISDIR;
    }

    let (root_pa, _) = process::current_info();
    let root = mmu::table_at(root_pa);
    let mut chunk = [0u8; 512];
    let mut done = 0;
    while done < len {
        let n = chunk.len().min(len - done);
        let got = match crate::fs::read_at(&inode, pos + done, &mut chunk[..n]) {
            Ok(g) => g,
            Err(_) => return -5, // EIO
        };
        if got == 0 {
            break;
        }
        if uaccess::copy_out(root, uva + done, &chunk[..got]).is_err() {
            return EFAULT;
        }
        done += got;
        if got < n {
            break;
        }
    }

    let mut procs = process::PROCS.lock();
    let p = procs.slots[cur].as_mut().expect("read: no current process");
    if let Some(Some(f)) = p.files.get_mut(fd - 3) {
        f.pos += done;
    }
    done as isize
}

/// getdents64(fd, buf, len): fill Linux-format dirent64 records.
pub fn sys_getdents64(tf: &mut TrapFrame) -> isize {
    let fd = tf.a0();
    let uva = tf.a1();
    let len = tf.a2();
    if fd < 3 {
        return EBADF;
    }

    let cur = crate::sched::CURRENT.load(core::sync::atomic::Ordering::Relaxed);
    let (ino, start_index) = {
        let procs = process::PROCS.lock();
        let p = procs.slots[cur].as_ref().expect("getdents: no current process");
        match p.files.get(fd - 3).copied().flatten() {
            Some(f) => (f.ino, f.pos),
            None => return EBADF,
        }
    };
    let Ok(dir) = crate::fs::inode(ino) else {
        return EBADF;
    };
    if !dir.is_dir() {
        return ENOTDIR;
    }

    // Collect entries after start_index into dirent64 records.
    let mut out = [0u8; 512];
    let mut out_len = 0usize;
    let mut index = 0usize;
    let mut consumed = 0usize;
    let collect = crate::fs::readdir(&dir, |name, entry_ino| {
        if index < start_index {
            index += 1;
            return;
        }
        let reclen = (19 + name.len() + 1 + 7) & !7; // header + name + NUL, 8-aligned
        if out_len + reclen > out.len().min(len) {
            return; // buffer full; picked up next call
        }
        let entry_is_dir = crate::fs::inode(entry_ino).map(|i| i.is_dir()).unwrap_or(false);
        out[out_len..out_len + 8].copy_from_slice(&(entry_ino as u64).to_le_bytes());
        out[out_len + 8..out_len + 16].copy_from_slice(&((index + 1) as u64).to_le_bytes());
        out[out_len + 16..out_len + 18].copy_from_slice(&(reclen as u16).to_le_bytes());
        out[out_len + 18] = if entry_is_dir { 4 } else { 8 }; // DT_DIR / DT_REG
        out[out_len + 19..out_len + 19 + name.len()].copy_from_slice(name.as_bytes());
        out[out_len + 19 + name.len()] = 0;
        out_len += reclen;
        index += 1;
        consumed += 1;
    });
    if collect.is_err() {
        return -5; // EIO
    }

    if out_len > 0 {
        let (root_pa, _) = process::current_info();
        if uaccess::copy_out(mmu::table_at(root_pa), uva, &out[..out_len]).is_err() {
            return EFAULT;
        }
    }

    let mut procs = process::PROCS.lock();
    let p = procs.slots[cur].as_mut().expect("getdents: no current process");
    if let Some(Some(f)) = p.files.get_mut(fd - 3) {
        f.pos = start_index + consumed;
    }
    out_len as isize
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
