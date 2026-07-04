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
pub const SYS_GETDENTS64: usize = 61;
pub const SYS_READ: usize = 63;
pub const SYS_WRITE: usize = 64;
pub const SYS_EXIT: usize = 93;
pub const SYS_SCHED_SETAFFINITY: usize = 122;
pub const SYS_SCHED_GETAFFINITY: usize = 123;
pub const SYS_REBOOT: usize = 142;
pub const SYS_GETPID: usize = 172;
pub const SYS_SYSINFO: usize = 179;
pub const SYS_SOCKET: usize = 198;
pub const SYS_BIND: usize = 200;
pub const SYS_CONNECT: usize = 203;
pub const SYS_SENDTO: usize = 206;
pub const SYS_RECVFROM: usize = 207;
pub const SYS_MUNMAP: usize = 215;
pub const SYS_CLONE: usize = 220; // plain fork semantics on LightOS
pub const SYS_EXECVE: usize = 221;
pub const SYS_MMAP: usize = 222;
pub const SYS_WAIT4: usize = 260;
pub const SYS_PROCLIST: usize = 500; // LightOS-specific: `ps` listing

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

/// Raw syscall with 6 arguments (for sendto/recvfrom).
#[allow(clippy::too_many_arguments)]
pub fn syscall5(
    n: usize,
    a0: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
) -> isize {
    let ret: isize;
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a7") n,
            inlateout("a0") a0 => ret,
            in("a1") a1,
            in("a2") a2,
            in("a3") a3,
            in("a4") a4,
            in("a5") a5,
        );
    }
    ret
}

pub fn write(fd: i32, buf: &[u8]) -> isize {
    syscall(SYS_WRITE, fd as usize, buf.as_ptr() as usize, buf.len(), 0)
}

pub fn read(fd: i32, buf: &mut [u8]) -> isize {
    syscall(
        SYS_READ,
        fd as usize,
        buf.as_mut_ptr() as usize,
        buf.len(),
        0,
    )
}

/// openat O_DIRECTORY: fail unless the path is a directory.
pub const O_DIRECTORY: usize = 0o200000;

pub fn open(path: &str) -> i32 {
    open_flags(path, 0)
}

pub fn open_flags(path: &str, flags: usize) -> i32 {
    // openat(AT_FDCWD, path, flags): path must be NUL-terminated.
    let mut tmp = [0u8; 128];
    let n = path.len().min(126);
    tmp[..n].copy_from_slice(&path.as_bytes()[..n]);
    syscall(
        SYS_OPENAT,
        -100isize as usize,
        tmp.as_ptr() as usize,
        flags,
        0,
    ) as i32
}

/// Fill `buf` with linux_dirent64 records; returns bytes or 0 at end.
pub fn getdents64(fd: i32, buf: &mut [u8]) -> isize {
    syscall(
        SYS_GETDENTS64,
        fd as usize,
        buf.as_mut_ptr() as usize,
        buf.len(),
        0,
    )
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
    syscall(
        SYS_WAIT4,
        -1isize as usize,
        status as *mut i32 as usize,
        0,
        0,
    ) as i32
}

// ---- sockets (AF_INET) ----
pub const AF_INET: usize = 2;
pub const SOCK_STREAM: usize = 1;
pub const SOCK_DGRAM: usize = 2;

/// Create a TCP (stream) socket. Returns an fd or a negative errno.
pub fn tcp_socket() -> i32 {
    syscall(SYS_SOCKET, AF_INET, SOCK_STREAM, 0, 0) as i32
}

/// Connect a TCP socket to `ip`:`port` (blocks through the handshake).
pub fn connect(fd: i32, ip: [u8; 4], port: u16) -> isize {
    let sa = sockaddr_in(ip, port);
    syscall(SYS_CONNECT, fd as usize, sa.as_ptr() as usize, sa.len(), 0)
}

/// Send bytes on a connected TCP socket. Returns bytes sent.
pub fn send(fd: i32, buf: &[u8]) -> isize {
    syscall5(
        SYS_SENDTO,
        fd as usize,
        buf.as_ptr() as usize,
        buf.len(),
        0,
        0,
        0,
    )
}

