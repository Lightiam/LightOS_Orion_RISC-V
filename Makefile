# LightOS — build, run, debug, test (QEMU virt, RV64GC)

TARGET  := riscv64gc-unknown-none-elf
KERNEL  := target/$(TARGET)/debug/lightos
RELEASE_KERNEL := target/$(TARGET)/release/lightos
DISK    := disk.img
VERSION := $(shell cat VERSION)

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
	cp $(USER_TARGET)/selftest $(ROOTFS_STAGE)/bin/selftest
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
	rm -rf $(DIST)

# Build and run the full boot test inside Docker (no host toolchain
# needed beyond Docker itself).
.PHONY: docker-test
docker-test:
	docker build -t lightos .
	docker run --rm lightos

# ---------------------------------------------------------------------
# Release packaging: a self-contained, downloadable QEMU bundle.
#
#   make release            -> dist/lightos-<ver>/ + dist/lightos-<ver>.tar.gz
#
# The bundle holds a release-optimized kernel, the root filesystem
# image, a self-locating run.sh, quickstart docs, and SHA256SUMS. A
# user unpacks it and runs ./run.sh — nothing else required but QEMU.
# ---------------------------------------------------------------------
DIST        := dist
RELEASE_DIR := $(DIST)/lightos-$(VERSION)

.PHONY: build-release release release-tarball
build-release: userspace
	cargo build --release

release: build-release $(DISK)
	rm -rf $(RELEASE_DIR)
	mkdir -p $(RELEASE_DIR)
	cp $(RELEASE_KERNEL) $(RELEASE_DIR)/lightos
	cp $(DISK) $(RELEASE_DIR)/rootfs.img
	cp scripts/lightos-run.sh $(RELEASE_DIR)/run.sh
	cp scripts/lightos-run.ps1 $(RELEASE_DIR)/run.ps1
	cp VERSION $(RELEASE_DIR)/VERSION
	cp docs/BUNDLE_README.md $(RELEASE_DIR)/README.md
	chmod +x $(RELEASE_DIR)/run.sh
	cd $(RELEASE_DIR) && sha256sum lightos rootfs.img run.sh run.ps1 > SHA256SUMS
	@echo "release bundle staged at $(RELEASE_DIR)"
	$(MAKE) release-tarball

release-tarball:
	cd $(DIST) && tar czf lightos-$(VERSION).tar.gz lightos-$(VERSION)
	cd $(DIST) && sha256sum lightos-$(VERSION).tar.gz > lightos-$(VERSION).tar.gz.sha256
	@echo "tarball: $(DIST)/lightos-$(VERSION).tar.gz"
