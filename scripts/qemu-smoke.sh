#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${ROOT_DIR}/kernel/target/x86_64-vantara_os/debug/bootimage-kernel.bin"
LOG_FILE="${ROOT_DIR}/target/qemu-smoke.log"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"

mkdir -p "${ROOT_DIR}/target"

if ! command -v "${QEMU_BIN}" >/dev/null 2>&1; then
  echo "qemu smoke test skipped: ${QEMU_BIN} not found" >&2
  echo "install qemu-system-x86_64 or run with QEMU=/path/to/qemu-system-x86_64" >&2
  exit 127
fi

if [[ ! -f "${BOOTIMAGE}" ]]; then
  make -C "${ROOT_DIR}" kernel-build
fi

timeout 20s "${QEMU_BIN}" \
  -drive "format=raw,file=${BOOTIMAGE}" \
  -display none \
  -serial "file:${LOG_FILE}" \
  -no-reboot \
  -no-shutdown &

QEMU_PID=$!
sleep 5
kill "${QEMU_PID}" >/dev/null 2>&1 || true
wait "${QEMU_PID}" >/dev/null 2>&1 || true

grep -q "Vantara OS Kernel" "${LOG_FILE}"
grep -q "boot check ok: heap" "${LOG_FILE}"
grep -q "Vantara Kernel is running" "${LOG_FILE}"

echo "qemu smoke test passed: ${LOG_FILE}"