/// Receive bytes from a connected TCP socket. Returns bytes read, 0 at
/// end of stream, or a negative errno.
pub fn recv(fd: i32, buf: &mut [u8]) -> isize {
    syscall5(
        SYS_RECVFROM,
        fd as usize,
        buf.as_mut_ptr() as usize,
        buf.len(),
        0,
        0,
        0,
    )
}

/// Build a 16-byte `sockaddr_in` for `ip`:`port`.
pub fn sockaddr_in(ip: [u8; 4], port: u16) -> [u8; 16] {
    let mut sa = [0u8; 16];
    sa[0] = AF_INET as u8; // sin_family (little-endian u16)
    sa[2..4].copy_from_slice(&port.to_be_bytes()); // sin_port (network order)
    sa[4..8].copy_from_slice(&ip); // sin_addr
    sa
}

/// Create a UDP socket. Returns an fd, or a negative errno.
pub fn socket() -> i32 {
    syscall(SYS_SOCKET, AF_INET, SOCK_DGRAM, 0, 0) as i32
}

/// Bind the socket to a local UDP `port` (0 = ephemeral).
pub fn bind(fd: i32, port: u16) -> isize {
    let sa = sockaddr_in([0, 0, 0, 0], port);
    syscall(SYS_BIND, fd as usize, sa.as_ptr() as usize, sa.len(), 0)
}

/// Send `buf` to `ip`:`port`. Returns bytes sent or a negative errno.
pub fn sendto(fd: i32, buf: &[u8], ip: [u8; 4], port: u16) -> isize {
    let sa = sockaddr_in(ip, port);
    syscall5(
        SYS_SENDTO,
        fd as usize,
        buf.as_ptr() as usize,
        buf.len(),
        0,
        sa.as_ptr() as usize,
        sa.len(),
    )
}

/// Receive a datagram into `buf`. Returns (bytes, src_ip, src_port); a
/// negative byte count is an errno (e.g. -11 EAGAIN on timeout).
pub fn recvfrom(fd: i32, buf: &mut [u8]) -> (isize, [u8; 4], u16) {
    let mut sa = [0u8; 16];
    let n = syscall5(
        SYS_RECVFROM,
        fd as usize,
        buf.as_mut_ptr() as usize,
        buf.len(),
        0,
        sa.as_mut_ptr() as usize,
        sa.len(),
    );
    let port = u16::from_be_bytes([sa[2], sa[3]]);
    let ip = [sa[4], sa[5], sa[6], sa[7]];
    (n, ip, port)
}

/// System information (uptime seconds, total RAM, free RAM, process
/// count) — a LightOS-native shape, not Linux's `struct sysinfo`.
pub struct SysInfo {
    pub uptime_secs: u64,
    pub total_ram: u64,
    pub free_ram: u64,
    pub procs: u64,
}

/// Query kernel system information.
pub fn sysinfo() -> SysInfo {
    let mut buf = [0u8; 32];
    syscall(SYS_SYSINFO, buf.as_mut_ptr() as usize, 0, 0, 0);
    let rd = |o: usize| u64::from_le_bytes(buf[o..o + 8].try_into().unwrap());
    SysInfo {
        uptime_secs: rd(0),
        total_ram: rd(8),
        free_ram: rd(16),
        procs: rd(24),
    }
}

/// Copy the kernel's `ps`-style process listing into `buf`; returns
/// the number of bytes written.
pub fn proclist(buf: &mut [u8]) -> isize {
    syscall(SYS_PROCLIST, buf.as_mut_ptr() as usize, buf.len(), 0, 0)
}

/// Power the machine off (`cmd` 0) or reboot it (nonzero). Only returns
/// on failure.
pub fn reboot(restart: bool) -> isize {
    syscall(SYS_REBOOT, usize::from(restart), 0, 0, 0)
}

/// Register an NCE affinity hint mask for this process.
pub fn sched_setaffinity(mask: usize) -> isize {
    let m = mask.to_le_bytes();
    syscall(SYS_SCHED_SETAFFINITY, 0, 8, m.as_ptr() as usize, 0)
}

pub fn sched_getaffinity() -> usize {
    let mut m = [0u8; 8];
    syscall(SYS_SCHED_GETAFFINITY, 0, 8, m.as_mut_ptr() as usize, 0);
    usize::from_le_bytes(m)
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
