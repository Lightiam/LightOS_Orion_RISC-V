#!/usr/bin/env bash
# LightOS end-to-end boot test.
#
# Boots the kernel in QEMU (headless), captures UART output for a few
# seconds, and asserts that every phase milestone marker implemented so
# far is present. This is the kernel's equivalent of a browser E2E run:
# real emulated hardware, real boot, real console assertions.
set -u

KERNEL=${KERNEL:-target/riscv64gc-unknown-none-elf/debug/lightos}
DISK=${DISK:-disk.img}
TIMEOUT=${TIMEOUT:-10}
OUT=$(mktemp)
trap 'rm -f "$OUT"' EXIT

[ -f "$DISK" ] || dd if=/dev/zero of="$DISK" bs=1M count=32 status=none

timeout --foreground "$TIMEOUT" qemu-system-riscv64 \
    -machine virt -cpu rv64 -smp 4 -m 128M \
    -bios none -kernel "$KERNEL" \
    -drive file="$DISK",format=raw,id=hd0 \
    -device virtio-blk-device,drive=hd0 \
    -serial stdio -display none \
    </dev/null >"$OUT" 2>&1

echo "---- UART output ----"
cat "$OUT"
echo "---------------------"

FAIL=0
expect() {
    if grep -q "$1" "$OUT"; then
        echo "PASS: $1"
    else
        echo "FAIL: missing '$1'"
        FAIL=1
    fi
}

expect "LightOS booting..."
expect "\[phase 0\] milestone"

if grep -qi "panic" "$OUT"; then
    echo "FAIL: kernel panicked"
    FAIL=1
fi

exit $FAIL
