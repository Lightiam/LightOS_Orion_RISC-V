fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-link-arg-bin=lightos=-T{manifest}/boot/linker.ld");
    println!("cargo:rerun-if-changed={manifest}/boot/linker.ld");
    println!("cargo:rerun-if-changed={manifest}/boot/entry.S");
}
