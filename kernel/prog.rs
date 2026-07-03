//! Embedded program registry: userspace ELF images linked into the
//! kernel until the root filesystem can serve them (Phase 5 mounts
//! Minix3 and looks there first).

static INIT_ELF: &[u8] =
    include_bytes!("../userspace/target/riscv64gc-unknown-none-elf/release/init");
static HELLO_ELF: &[u8] =
    include_bytes!("../userspace/target/riscv64gc-unknown-none-elf/release/hello");

/// Programs available to exec() before/without a filesystem.
static PROGRAMS: &[(&str, &[u8])] = &[
    ("init", INIT_ELF),
    ("/bin/init", INIT_ELF),
    ("hello", HELLO_ELF),
    ("/bin/hello", HELLO_ELF),
];

pub fn lookup(name: &str) -> Option<&'static [u8]> {
    PROGRAMS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, bytes)| *bytes)
}
