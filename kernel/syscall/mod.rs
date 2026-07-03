//! Syscall dispatch: `ecall` from U-mode lands here with the syscall
//! number in a7 (Linux riscv64 ABI numbering).
//!
//! Dispatch never returns: non-blocking calls set a0 in the trap frame
//! and resume the caller; blocking calls park the process and jump to
//! the scheduler (their result is written by whoever wakes them).

pub mod posix;

use crate::sched;
use crate::trap::context::TrapFrame;
use crate::uart_println;

pub const SYS_OPENAT: usize = 56;
pub const SYS_CLOSE: usize = 57;
pub const SYS_GETDENTS64: usize = 61;
pub const SYS_READ: usize = 63;
pub const SYS_WRITE: usize = 64;
pub const SYS_EXIT: usize = 93;
pub const SYS_SCHED_SETAFFINITY: usize = 122;
pub const SYS_SCHED_GETAFFINITY: usize = 123;
pub const SYS_GETPID: usize = 172;
pub const SYS_MUNMAP: usize = 215;
pub const SYS_CLONE: usize = 220;
pub const SYS_EXECVE: usize = 221;
pub const SYS_MMAP: usize = 222;
pub const SYS_WAIT4: usize = 260;

pub const ENOSYS: isize = -38;

/// Handle the syscall in `tf` and return to user space. Never returns.
pub fn dispatch(tf: &mut TrapFrame) -> ! {
    let nr = tf.a7();
    let ret: isize = match nr {
        SYS_WRITE => posix::sys_write(tf),
        SYS_READ => posix::sys_read(tf), // may block (never returns)
        SYS_OPENAT => posix::sys_openat(tf),
        SYS_CLOSE => posix::sys_close(tf),
        SYS_GETDENTS64 => posix::sys_getdents64(tf),
        SYS_SCHED_SETAFFINITY => posix::sys_sched_setaffinity(tf),
        SYS_SCHED_GETAFFINITY => posix::sys_sched_getaffinity(tf),
        SYS_EXIT => posix::sys_exit(tf), // never returns
        SYS_GETPID => posix::sys_getpid(),
        SYS_CLONE => posix::sys_fork(tf),
        SYS_EXECVE => posix::sys_execve(tf),
        SYS_WAIT4 => posix::sys_wait4(tf), // never returns
        SYS_MMAP => posix::sys_mmap(tf),
        SYS_MUNMAP => posix::sys_munmap(tf),
        _ => {
            uart_println!("syscall: unimplemented nr {}", nr);
            ENOSYS
        }
    };
    tf.set_a0(ret as usize);
    sched::resume_current()
}
