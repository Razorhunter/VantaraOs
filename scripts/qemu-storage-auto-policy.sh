#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_TARGET_DIR="${ROOT_DIR}/target/storage-auto-cargo"
BOOTIMAGE="${CARGO_TARGET_DIR}/x86_64-vantara_os/debug/bootimage-kernel.bin"

bash "${ROOT_DIR}/scripts/build-kernel-variant.sh" \
  "${CARGO_TARGET_DIR}" \
  --no-default-features \
  --features storage-auto

BOOTIMAGE_OVERRIDE="${BOOTIMAGE}" \
PERSIST_IMAGE_OVERRIDE="${ROOT_DIR}/target/vantara-storage-auto-nvme.img" \
QEMU_MACHINE=q35 \
PERSIST_BUS=nvme \
PERSIST_BACKEND=nvme \
STORAGE_POLICY=auto \
STORAGE_FALLBACK=false \
QEMU="${QEMU:-qemu-system-x86_64}" \
  bash "${ROOT_DIR}/scripts/qemu-filesystem-persistence.sh"
cp "${ROOT_DIR}/target/qemu-filesystem-persistence-first.log" \
  "${ROOT_DIR}/target/qemu-storage-auto-nvme-first.log"
cp "${ROOT_DIR}/target/qemu-filesystem-persistence-second.log" \
  "${ROOT_DIR}/target/qemu-storage-auto-nvme-second.log"

BOOTIMAGE_OVERRIDE="${BOOTIMAGE}" \
PERSIST_IMAGE_OVERRIDE="${ROOT_DIR}/target/vantara-storage-auto-ahci.img" \
QEMU_MACHINE=q35 \
PERSIST_BACKEND=ahci \
STORAGE_POLICY=auto \
STORAGE_FALLBACK=true \
QEMU="${QEMU:-qemu-system-x86_64}" \
  bash "${ROOT_DIR}/scripts/qemu-filesystem-persistence.sh"
cp "${ROOT_DIR}/target/qemu-filesystem-persistence-first.log" \
  "${ROOT_DIR}/target/qemu-storage-auto-ahci-first.log"
cp "${ROOT_DIR}/target/qemu-filesystem-persistence-second.log" \
  "${ROOT_DIR}/target/qemu-storage-auto-ahci-second.log"

BOOTIMAGE_OVERRIDE="${BOOTIMAGE}" \
PERSIST_IMAGE_OVERRIDE="${ROOT_DIR}/target/vantara-storage-auto-ata.img" \
QEMU_MACHINE= \
PERSIST_BACKEND=ata-pio \
STORAGE_POLICY=auto \
STORAGE_FALLBACK=true \
QEMU="${QEMU:-qemu-system-x86_64}" \
  bash "${ROOT_DIR}/scripts/qemu-filesystem-persistence.sh"
cp "${ROOT_DIR}/target/qemu-filesystem-persistence-first.log" \
  "${ROOT_DIR}/target/qemu-storage-auto-ata-first.log"
cp "${ROOT_DIR}/target/qemu-filesystem-persistence-second.log" \
  "${ROOT_DIR}/target/qemu-storage-auto-ata-second.log"

STRICT_TARGET_DIR="${ROOT_DIR}/target/storage-ata-cargo"
STRICT_BOOTIMAGE="${STRICT_TARGET_DIR}/x86_64-vantara_os/debug/bootimage-kernel.bin"
bash "${ROOT_DIR}/scripts/build-kernel-variant.sh" \
  "${STRICT_TARGET_DIR}" \
  --no-default-features \
  --features storage-ata
BOOTIMAGE_OVERRIDE="${STRICT_BOOTIMAGE}" \
PERSIST_IMAGE_OVERRIDE="${ROOT_DIR}/target/vantara-storage-strict-ata.img" \
QEMU_MACHINE= \
PERSIST_BACKEND=ata-pio \
STORAGE_POLICY=ata \
STORAGE_FALLBACK=false \
QEMU="${QEMU:-qemu-system-x86_64}" \
  bash "${ROOT_DIR}/scripts/qemu-filesystem-persistence.sh"
cp "${ROOT_DIR}/target/qemu-filesystem-persistence-first.log" \
  "${ROOT_DIR}/target/qemu-storage-strict-ata-first.log"
cp "${ROOT_DIR}/target/qemu-filesystem-persistence-second.log" \
  "${ROOT_DIR}/target/qemu-storage-strict-ata-second.log"

echo "storage policy test passed: auto NVMe/AHCI/ATA branches and strict ATA recovery survived two boots"
