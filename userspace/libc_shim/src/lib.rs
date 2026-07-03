//! Minimal libc surface for LightOS userspace.
//!
//! Thin `no_std` wrappers around the LightOS syscall ABI (Linux
//! riscv64 numbering, invoked via `ecall`), plus `_start`, a panic
//! handler, and `print!`/`println!` macros over fd 1.
#![no_std]

use core::fmt;

// Linux riscv64 syscall numbers (the subset LightOS implements).
pub const SYS_OPENAT: usize = 56;
pub const SYS_CLOSE: usize = 57;
pub const SYS_READ: usize = 63;
pub const SYS_WRITE: usize = 64;
pub const SYS_EXIT: usize = 93;
pub const SYS_GETPID: usize = 172;
pub const SYS_MUNMAP: usize = 215;
pub const SYS_CLONE: usize = 220; // plain fork semantics on LightOS
pub const SYS_EXECVE: usize = 221;
pub const SYS_MMAP: usize = 222;
pub const SYS_WAIT4: usize = 260;

// Program entry: the kernel starts every process here with a fresh
// stack. Calls `main() -> i32`, then exits with its return value.
core::arch::global_asm!(
    r#"
    .section .text.start
    .global _start
_start:
    call    main
    li      a7, 93          # SYS_EXIT
    ecall
1:  j       1b
"#
);

/// Raw syscall: up to 4 arguments (all LightOS calls fit).
pub fn syscall(n: usize, a0: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let ret: isize;
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a7") n,
            inlateout("a0") a0 => ret,
            in("a1") a1,
            in("a2") a2,
            in("a3") a3,
        );
    }
    ret
}

pub fn write(fd: i32, buf: &[u8]) -> isize {
    syscall(SYS_WRITE, fd as usize, buf.as_ptr() as usize, buf.len(), 0)
}

pub fn read(fd: i32, buf: &mut [u8]) -> isize {
    syscall(SYS_READ, fd as usize, buf.as_mut_ptr() as usize, buf.len(), 0)
}

pub fn open(path: &str) -> i32 {
    // openat(AT_FDCWD, path, flags=0): path must be NUL-terminated.
    let mut tmp = [0u8; 128];
    let n = path.len().min(126);
    tmp[..n].copy_from_slice(&path.as_bytes()[..n]);
    syscall(SYS_OPENAT, -100isize as usize, tmp.as_ptr() as usize, 0, 0) as i32
}

pub fn close(fd: i32) -> isize {
    syscall(SYS_CLOSE, fd as usize, 0, 0, 0)
}

pub fn exit(code: i32) -> ! {
    syscall(SYS_EXIT, code as usize, 0, 0, 0);
    unreachable!()
}

pub fn getpid() -> i32 {
    syscall(SYS_GETPID, 0, 0, 0, 0) as i32
}

/// Returns 0 in the child, the child's pid in the parent, <0 on error.
pub fn fork() -> i32 {
    syscall(SYS_CLONE, 0, 0, 0, 0) as i32
}

/// Replace the current image with the named program (NUL-terminated
/// internally). Only returns on error.
pub fn exec(path: &str) -> isize {
    let mut tmp = [0u8; 128];
    let n = path.len().min(126);
    tmp[..n].copy_from_slice(&path.as_bytes()[..n]);
    syscall(SYS_EXECVE, tmp.as_ptr() as usize, 0, 0, 0)
}

/// Wait for any child to exit; returns its pid and stores the wstatus
/// (exit code << 8, wait(2) convention).
pub fn wait(status: &mut i32) -> i32 {
    syscall(SYS_WAIT4, -1isize as usize, status as *mut i32 as usize, 0, 0) as i32
}

pub fn mmap(len: usize) -> *mut u8 {
    syscall(SYS_MMAP, 0, len, 0, 0) as *mut u8
}

pub fn munmap(ptr: *mut u8, len: usize) -> isize {
    syscall(SYS_MUNMAP, ptr as usize, len, 0, 0)
}

/// Busy-wait long enough for several 10 ms scheduler quanta to elapse,
/// so concurrent output visibly interleaves.
pub fn spin_delay(loops: usize) {
    for _ in 0..loops {
        unsafe { core::arch::asm!("nop") };
    }
}

pub struct Stdout;

impl fmt::Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write(1, s.as_bytes());
        Ok(())
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!($crate::Stdout, $($arg)*);
    }};
}

#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = writeln!($crate::Stdout, $($arg)*);
    }};
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("user panic: {}", info);
    exit(101)
}
