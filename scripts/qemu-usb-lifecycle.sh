#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${ROOT_DIR}/target/kernel/x86_64-vantara_os/debug/bootimage-kernel.bin"
USB_IMAGE="${ROOT_DIR}/target/qemu-usb-lifecycle.img"
LOG_FILE="${ROOT_DIR}/target/qemu-usb-lifecycle.log"
QEMU_LOG="${ROOT_DIR}/target/qemu-usb-lifecycle-qemu.log"
MONITOR="${ROOT_DIR}/target/qemu-usb-lifecycle-monitor"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"
QEMU_PID=""

cleanup() {
  if [[ -n "${QEMU_PID}" ]]; then kill "${QEMU_PID}" >/dev/null 2>&1 || true; wait "${QEMU_PID}" >/dev/null 2>&1 || true; fi
  rm -f "${MONITOR}.in" "${MONITOR}.out"
}
trap cleanup EXIT
command -v "${QEMU_BIN}" >/dev/null 2>&1 || { echo "USB lifecycle test skipped: ${QEMU_BIN} not found" >&2; exit 127; }

rm -f "${USB_IMAGE}" "${LOG_FILE}" "${QEMU_LOG}" "${MONITOR}.in" "${MONITOR}.out"
truncate -s 1M "${USB_IMAGE}"
mkfifo "${MONITOR}.in" "${MONITOR}.out"
"${QEMU_BIN}" \
  -machine pc \
  -drive "format=raw,file=${BOOTIMAGE},index=0,media=disk" \
  -device "piix3-usb-uhci,id=uhci" \
  -drive "format=raw,file=${USB_IMAGE},if=none,id=usbdisk" \
  -device "usb-storage,id=usbstore,bus=uhci.0,drive=usbdisk" \
  -display none -serial "file:${LOG_FILE}" -monitor "pipe:${MONITOR}" \
  -no-reboot -no-shutdown >"${QEMU_LOG}" 2>&1 &
QEMU_PID=$!

wait_log() {
  local pattern="$1"
  for _ in $(seq 1 300); do
    grep -Fq "${pattern}" "${LOG_FILE}" 2>/dev/null && return 0
    kill -0 "${QEMU_PID}" >/dev/null 2>&1 || { cat "${QEMU_LOG}" >&2; return 1; }
    sleep 0.1
  done
  tail -n 160 "${LOG_FILE}" >&2 || true
  return 1
}

wait_log "[USB-MASS] ready addr=1"
printf 'device_del usbstore\n' >"${MONITOR}.in"
wait_log "[USB-MASS] disconnected port=0 addr=1; future I/O rejected"
! grep -Eiq "kernel panic|EXCEPTION:" "${LOG_FILE}"
echo "USB lifecycle test passed: runtime hot-unplug detected and handles invalidated"
