#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${ROOT_DIR}/kernel/target/x86_64-vantara_os/debug/bootimage-kernel.bin"
LOG_FILE="${ROOT_DIR}/target/qemu-ahci-discovery.log"
QEMU_LOG="${ROOT_DIR}/target/qemu-ahci-discovery-qemu.log"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"
QEMU_PID=""

mkdir -p "${ROOT_DIR}/target"

cleanup() {
  if [[ -n "${QEMU_PID}" ]]; then
    kill "${QEMU_PID}" >/dev/null 2>&1 || true
    wait "${QEMU_PID}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

if ! command -v "${QEMU_BIN}" >/dev/null 2>&1; then
  echo "AHCI discovery test skipped: ${QEMU_BIN} not found" >&2
  exit 127
fi

rm -f "${LOG_FILE}" "${QEMU_LOG}"

"${QEMU_BIN}" \
  -machine q35 \
  -drive "format=raw,file=${BOOTIMAGE},index=0,media=disk" \
  -display none \
  -serial "file:${LOG_FILE}" \
  -monitor none \
  -no-reboot \
  -no-shutdown \
  >"${QEMU_LOG}" 2>&1 &
QEMU_PID=$!

wait_for_log() {
  local pattern="$1"
  for _ in $(seq 1 250); do
    if grep -Fq "${pattern}" "${LOG_FILE}" 2>/dev/null; then
      return 0
    fi
    if ! kill -0 "${QEMU_PID}" >/dev/null 2>&1; then
      echo "QEMU exited before AHCI checkpoint: ${pattern}" >&2
      cat "${QEMU_LOG}" >&2 || true
      return 1
    fi
    sleep 0.1
  done
  echo "timeout waiting for AHCI checkpoint: ${pattern}" >&2
  tail -n 100 "${LOG_FILE}" >&2 || true
  return 1
}

wait_for_log "[AHCI] "
if grep -Fq "[AHCI] no controller detected" "${LOG_FILE}"; then
  echo "AHCI discovery test failed: q35 controller not detected" >&2
  exit 1
fi
wait_for_log "abar=0x"
wait_for_log "mmio_ready=false"

if grep -Fq "KERNEL PANIC" "${LOG_FILE}"; then
  echo "AHCI discovery test failed: kernel panic detected" >&2
  exit 1
fi

kill "${QEMU_PID}" >/dev/null 2>&1 || true
wait "${QEMU_PID}" >/dev/null 2>&1 || true
QEMU_PID=""

echo "AHCI discovery test passed: q35 PCI controller and ABAR detected"
echo "serial log: ${LOG_FILE}"
