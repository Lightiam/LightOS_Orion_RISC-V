# LightOS build + test environment: Rust (riscv64gc bare-metal target),
# QEMU RISC-V system emulator, and the Minix tooling used to validate
# the root filesystem image.
#
#   docker build -t lightos .
#   docker run --rm lightos            # runs the full QEMU boot test
#   docker run --rm -it lightos make run   # interactive shell session
FROM rust:1.94-slim-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
        make \
        python3 \
        qemu-system-misc \
        util-linux \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /lightos
COPY . .

# rust-toolchain.toml pins the channel; pre-fetch the target so the
# image is ready to build offline.
RUN rustup target add riscv64gc-unknown-none-elf

RUN make build disk

CMD ["make", "test"]
