#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE_BOOTIMAGE="${ROOT_DIR}/target/kernel/x86_64-vantara_os/debug/bootimage-kernel.bin"
BOOTIMAGE="${ROOT_DIR}/target/qemu-nvme-boot.img"
NVME_IMAGE="${ROOT_DIR}/target/qemu-nvme-test.img"
LOG_FILE="${ROOT_DIR}/target/qemu-nvme-discovery.log"
QEMU_LOG="${ROOT_DIR}/target/qemu-nvme-discovery-qemu.log"
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
  echo "NVMe discovery test skipped: ${QEMU_BIN} not found" >&2
  exit 127
fi

rm -f "${BOOTIMAGE}" "${NVME_IMAGE}" "${LOG_FILE}" "${QEMU_LOG}"
cp "${SOURCE_BOOTIMAGE}" "${BOOTIMAGE}"
truncate -s 1M "${NVME_IMAGE}"
printf 'VANTNVME' | dd of="${NVME_IMAGE}" conv=notrunc status=none

"${QEMU_BIN}" \
  -machine q35 \
  -drive "format=raw,file=${BOOTIMAGE},index=0,media=disk" \
  -drive "format=raw,file=${NVME_IMAGE},if=none,id=nvme0" \
  -device "nvme,drive=nvme0,serial=VANTARA0001" \
  -display none \
  -serial "file:${LOG_FILE}" \
  -monitor none \
  -no-reboot \
  -no-shutdown \
  >"${QEMU_LOG}" 2>&1 &
QEMU_PID=$!

wait_for_log() {
  local pattern="$1"
  for _ in $(seq 1 300); do
    if grep -Fq "${pattern}" "${LOG_FILE}" 2>/dev/null; then
      return 0
    fi
    if ! kill -0 "${QEMU_PID}" >/dev/null 2>&1; then
      echo "QEMU exited before NVMe checkpoint: ${pattern}" >&2
      cat "${QEMU_LOG}" >&2 || true
      return 1
    fi
    sleep 0.1
  done
  echo "timeout waiting for NVMe checkpoint: ${pattern}" >&2
  tail -n 120 "${LOG_FILE}" >&2 || true
  return 1
}

wait_for_log "[NVME] "
if grep -Fq "[NVME] no controller detected" "${LOG_FILE}"; then
  echo "NVMe discovery test failed: controller not detected" >&2
  exit 1
fi
wait_for_log "bar0=0x"
wait_for_log "mmio_ready=true"
wait_for_log "cap=0x"
wait_for_log "vs=0x"
wait_for_log "cc=0x"
wait_for_log "csts=0x"
wait_for_log "mqes="
wait_for_log "dstrd="
wait_for_log "mpsmin=4096"
wait_for_log "mpsmax="
wait_for_log "owned=true"
wait_for_log "adminq=true"
wait_for_log "depth=64"
wait_for_log "asq=0x"
wait_for_log "acq=0x"
wait_for_log "identify_ctrl=true"
wait_for_log "identify_ns=true"
wait_for_log "model=QEMU NVMe Ctrl"
wait_for_log "serial=VANTARA0001"
wait_for_log "namespaces="
wait_for_log "nsid=1"
wait_for_log "blocks=2048"
wait_for_log "lba_size=512"
wait_for_log "ioq=true"
wait_for_log "iodepth=64"
wait_for_log "read_lba0=true"
wait_for_log "data_prefix=VANTNVME"
wait_for_log "[NVME-BLOCK] ready blocks=2048 read_lba0=true writable=true flush=true"

if grep -Fq "KERNEL PANIC" "${LOG_FILE}"; then
  echo "NVMe discovery test failed: kernel panic detected" >&2
  exit 1
fi

kill "${QEMU_PID}" >/dev/null 2>&1 || true
wait "${QEMU_PID}" >/dev/null 2>&1 || true
QEMU_PID=""

echo "NVMe BlockDevice test passed: single-block read returned VANTNVME; write + flush ready"
echo "serial log: ${LOG_FILE}"
