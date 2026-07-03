//! LightOS init (PID 1).
//!
//! Phase 3/4 duties: prove the process model end-to-end (fork, wait,
//! preemptive interleaving, syscalls from real U-mode). Phase 5 adds
//! mounting the root filesystem and spawning the shell.
#![no_std]
#![no_main]

use libc_shim::{exit, fork, getpid, println, spin_delay, wait};

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

    // PID 1 must never exit; idle politely.
    loop {
        spin_delay(DELAY * 10);
    }
}
