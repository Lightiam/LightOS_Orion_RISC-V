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

# Root filesystem: Minix3 image with /etc/motd and userspace binaries.
USER_TARGET := userspace/target/riscv64gc-unknown-none-elf/release
ROOTFS_STAGE := target/rootfs

$(DISK): userspace scripts/mkfs_minix3.py $(wildcard rootfs/**/*)
	rm -rf $(ROOTFS_STAGE)
	mkdir -p $(ROOTFS_STAGE)/bin
	cp -r rootfs/. $(ROOTFS_STAGE)/
	cp $(USER_TARGET)/init $(ROOTFS_STAGE)/bin/init
	cp $(USER_TARGET)/hello $(ROOTFS_STAGE)/bin/hello
	cp $(USER_TARGET)/sh $(ROOTFS_STAGE)/bin/sh
	cp $(USER_TARGET)/ncectl $(ROOTFS_STAGE)/bin/ncectl
	chmod +x $(ROOTFS_STAGE)/bin/*
	python3 scripts/mkfs_minix3.py $(DISK) $(ROOTFS_STAGE) 8192
	fsck.minix -f $(DISK)

.PHONY: disk
disk:
	rm -f $(DISK)
	$(MAKE) $(DISK)

run: build disk
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
	cargo clippy --lib --bins

clean:
	cargo clean
	cd userspace && cargo clean
	rm -f $(DISK)

# Build and run the full boot test inside Docker (no host toolchain
# needed beyond Docker itself).
.PHONY: docker-test
docker-test:
	docker build -t lightos .
	docker run --rm lightos
