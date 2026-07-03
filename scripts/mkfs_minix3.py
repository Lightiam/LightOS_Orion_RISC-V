#!/usr/bin/env python3
"""Build a Minix3 (V3, 1024-byte block) filesystem image from a
staging directory.

Usage: mkfs_minix3.py <image> <staging_dir> [size_kib]

Layout: boot block, superblock, inode bitmap, zone bitmap, inode
table, data zones. Inodes are 64 bytes (V2/V3 format); directory
entries are u32 inode + 60-byte name. Files use 7 direct zones plus
one single-indirect zone (max ~263 KiB — plenty for LightOS
userspace). Validate the result with `fsck.minix` (util-linux).
"""

import os
import struct
import sys

BS = 1024
INODE_SIZE = 64
MAGIC_V3 = 0x4D5A
DIRECT_ZONES = 7
ZONES_PER_INDIRECT = BS // 4

S_IFDIR = 0o040000
S_IFREG = 0o100000


class Builder:
    def __init__(self, total_blocks: int, ninodes: int):
        self.total_blocks = total_blocks
        self.ninodes = ninodes
        self.imap_blocks = (ninodes + 1 + BS * 8 - 1) // (BS * 8)
        inode_blocks = ninodes * INODE_SIZE // BS
        # zmap covers data zones; iterate once to settle the layout.
        self.zmap_blocks = 1
        while True:
            first_data = 2 + self.imap_blocks + self.zmap_blocks + inode_blocks
            data_zones = total_blocks - first_data
            needed = (data_zones + 1 + BS * 8 - 1) // (BS * 8)
            if needed == self.zmap_blocks:
                break
            self.zmap_blocks = needed
        self.first_data = first_data
        self.image = bytearray(total_blocks * BS)
        self.imap = bytearray(self.imap_blocks * BS)
        self.zmap = bytearray(self.zmap_blocks * BS)
        self.inodes = bytearray(ninodes * INODE_SIZE)
        self.next_inode = 1
        self.next_zone = self.first_data
        # Bit 0 of each bitmap is reserved (inode 0 / "zone 0").
        self.set_bit(self.imap, 0)
        self.set_bit(self.zmap, 0)

    @staticmethod
    def set_bit(bitmap: bytearray, n: int):
        bitmap[n // 8] |= 1 << (n % 8)

    def alloc_inode(self) -> int:
        ino = self.next_inode
        assert ino <= self.ninodes, "out of inodes"
        self.next_inode += 1
        self.set_bit(self.imap, ino)
        return ino

    def alloc_zone(self) -> int:
        zone = self.next_zone
        assert zone < self.total_blocks, "image full"
        self.next_zone += 1
        self.set_bit(self.zmap, zone - self.first_data + 1)
        return zone

    def write_inode(self, ino: int, mode: int, nlinks: int, size: int, zones):
        z = list(zones) + [0] * (10 - len(zones))
        struct.pack_into(
            "<HHHHIIII10I",
            self.inodes,
            (ino - 1) * INODE_SIZE,
            mode,
            nlinks,
            0,  # uid
            0,  # gid
            size,
            0,  # atime
            0,  # mtime
            0,  # ctime
            *z,
        )

    def store_data(self, data: bytes):
        """Allocate zones for `data`; returns the inode zone array."""
        zones = []
        blocks = [data[i : i + BS] for i in range(0, len(data), BS)]
        direct, rest = blocks[:DIRECT_ZONES], blocks[DIRECT_ZONES:]
        for chunk in direct:
            zone = self.alloc_zone()
            self.image[zone * BS : zone * BS + len(chunk)] = chunk
            zones.append(zone)
        if rest:
            assert len(rest) <= ZONES_PER_INDIRECT, "file too large"
            indirect = self.alloc_zone()
            zones += [0] * (DIRECT_ZONES - len(zones))  # pad direct slots
            zones.append(indirect)
            table = bytearray(BS)
            for i, chunk in enumerate(rest):
                zone = self.alloc_zone()
                self.image[zone * BS : zone * BS + len(chunk)] = chunk
                struct.pack_into("<I", table, i * 4, zone)
            self.image[indirect * BS : (indirect + 1) * BS] = table
        return zones

    def add_file(self, data: bytes, executable: bool) -> int:
        ino = self.alloc_inode()
        mode = S_IFREG | (0o755 if executable else 0o644)
        self.write_inode(ino, mode, 1, len(data), self.store_data(data))
        return ino

    def add_tree(self, host_dir: str, parent_ino: int = 0) -> int:
        """Recursively import a host directory. The directory inode is
        reserved *before* the children so the root becomes inode 1."""
        ino = self.alloc_inode()
        entries = [(".", ino), ("..", parent_ino or ino)]
        nlinks = 2
        for name in sorted(os.listdir(host_dir)):
            full = os.path.join(host_dir, name)
            if os.path.isdir(full):
                entries.append((name, self.add_tree(full, ino)))
                nlinks += 1  # child's '..'
            else:
                with open(full, "rb") as f:
                    data = f.read()
                entries.append((name, self.add_file(data, os.access(full, os.X_OK))))
        blob = b"".join(
            struct.pack("<I", entry_ino) + name.encode()[:59].ljust(60, b"\0")
            for name, entry_ino in entries
        )
        self.write_inode(ino, S_IFDIR | 0o755, nlinks, len(blob), self.store_data(blob))
        return ino

    def finish(self, path: str):
        # Bits past the last *valid* inode/zone are padding and must
        # read as "used" for fsck; bits for valid-but-free objects stay
        # clear.
        for n in range(self.ninodes + 1, self.imap_blocks * BS * 8):
            self.set_bit(self.imap, n)
        for n in range(self.total_blocks - self.first_data + 1, self.zmap_blocks * BS * 8):
            self.set_bit(self.zmap, n)

        sb = bytearray(BS)
        struct.pack_into(
            "<IHHHHHHIIHHHB",
            sb,
            0,
            self.ninodes,       # s_ninodes
            0,                  # s_pad0
            self.imap_blocks,   # s_imap_blocks
            self.zmap_blocks,   # s_zmap_blocks
            self.first_data,    # s_firstdatazone
            0,                  # s_log_zone_size
            0,                  # s_pad1
            0x7FFFFFFF,         # s_max_size
            self.total_blocks,  # s_zones
            MAGIC_V3,           # s_magic
            0,                  # s_pad2
            BS,                 # s_blocksize
            0,                  # s_disk_version
        )
        self.image[BS : 2 * BS] = sb
        off = 2 * BS
        self.image[off : off + len(self.imap)] = self.imap
        off += len(self.imap)
        self.image[off : off + len(self.zmap)] = self.zmap
        off += len(self.zmap)
        self.image[off : off + len(self.inodes)] = self.inodes
        with open(path, "wb") as f:
            f.write(self.image)


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    image, staging = sys.argv[1], sys.argv[2]
    size_kib = int(sys.argv[3]) if len(sys.argv) > 3 else 8192
    b = Builder(total_blocks=size_kib, ninodes=1024)
    root = b.add_tree(staging)
    assert root == 1, f"root must be inode 1, got {root}"
    b.finish(image)
    print(
        f"mkfs_minix3: {image}: {b.next_inode - 1} inodes, "
        f"{b.next_zone - b.first_data} data zones used"
    )


if __name__ == "__main__":
    main()
