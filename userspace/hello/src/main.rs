//! exec() smoke-test binary: proves execve replaced the caller's image.
#![no_std]
#![no_main]

use libc_shim::{getpid, println};

#[no_mangle]
extern "C" fn main() -> i32 {
    println!("hello: exec works, running as pid {}", getpid());
    42
}
