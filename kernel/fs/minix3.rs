//! Minix3 filesystem driver (read-only in v1; the format is
//! royalty-free and fully documented).
//!
//! On-disk layout (1024-byte blocks): boot, superblock, inode bitmap,
//! zone bitmap, inode table (64-byte inodes), data zones. Directory
//! entries are 64 bytes: u32 inode + 60-byte name. Zones 0..6 are
//! direct, zone 7 single-indirect, zone 8 double-indirect (v1 supports
//! direct + single-indirect: files up to 269 KiB).

use crate::drivers::virtio::blk;
use alloc::vec;
use alloc::vec::Vec;

pub const BLOCK_SIZE: usize = 1024;
const MAGIC_V3: u16 = 0x4d5a;
const INODE_SIZE: usize = 64;
pub const ROOT_INO: u32 = 1;

const DIRENT_SIZE: usize = 64;
const ZONES_PER_BLOCK: usize = BLOCK_SIZE / 4;

pub const S_IFDIR: u16 = 0o040000;
pub const S_IFREG: u16 = 0o100000;
const S_IFMT: u16 = 0o170000;

#[derive(Clone, Copy)]
pub struct Inode {
    pub ino: u32,
    pub mode: u16,
    pub nlinks: u16,
    pub size: u32,
    zones: [u32; 10],
}

impl Inode {
    pub fn is_dir(&self) -> bool {
        self.mode & S_IFMT == S_IFDIR
    }
    pub fn is_file(&self) -> bool {
        self.mode & S_IFMT == S_IFREG
    }
}

pub struct MinixFs {
    imap_blocks: u16,
    zmap_blocks: u16,
    ninodes: u32,
    zones: u32,
}

fn read_block(block: u32, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), &'static str> {
    blk::read_sectors(block as u64 * 2, buf)
}

fn le16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

impl MinixFs {
    /// Read and validate the superblock (block 1).
    pub fn mount() -> Result<MinixFs, &'static str> {
        let mut sb = [0u8; BLOCK_SIZE];
        read_block(1, &mut sb)?;
        let magic = le16(&sb, 24);
        if magic != MAGIC_V3 {
            return Err("minix3: bad superblock magic");
        }
        if le16(&sb, 28) as usize != BLOCK_SIZE {
            return Err("minix3: unsupported block size");
        }
        Ok(MinixFs {
            ninodes: le32(&sb, 0),
            imap_blocks: le16(&sb, 6),
            zmap_blocks: le16(&sb, 8),
            zones: le32(&sb, 20),
        })
    }

    fn inode_table_block(&self) -> u32 {
        2 + self.imap_blocks as u32 + self.zmap_blocks as u32
    }

    /// Load inode `ino` (1-based, root = 1).
    pub fn inode(&self, ino: u32) -> Result<Inode, &'static str> {
        if ino == 0 || ino > self.ninodes {
            return Err("minix3: inode out of range");
        }
        let index = (ino - 1) as usize;
        let block = self.inode_table_block() + (index / (BLOCK_SIZE / INODE_SIZE)) as u32;
        let off = (index % (BLOCK_SIZE / INODE_SIZE)) * INODE_SIZE;
        let mut buf = [0u8; BLOCK_SIZE];
        read_block(block, &mut buf)?;
        let mut zones = [0u32; 10];
        for (i, z) in zones.iter_mut().enumerate() {
            *z = le32(&buf, off + 24 + i * 4);
        }
        Ok(Inode {
            ino,
            mode: le16(&buf, off),
            nlinks: le16(&buf, off + 2),
            size: le32(&buf, off + 8),
            zones,
        })
    }

    /// Zone number holding byte range `[n*1024, (n+1)*1024)` of the
    /// file, following the single-indirect zone when needed.
    fn file_zone(&self, inode: &Inode, n: usize) -> Result<u32, &'static str> {
        if n < 7 {
            return Ok(inode.zones[n]);
        }
        let indirect = n - 7;
        if indirect < ZONES_PER_BLOCK {
            if inode.zones[7] == 0 {
                return Ok(0); // hole
            }
            let mut buf = [0u8; BLOCK_SIZE];
            read_block(inode.zones[7], &mut buf)?;
            return Ok(le32(&buf, indirect * 4));
        }
        Err("minix3: file too large (double-indirect unsupported)")
    }

    /// Read up to `buf.len()` bytes at `offset`; returns bytes read.
    pub fn read_at(
        &self,
        inode: &Inode,
        offset: usize,
        buf: &mut [u8],
    ) -> Result<usize, &'static str> {
        let size = inode.size as usize;
        if offset >= size {
            return Ok(0);
        }
        let want = buf.len().min(size - offset);
        let mut done = 0;
        while done < want {
            let pos = offset + done;
            let zone = self.file_zone(inode, pos / BLOCK_SIZE)?;
            let in_block = pos % BLOCK_SIZE;
            let n = (BLOCK_SIZE - in_block).min(want - done);
            if zone == 0 {
                // Sparse hole reads as zeros.
                buf[done..done + n].fill(0);
            } else {
                let mut block = [0u8; BLOCK_SIZE];
                read_block(zone, &mut block)?;
                buf[done..done + n].copy_from_slice(&block[in_block..in_block + n]);
            }
            done += n;
        }
        Ok(done)
    }

    /// Whole-file read (for exec).
    pub fn read_file(&self, inode: &Inode) -> Result<Vec<u8>, &'static str> {
        let mut data = vec![0u8; inode.size as usize];
        let n = self.read_at(inode, 0, &mut data)?;
        data.truncate(n);
        Ok(data)
    }

    /// Iterate directory entries: `f(name, ino)`.
    pub fn readdir(&self, dir: &Inode, mut f: impl FnMut(&str, u32)) -> Result<(), &'static str> {
        if !dir.is_dir() {
            return Err("minix3: not a directory");
        }
        let mut offset = 0;
        let mut entry = [0u8; DIRENT_SIZE];
        while offset + DIRENT_SIZE <= dir.size as usize {
            let n = self.read_at(dir, offset, &mut entry)?;
            if n < DIRENT_SIZE {
                break;
            }
            let ino = le32(&entry, 0);
            if ino != 0 {
                let name_end = entry[4..].iter().position(|&b| b == 0).unwrap_or(60);
                if let Ok(name) = core::str::from_utf8(&entry[4..4 + name_end]) {
                    f(name, ino);
                }
            }
            offset += DIRENT_SIZE;
        }
        Ok(())
    }

    /// Look up one component in a directory.
    pub fn dir_lookup(&self, dir: &Inode, name: &str) -> Result<Option<u32>, &'static str> {
        let mut found = None;
        self.readdir(dir, |entry_name, ino| {
            if entry_name == name {
                found = Some(ino);
            }
        })?;
        Ok(found)
    }

    /// Resolve an absolute path to an inode.
    pub fn lookup(&self, path: &str) -> Result<Option<Inode>, &'static str> {
        let mut inode = self.inode(ROOT_INO)?;
        for comp in path.split('/').filter(|c| !c.is_empty() && *c != ".") {
            if !inode.is_dir() {
                return Ok(None);
            }
            match self.dir_lookup(&inode, comp)? {
                Some(ino) => inode = self.inode(ino)?,
                None => return Ok(None),
            }
        }
        Ok(Some(inode))
    }

    pub fn stats(&self) -> (u32, u32) {
        (self.ninodes, self.zones)
    }
}
