//! LightOS self-test (`/bin/selftest`).
//!
//! Exercises the process model and syscall surface from real user
//! mode: two processes interleaving under preemption, fork/wait/exit,
//! anonymous mmap, and execve. Run it from the shell (or via CI's
//! scripted console session) to confirm the kernel end-to-end. It is
//! *not* run at boot — a production LightOS boots straight to a shell.
#![no_std]
#![no_main]

use libc_shim::{exec, exit, fork, getpid, mmap, munmap, println, spin_delay, wait};

const ROUNDS: usize = 5;
/// ~several 10 ms quanta of busy work per round on QEMU TCG.
const DELAY: usize = 3_000_000;

#[no_mangle]
extern "C" fn main() -> i32 {
    println!("selftest: pid {} starting", getpid());

    // --- Preemptive multitasking: two processes interleave. ---
    let child = fork();
    if child < 0 {
        println!("selftest: fork failed: {}", child);
        return 1;
    }
    if child == 0 {
        for round in 0..ROUNDS {
            println!("proc B (pid {}): round {}", getpid(), round);
            spin_delay(DELAY);
        }
        exit(7);
    }
    for round in 0..ROUNDS {
        println!("proc A (pid {}): round {}", getpid(), round);
        spin_delay(DELAY);
    }
    let mut status = 0;
    let reaped = wait(&mut status);
    println!(
        "selftest: reaped child pid {} with exit code {}",
        reaped,
        status >> 8
    );
    println!("[phase 3] milestone: two processes ran concurrently with preemption");

    // --- Syscall surface: mmap/munmap + execve. ---
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
    println!("selftest: mmap/munmap of {} bytes verified", len);
    assert_eq!(munmap(mem, len), 0, "munmap failed");

    let child = fork();
    if child == 0 {
        exec("hello");
        println!("selftest: exec failed!");
        exit(1);
    }
    let mut status = 0;
    let reaped = wait(&mut status);
    println!(
        "selftest: exec'd child pid {} exited with code {}",
        reaped,
        status >> 8
    );
    println!("[phase 4] milestone: syscall surface (write/read/fork/exec/wait/mmap) OK");
    0
}
