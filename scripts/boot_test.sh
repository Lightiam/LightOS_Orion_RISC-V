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
TIMEOUT=${TIMEOUT:-25}
OUT=$(mktemp)
trap 'rm -f "$OUT"' EXIT

[ -f "$DISK" ] || { echo "missing $DISK — run 'make test' (builds the rootfs image)"; exit 1; }

# Scripted console session: a bare 'Z' for the blocking-read check,
# then a full interactive shell workout (Phase 6).
feed_input() {
    sleep 4
    printf 'Z'                          # init's blocking read(0)
    sleep 3
    printf 'ls /\n';                     sleep 1
    printf 'cat /etc/motd\n';            sleep 1
    printf 'echo shell-echo-works\n';    sleep 1
    printf 'cd /bin\n';                  sleep 1
    printf 'pwd\n';                      sleep 1
    printf 'ls\n';                       sleep 1
    printf 'hello\n';                    sleep 1
    printf 'exit\n';                     sleep 2
}

feed_input | timeout --foreground "$TIMEOUT" qemu-system-riscv64 \
    -machine virt -cpu rv64 -smp 4 -m 128M \
    -bios none -kernel "$KERNEL" \
    -drive file="$DISK",format=raw,id=hd0 \
    -device virtio-blk-device,drive=hd0 \
    -serial stdio -display none \
    >"$OUT" 2>&1

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

expect "LightOS Orion — a LightRail AI system"
expect "LightOS booting..."
expect "\[phase 0\] milestone"
expect "\[phase 1\] milestone"
expect "timer: 100 ticks"
expect "\[phase 2\] milestone"

# Phase 3: two user processes, preemptive interleaving, wait/exit.
expect "init: hello from userspace, pid 1"
expect "proc A (pid 1): round 4"
expect "proc B (pid 2): round 4"
expect "init: reaped child pid 2 with exit code 7"
expect "\[phase 3\] milestone"

# Preemption proof: output must alternate between A and B at least
# twice (a cooperative/serial run would switch at most once).
SWITCHES=$(grep -o "proc [AB]" "$OUT" | uniq | wc -l)
if [ "$SWITCHES" -ge 3 ]; then
    echo "PASS: preemptive interleaving ($((SWITCHES - 1)) A/B switches)"
else
    echo "FAIL: no interleaving (switches=$((SWITCHES - 1)))"
    FAIL=1
fi

# Phase 4: remaining syscall surface + blocking console read.
expect "init: mmap/munmap of 12288 bytes verified"
expect "hello: exec works, running as pid"
expect "init: exec'd child pid .* exited with code 42"
expect "\[phase 4\] milestone"
expect "init: blocking read(0) returned 'Z'"

# Phase 5: virtio-blk + Minix3 root + /etc/motd + shell launch.
expect "virtio-blk: capacity"
expect "vfs: mounted Minix3 root"
expect "Welcome to LightOS"
expect "\[phase 5\] milestone"
expect "LightOS sh v0.1"

# Phase 6: interactive shell session (scripted stdin above).
expect "bin/"                      # ls /
expect "etc/"
expect "^shell-echo-works"         # echo output (not the typed line)
expect "lightos:/bin\\$"           # prompt after cd /bin
expect "hello: exec works"         # external command from /bin
expect "init: shell exited, respawning"
if [ "$(grep -c 'Welcome to LightOS' "$OUT")" -ge 2 ]; then
    echo "PASS: cat /etc/motd re-read the file interactively"
else
    echo "FAIL: cat /etc/motd produced no second copy"
    FAIL=1
fi
if grep -q "sh  " "$OUT" && grep -q "init  " "$OUT"; then
    echo "PASS: ls /bin lists binaries"
else
    echo "FAIL: ls /bin did not list binaries"
    FAIL=1
fi

if grep -qi "panic" "$OUT"; then
    echo "FAIL: kernel panicked"
    FAIL=1
fi

exit $FAIL
