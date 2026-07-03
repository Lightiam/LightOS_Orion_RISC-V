//! In-memory filesystem for early boot: serves the userspace ELF
//! images embedded in the kernel until (or in place of) the Minix3
//! root on the block device. exec() falls back here when the disk has
//! no such path.

static INIT_ELF: &[u8] =
    include_bytes!("../../userspace/target/riscv64gc-unknown-none-elf/release/init");
static HELLO_ELF: &[u8] =
    include_bytes!("../../userspace/target/riscv64gc-unknown-none-elf/release/hello");

static FILES: &[(&str, &[u8])] = &[
    ("init", INIT_ELF),
    ("/bin/init", INIT_ELF),
    ("hello", HELLO_ELF),
    ("/bin/hello", HELLO_ELF),
];

pub fn lookup(name: &str) -> Option<&'static [u8]> {
    FILES.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)
}
