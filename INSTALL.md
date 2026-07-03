# Installing LightOS

LightOS runs as a guest under QEMU's RISC-V system emulator. There's no
hardware to flash and nothing to install into your host OS — you
download a small bundle (a kernel + a root filesystem image) and boot
it.

## Which path for your OS

| Host | Recommended path |
|------|------------------|
| Linux | one-line installer, or download a bundle, or build from source |
| macOS | one-line installer (`brew install qemu` first) |
| **Windows** | **use WSL** and follow the Linux steps, *or* native QEMU for Windows (see below). The `curl … \| bash` one-liner does **not** run in PowerShell/CMD. |

> **Note:** the one-line installer downloads a *published GitHub
> release*. If no release has been cut yet, it will tell you so and
> point you at the build-from-source path below (which always works).

## Prerequisite: QEMU

You need `qemu-system-riscv64`:

| Platform | Command |
|----------|---------|
| Debian/Ubuntu | `sudo apt-get install qemu-system-misc` |
| Fedora | `sudo dnf install qemu-system-riscv` |
| Arch | `sudo pacman -S qemu-system-riscv` |
| macOS (Homebrew) | `brew install qemu` |

## Option 1 — one-line installer (recommended)

Installs the latest release into `~/.lightos` and adds a `lightos`
command to `~/.local/bin`:

```sh
curl -fsSL https://raw.githubusercontent.com/Lightiam/LightOS_Orion_RISC-V/main/scripts/install.sh | bash
```

Then boot it:

```sh
lightos
```

(If `lightos` isn't found, add `~/.local/bin` to your `PATH`.)

## Option 2 — download a release bundle

Grab `lightos-<version>.tar.gz` from the
[Releases page](https://github.com/Lightiam/LightOS_Orion_RISC-V/releases),
then:

```sh
tar xzf lightos-*.tar.gz
cd lightos-*/
sha256sum -c SHA256SUMS   # optional: verify integrity
./run.sh
```

## Option 3 — build from source

```sh
git clone https://github.com/Lightiam/LightOS_Orion_RISC-V
cd LightOS_Orion_RISC-V
make run          # build everything and boot
# or:
make release      # produce dist/lightos-<version>.tar.gz
```

Requirements to build: Rust (via rustup — `rust-toolchain.toml` pins
the target), GNU make, python3, and `fsck.minix` (util-linux).

## Option 4 — Docker (no host toolchain)

```sh
docker build -t lightos .
docker run --rm -it lightos make run
```

## Windows

The `curl … | bash` one-liner is a bash script — it will not run in
PowerShell or CMD (there, `curl` is an alias for `Invoke-WebRequest`).
Pick one of:

**A. WSL (recommended).** In a WSL Ubuntu shell, follow the Linux
instructions exactly:

```sh
sudo apt-get update && sudo apt-get install -y qemu-system-misc
curl -fsSL https://raw.githubusercontent.com/Lightiam/LightOS_Orion_RISC-V/main/scripts/install.sh | bash
lightos
```

**B. Native QEMU for Windows.** Install QEMU from
<https://qemu.weilnetz.de/w64/> and add its folder to your `PATH`.
Download a release bundle from the
[Releases page](https://github.com/Lightiam/LightOS_Orion_RISC-V/releases),
unzip it, and in PowerShell run the included launcher:

```powershell
cd .\lightos-0.1.0\
.\run.ps1
```

## Using LightOS

You boot to an interactive shell:

```
lightos:/$ help
lightos:/$ ls /
lightos:/$ cat /etc/motd
lightos:/$ selftest         # exercise processes, syscalls, mmap
lightos:/$ ncectl           # inspect the NCE accelerator devices
```

Exit the emulator with **Ctrl-A** then **x**.

### Runner options

```sh
lightos --memory 256    # 256 MiB of guest RAM
lightos --smp 2         # 2 harts
lightos --headless      # no interactive stdio (smoke test)
lightos -- <qemu args>  # pass extra flags straight to QEMU
```

## Publishing a release (maintainers)

Releases are cut by pushing a version tag; CI builds the bundle and
publishes it:

```sh
# bump VERSION and the Cargo.toml versions to match, commit, then:
git tag v0.1.0
git push origin v0.1.0
```

The `.github/workflows/release.yml` workflow runs the full boot test,
builds the bundle, and attaches the tarball + checksum to a GitHub
Release.
