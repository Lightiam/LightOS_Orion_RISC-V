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
TIMEOUT=${TIMEOUT:-35}
OUT=$(mktemp)
trap 'rm -f "$OUT"' EXIT

[ -f "$DISK" ] || { echo "missing $DISK — run 'make test' (builds the rootfs image)"; exit 1; }

# Scripted console session. A production LightOS boots straight to a
# shell, so the process/syscall self-tests (phases 3-4) are driven from
# the shell via /bin/selftest, then a full interactive workout.
feed_input() {
    sleep 4
    printf 'selftest\n';                 sleep 8   # phases 3-4
    printf 'ls /\n';                     sleep 1
    printf 'cat /etc/motd\n';            sleep 1
    printf 'echo shell-echo-works\n';    sleep 1
    printf 'cd /bin\n';                  sleep 1
    printf 'pwd\n';                      sleep 1
    printf 'ls\n';                       sleep 1
    printf 'hello\n';                    sleep 1
    printf 'ncectl\n';                   sleep 1
    printf 'exit\n';                     sleep 2   # shell respawn
    printf 'cd /\n';                     sleep 1
    printf 'uname\n';                    sleep 1
    printf 'free\n';                     sleep 1
    printf 'uptime\n';                   sleep 1
    printf 'ps\n';                       sleep 1
    printf 'netprobe\n';                 sleep 5   # UDP socket DNS round-trip
    printf 'poweroff\n';                 sleep 2   # clean machine shutdown
}

feed_input | timeout --foreground "$TIMEOUT" qemu-system-riscv64 \
    -machine virt -cpu rv64 -smp 4 -m 128M \
    -bios none -kernel "$KERNEL" \
    -netdev user,id=net0 \
    -device virtio-net-device,netdev=net0 \
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

# Phase 3: two user processes, preemptive interleaving, wait/exit
# (driven by /bin/selftest from the shell).
expect "selftest: pid .* starting"
expect "proc A (pid .*): round 4"
expect "proc B (pid .*): round 4"
expect "selftest: reaped child pid .* with exit code 7"
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

# Phase 4: remaining syscall surface (mmap/munmap, execve).
expect "selftest: mmap/munmap of 12288 bytes verified"
expect "hello: exec works, running as pid"
expect "selftest: exec'd child pid .* exited with code 42"
expect "\[phase 4\] milestone"

# Phase 5: virtio-blk + Minix3 root + /etc/motd + shell launch.
# (The shell's own line editor exercises the blocking read(0) path on
# every command it reads.)
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

# Phase 7: NCE HAL via /dev/nce0 from userspace.
expect "nce: no NCE nodes in device tree"
expect "nce0: state=idle"
expect "idle->turbo correctly rejected"
expect "nce0: state=turbo"
expect "sched_setaffinity(nce0) -> 0"
expect "\[phase 7\] milestone"

# Networking: virtio-net + ARP + ICMP against the QEMU gateway.
expect "virtio-net: .* up, MAC"
expect "net: interface up 10.0.2.15/24"
expect "net: gateway 10.0.2.2 is at"
expect "net: ping reply from 10.0.2.2"
expect "\[net\] milestone: ARP + ICMP over virtio-net OK"

# UDP sockets: a userspace program does a DNS request/response.
expect "netprobe: DNS query for example.com"
expect "netprobe: reply .* bytes from 10.0.2.3:53"
expect "\[net\] udp round-trip OK"

# System commands: uname / free / uptime / ps / poweroff.
expect "LightOS .* riscv64 QEMU-virt"    # uname
expect "RAM .* KiB"                       # free
expect "up .* seconds, .* processes"      # uptime
expect "PID  PPID  STATE"                 # ps header
expect "run.*/bin/sh"                     # ps lists the running shell
expect "LightOS: powering off"            # poweroff reached the finisher

if grep -qi "panic" "$OUT"; then
    echo "FAIL: kernel panicked"
    FAIL=1
fi

exit $FAIL
