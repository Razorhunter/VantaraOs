# bootloader 0.11.15 pins x86_64 0.15.2, whose Step implementation predates
# the forward_overflowing/backward_overflowing additions in newer nightlies.
# This toolchain is new enough for edition 2024 and still compatible with the
# bootloader's locked BIOS/UEFI firmware crates.
FROM rust:1.85-bookworm

ENV DEBIAN_FRONTEND=noninteractive
ENV VANTARA_PINNED_TOOLCHAIN=1
WORKDIR /workspace

RUN rustup toolchain install nightly-2025-03-01 --profile minimal \
    && rustup default nightly-2025-03-01

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    clang \
    lld \
    llvm \
    nasm \
    qemu-system-x86 \
    qemu-utils \
    ovmf \
    xorriso \
    grub-pc-bin \
    grub-efi-amd64-bin \
    mtools \
    git \
    curl \
    ca-certificates \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add rust-src llvm-tools-preview rustfmt clippy \
    && rustup target add x86_64-unknown-none

RUN cargo install bootimage

RUN qemu-system-x86_64 --version

CMD ["bash"]
