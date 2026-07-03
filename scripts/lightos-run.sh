#!/usr/bin/env bash
# LightOS launcher — boots the OS in QEMU from a release bundle.
#
# This script lives next to the kernel (`lightos`) and root filesystem
# (`rootfs.img`) inside a LightOS release bundle. Run it with no
# arguments for an interactive shell session:
#
#     ./run.sh
#
# Options:
#   --headless        run without stdio interaction (CI / smoke test)
#   --memory <MiB>    guest RAM (default 128)
#   --smp <N>         number of harts (default 4)
#   --                pass everything after this straight to QEMU
#
# Exit the interactive session with Ctrl-A then x.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KERNEL="$HERE/lightos"
DISK="$HERE/rootfs.img"

MEM=128
SMP=4
SERIAL="stdio"
EXTRA=()

while [ $# -gt 0 ]; do
    case "$1" in
        --headless) SERIAL="mon:stdio"; shift ;;
        --memory)   MEM="$2"; shift 2 ;;
        --smp)      SMP="$2"; shift 2 ;;
        --)         shift; EXTRA=("$@"); break ;;
        -h|--help)  sed -n '2,17p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "run.sh: unknown option '$1' (use -- to pass QEMU args)" >&2; exit 2 ;;
    esac
done

QEMU=qemu-system-riscv64
if ! command -v "$QEMU" >/dev/null 2>&1; then
    cat >&2 <<'EOF'
error: qemu-system-riscv64 not found.

Install the RISC-V system emulator, then re-run:
  Debian/Ubuntu : sudo apt-get install qemu-system-misc
  Fedora        : sudo dnf install qemu-system-riscv
  Arch          : sudo pacman -S qemu-system-riscv
  macOS (brew)  : brew install qemu
EOF
    exit 1
fi

for f in "$KERNEL" "$DISK"; do
    [ -f "$f" ] || { echo "error: missing $f (is this a complete bundle?)" >&2; exit 1; }
done

echo "Booting LightOS (mem ${MEM}M, ${SMP} harts). Exit with Ctrl-A x."
exec "$QEMU" \
    -machine virt \
    -cpu rv64 \
    -smp "$SMP" \
    -m "${MEM}M" \
    -bios none \
    -kernel "$KERNEL" \
    -drive "file=$DISK,format=raw,id=hd0" \
    -device virtio-blk-device,drive=hd0 \
    -serial "$SERIAL" \
    -display none \
    "${EXTRA[@]}"
