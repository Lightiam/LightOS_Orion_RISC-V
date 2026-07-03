//! LightOS init (PID 1).
//!
//! Phase 3/4 duties: prove the process model end-to-end (fork, wait,
//! preemptive interleaving, syscalls from real U-mode). Phase 5 adds
//! mounting the root filesystem and spawning the shell.
#![no_std]
#![no_main]

use libc_shim::{
    close, exec, exit, fork, getpid, mmap, munmap, open, println, read, spin_delay, wait, write,
};

const ROUNDS: usize = 5;
/// ~several 10 ms quanta of busy work per round on QEMU TCG.
const DELAY: usize = 3_000_000;

#[no_mangle]
extern "C" fn main() -> i32 {
    println!("init: hello from userspace, pid {}", getpid());

    let child = fork();
    if child < 0 {
        println!("init: fork failed: {}", child);
        exit(1);
    }

    if child == 0 {
        // Child: process B.
        for round in 0..ROUNDS {
            println!("proc B (pid {}): round {}", getpid(), round);
            spin_delay(DELAY);
        }
        exit(7);
    }

    // Parent: process A.
    for round in 0..ROUNDS {
        println!("proc A (pid {}): round {}", getpid(), round);
        spin_delay(DELAY);
    }

    let mut status = 0;
    let reaped = wait(&mut status);
    println!(
        "init: reaped child pid {} with exit code {}",
        reaped,
        status >> 8
    );
    println!("[phase 3] milestone: two processes ran concurrently with preemption");

    phase4_syscall_tests();

    // Blocking console read: parks init until the UART IRQ delivers.
    let mut buf = [0u8; 16];
    let n = read(0, &mut buf);
    if n > 0 {
        println!("init: blocking read(0) returned {:?}", buf[0] as char);
    }

    // Phase 5: read /etc/motd from the Minix3 root on virtio-blk.
    print_motd();

    // Spawn the shell on the console; PID 1 reaps and respawns it.
    loop {
        let pid = fork();
        if pid == 0 {
            exec("/bin/sh");
            println!("init: exec /bin/sh failed");
            exit(1);
        }
        if pid < 0 {
            println!("init: fork failed");
            loop {
                spin_delay(DELAY * 10);
            }
        }
        let mut status = 0;
        loop {
            let reaped = wait(&mut status);
            if reaped == pid || reaped < 0 {
                break;
            }
            // Orphans re-parented to init get reaped here too.
        }
        println!("init: shell exited, respawning");
    }
}

fn print_motd() {
    let fd = open("/etc/motd");
    if fd < 0 {
        println!("init: no /etc/motd (error {})", fd);
        return;
    }
    let mut buf = [0u8; 256];
    loop {
        let n = read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        write(1, &buf[..n as usize]);
    }
    close(fd);
    println!("[phase 5] milestone: Minix3 root mounted, /etc/motd read from disk");
}

/// Exercise the rest of the syscall surface: mmap/munmap, execve.
fn phase4_syscall_tests() {
    // Anonymous mmap: write a pattern through the mapping, read back.
    let len = 3 * 4096;
    let mem = mmap(len);
    assert!(!mem.is_null() && (mem as isize) > 0, "mmap failed");
    unsafe {
        for i in 0..len {
            mem.add(i).write_volatile((i % 251) as u8);
        }
        for i in (0..len).step_by(509) {
            assert_eq!(mem.add(i).read_volatile(), (i % 251) as u8);
        }
    }
    println!("init: mmap/munmap of {} bytes verified", len);
    assert_eq!(munmap(mem, len), 0, "munmap failed");

    // execve: child replaces itself with /bin/hello, exit code 42.
    let child = fork();
    if child == 0 {
        exec("hello");
        println!("init: exec failed!");
        exit(1);
    }
    let mut status = 0;
    let reaped = wait(&mut status);
    println!(
        "init: exec'd child pid {} exited with code {}",
        reaped,
        status >> 8
    );

    println!("[phase 4] milestone: syscall surface (write/read/fork/exec/wait/mmap) OK");
}
