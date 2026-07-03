//! Virtual File System: a thin façade over the mounted root
//! filesystem (Minix3 on virtio-blk) with the embedded ramfs as
//! early-boot fallback for program images.

pub mod minix3;
pub mod ramfs;

use crate::lock::SpinLock;
use crate::uart_println;
use alloc::vec::Vec;
use minix3::{Inode, MinixFs};

/// Mounted root FS. Lock invariant: guards mount/unmount only; the FS
/// itself is immutable (read-only driver) once mounted.
static ROOT: SpinLock<Option<MinixFs>> = SpinLock::new(None);

/// Mount the Minix3 root from the first virtio-blk device.
pub fn mount_root() -> bool {
    if !crate::drivers::virtio::blk::init() {
        uart_println!("vfs: no block device; running from embedded ramfs only");
        return false;
    }
    match MinixFs::mount() {
        Ok(fs) => {
            let (ninodes, zones) = fs.stats();
            uart_println!(
                "vfs: mounted Minix3 root ({} inodes, {} zones)",
                ninodes,
                zones
            );
            *ROOT.lock() = Some(fs);
            true
        }
        Err(e) => {
            uart_println!("vfs: mount failed: {}", e);
            false
        }
    }
}

fn with_root<T>(f: impl FnOnce(&MinixFs) -> Result<T, &'static str>) -> Result<T, &'static str> {
    let guard = ROOT.lock();
    match guard.as_ref() {
        Some(fs) => f(fs),
        None => Err("vfs: no root filesystem mounted"),
    }
}

/// Resolve `path`; `None` when it doesn't exist.
pub fn lookup(path: &str) -> Result<Option<Inode>, &'static str> {
    with_root(|fs| fs.lookup(path))
}

pub fn read_at(inode: &Inode, offset: usize, buf: &mut [u8]) -> Result<usize, &'static str> {
    with_root(|fs| fs.read_at(inode, offset, buf))
}

pub fn read_file(inode: &Inode) -> Result<Vec<u8>, &'static str> {
    with_root(|fs| fs.read_file(inode))
}

pub fn readdir(inode: &Inode, f: impl FnMut(&str, u32)) -> Result<(), &'static str> {
    with_root(|fs| fs.readdir(inode, f))
}

pub fn inode(ino: u32) -> Result<Inode, &'static str> {
    with_root(|fs| fs.inode(ino))
}

/// Load a program image: disk first, embedded ramfs as fallback.
/// Returns the bytes to hand to the ELF loader.
pub fn load_program(path: &str) -> Option<Vec<u8>> {
    if let Ok(Some(inode)) = lookup(path) {
        if inode.is_file() {
            if let Ok(bytes) = read_file(&inode) {
                return Some(bytes);
            }
        }
    }
    ramfs::lookup(path).map(|b| b.to_vec())
}
