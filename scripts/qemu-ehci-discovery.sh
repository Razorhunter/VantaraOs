#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${BOOTIMAGE:-${ROOT_DIR}/target/kernel/x86_64-vantara_os/debug/bootimage-kernel.bin}"
LOG_FILE="${ROOT_DIR}/target/qemu-ehci-discovery.log"
QEMU_LOG="${ROOT_DIR}/target/qemu-ehci-discovery-qemu.log"
USB_IMAGE="${ROOT_DIR}/target/qemu-ehci-device.img"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"
QEMU_PID=""

cleanup() {
  if [[ -n "${QEMU_PID}" ]]; then kill "${QEMU_PID}" >/dev/null 2>&1 || true; wait "${QEMU_PID}" >/dev/null 2>&1 || true; fi
}
trap cleanup EXIT
command -v "${QEMU_BIN}" >/dev/null 2>&1 || { echo "EHCI discovery test skipped: ${QEMU_BIN} not found" >&2; exit 127; }
rm -f "${LOG_FILE}" "${QEMU_LOG}" "${USB_IMAGE}"
truncate -s 1M "${USB_IMAGE}"
"${QEMU_BIN}" \
  -machine pc \
  -drive "format=raw,file=${BOOTIMAGE},index=0,media=disk" \
  -device "usb-ehci,id=ehci" \
  -drive "format=raw,file=${USB_IMAGE},if=none,id=ehcidisk" \
  -device "usb-storage,id=ehcidevice,bus=ehci.0,drive=ehcidisk" \
  -display none -serial "file:${LOG_FILE}" -monitor none -no-reboot -no-shutdown \
  >"${QEMU_LOG}" 2>&1 &
QEMU_PID=$!

for _ in $(seq 1 250); do
  grep -Fq "[EHCI] " "${LOG_FILE}" 2>/dev/null && break
  kill -0 "${QEMU_PID}" >/dev/null 2>&1 || { cat "${QEMU_LOG}" >&2; exit 1; }
  sleep 0.1
done
grep -Fq "[EHCI] " "${LOG_FILE}"
! grep -Fq "[EHCI] no USB 2.0 controller detected" "${LOG_FILE}"
grep -Fq "bar0=0x" "${LOG_FILE}"
grep -Fq "version=" "${LOG_FILE}"
grep -Fq "ports=" "${LOG_FILE}"
grep -Fq "[EHCI] async schedule ready" "${LOG_FILE}"
grep -Fq "[EHCI] routing configured flag=1" "${LOG_FILE}"
grep -Fq "[EHCI] enumerated high-speed port=" "${LOG_FILE}"
grep -Fq "[EHCI-BULK] SCSI INQUIRY complete" "${LOG_FILE}"
! grep -Eiq "kernel panic|EXCEPTION:" "${LOG_FILE}"
echo "EHCI test passed: high-speed Bulk-Only SCSI transfer completed"
