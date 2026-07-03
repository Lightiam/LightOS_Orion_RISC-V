# LightOS — build, run, debug, test (QEMU virt, RV64GC)

TARGET  := riscv64gc-unknown-none-elf
KERNEL  := target/$(TARGET)/debug/lightos
DISK    := disk.img

QEMU_ARGS = \
	-machine virt \
	-cpu rv64 \
	-smp 4 \
	-m 128M \
	-bios none \
	-kernel $(KERNEL) \
	-drive file=$(DISK),format=raw,id=hd0 \
	-device virtio-blk-device,drive=hd0 \
	-serial stdio \
	-display none

.PHONY: build userspace run gdb test clean fmt clippy

# Userspace ELFs are embedded into the kernel (kernel/prog.rs), so they
# must be built first.
userspace:
	cd userspace && cargo build --release

build: userspace
	cargo build

$(DISK):
	dd if=/dev/zero of=$(DISK) bs=1M count=32 status=none

run: build $(DISK)
	qemu-system-riscv64 $(QEMU_ARGS)

# Non-interactive boot check: boot for a few seconds, capture UART,
# assert the current phase milestone appears. Used by CI and by the
# per-phase verification workflow.
test: build $(DISK)
	./scripts/boot_test.sh

gdb: build $(DISK)
	qemu-system-riscv64 $(QEMU_ARGS) -s -S &
	gdb-multiarch $(KERNEL) \
		-ex "target remote :1234" \
		-ex "set architecture riscv:rv64"

fmt:
	cargo fmt --all

clippy:
	cargo clippy --all-targets

clean:
	cargo clean
	cd userspace && cargo clean
	rm -f $(DISK)
