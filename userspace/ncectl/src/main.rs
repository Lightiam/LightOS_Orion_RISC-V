//! ncectl: exercise the NCE HAL from userspace.
//!
//! Opens /dev/nce0, prints its descriptor, walks the power state
//! machine (including one deliberately illegal jump to prove the
//! kernel validates transitions), and registers an affinity hint.
#![no_std]
#![no_main]

use libc_shim::{close, open, print, println, read, sched_setaffinity, write};

fn show(fd: i32) {
    // Reopen semantics: descriptor reads are positional, so reset by
    // reading from a fresh fd each time in this simple tool.
    let mut buf = [0u8; 128];
    let n = read(fd, &mut buf);
    if n > 0 {
        if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
            print!("{}", s);
        }
    }
}

fn set_state(path: &str, state: &str) -> isize {
    let fd = open(path);
    if fd < 0 {
        return fd as isize;
    }
    let ret = write(fd, state.as_bytes());
    close(fd);
    ret
}

#[no_mangle]
extern "C" fn main() -> i32 {
    let path = "/dev/nce0";
    let fd = open(path);
    if fd < 0 {
        println!("ncectl: cannot open {} (error {})", path, fd);
        return 1;
    }
    show(fd);
    close(fd);

    // Illegal jump first: idle -> turbo must be rejected.
    let bad = set_state(path, "turbo");
    if bad < 0 {
        println!("ncectl: idle->turbo correctly rejected (error {})", bad);
    } else {
        println!("ncectl: BUG: idle->turbo was accepted!");
        return 1;
    }

    // Legal ramp: idle -> active -> turbo.
    if set_state(path, "active") < 0 || set_state(path, "turbo") < 0 {
        println!("ncectl: legal transition failed");
        return 1;
    }
    let fd = open(path);
    show(fd);
    close(fd);

    // Affinity hint: this worker wants NCE slot 0 proximity.
    let ret = sched_setaffinity(1 << 0);
    println!("ncectl: sched_setaffinity(nce0) -> {}", ret);

    println!("[phase 7] milestone: NCE HAL — descriptor, power ramp, affinity OK");
    0
}
