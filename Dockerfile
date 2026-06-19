FROM rustlang/rust:nightly

ENV DEBIAN_FRONTEND=noninteractive
WORKDIR /workspace

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    clang \
    lld \
    llvm \
    nasm \
    qemu-system-x86 \
    qemu-utils \
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
