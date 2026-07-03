//! LightOS init (PID 1).
//!
//! Production boot: the kernel has already mounted the root
//! filesystem, so init prints the message of the day and then spawns
//! and supervises the login shell — respawning it if it exits and
//! reaping orphaned children. The process/syscall self-tests live in
//! `/bin/selftest`, kept out of the boot path so a real install boots
//! straight to a usable shell.
#![no_std]
#![no_main]

use libc_shim::{close, exec, exit, fork, open, println, read, spin_delay, wait, write};

const DELAY: usize = 3_000_000;

#[no_mangle]
extern "C" fn main() -> i32 {
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
}
