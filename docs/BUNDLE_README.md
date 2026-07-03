# LightOS — QEMU release bundle

This is a self-contained LightOS release. Everything needed to boot the
OS is in this folder.

```
lightos       the kernel (RISC-V RV64GC, release build)
rootfs.img    the Minix3 root filesystem
run.sh        launcher
VERSION       this release's version
SHA256SUMS    integrity checksums
```

## Run it

The only requirement is the RISC-V system emulator, QEMU:

```
Debian/Ubuntu : sudo apt-get install qemu-system-misc
Fedora        : sudo dnf install qemu-system-riscv
Arch          : sudo pacman -S qemu-system-riscv
macOS (brew)  : brew install qemu
```

Then:

```sh
./run.sh
```

You'll boot to an interactive shell:

```
lightos:/$ ls /
bin/  etc/
lightos:/$ cat /etc/motd
lightos:/$ ncectl          # inspect the NCE accelerator devices
```

Exit the session with **Ctrl-A** then **x**.

## Options

```
./run.sh --memory 256      # give the guest 256 MiB
./run.sh --smp 2           # 2 harts
./run.sh --headless        # no interactive stdio (smoke test)
./run.sh -- <qemu args>    # pass extra flags straight to QEMU
```

## Verify integrity

```sh
sha256sum -c SHA256SUMS
```

LightOS is royalty-free and dual-licensed Apache-2.0 OR MIT.
Source: https://github.com/Lightiam/LightOS_Orion_RISC-V
