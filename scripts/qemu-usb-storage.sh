#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${ROOT_DIR}/target/kernel/x86_64-vantara_os/debug/bootimage-kernel.bin"
USB_IMAGE="${ROOT_DIR}/target/qemu-usb-storage.img"
LOG_FILE="${ROOT_DIR}/target/qemu-usb-storage.log"
QEMU_LOG="${ROOT_DIR}/target/qemu-usb-storage-qemu.log"
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
  echo "USB-storage test skipped: ${QEMU_BIN} not found" >&2
  exit 127
fi

rm -f "${USB_IMAGE}" "${LOG_FILE}" "${QEMU_LOG}"
truncate -s 1M "${USB_IMAGE}"
printf 'VANTUSB0' | dd of="${USB_IMAGE}" conv=notrunc status=none

"${QEMU_BIN}" \
  -machine pc \
  -drive "format=raw,file=${BOOTIMAGE},index=0,media=disk" \
  -device "piix3-usb-uhci,id=uhci" \
  -drive "format=raw,file=${USB_IMAGE},if=none,id=usbdisk" \
  -device "usb-storage,bus=uhci.0,drive=usbdisk" \
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
      echo "QEMU exited before USB checkpoint: ${pattern}" >&2
      cat "${QEMU_LOG}" >&2 || true
      return 1
    fi
    sleep 0.1
  done
  echo "timeout waiting for USB checkpoint: ${pattern}" >&2
  tail -n 160 "${LOG_FILE}" >&2 || true
  return 1
}

wait_for_log "[USB] UHCI controller found"
wait_for_log "[USB] UHCI schedule ready"
wait_for_log "[USB] Device detected on port"
wait_for_log "[USB] enumerated port="
wait_for_log "[USB-MASS] detected addr="
wait_for_log "[USB-MASS] ready addr="
wait_for_log "blocks=2048"
wait_for_log "block-size=512"
wait_for_log "lba0-checksum="

if grep -Eiq "kernel panic|EXCEPTION:" "${LOG_FILE}"; then
  echo "USB-storage test failed: kernel panic detected" >&2
  exit 1
fi

kill "${QEMU_PID}" >/dev/null 2>&1 || true
wait "${QEMU_PID}" >/dev/null 2>&1 || true
QEMU_PID=""

echo "USB-storage test passed: UHCI enumeration, SCSI capacity, and LBA 0 read completed"
echo "serial log: ${LOG_FILE}"
