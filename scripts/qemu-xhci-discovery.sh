#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${BOOTIMAGE:-${ROOT_DIR}/target/kernel/x86_64-vantara_os/debug/bootimage-kernel.bin}"
LOG_FILE="${ROOT_DIR}/target/qemu-xhci-discovery.log"
QEMU_LOG="${ROOT_DIR}/target/qemu-xhci-discovery-qemu.log"
USB_IMAGE="${ROOT_DIR}/target/qemu-xhci-device.img"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"
QEMU_PID=""
cleanup() { if [[ -n "${QEMU_PID}" ]]; then kill "${QEMU_PID}" >/dev/null 2>&1 || true; wait "${QEMU_PID}" >/dev/null 2>&1 || true; fi; }
trap cleanup EXIT
command -v "${QEMU_BIN}" >/dev/null 2>&1 || { echo "xHCI test skipped: ${QEMU_BIN} not found" >&2; exit 127; }
rm -f "${LOG_FILE}" "${QEMU_LOG}" "${USB_IMAGE}"
truncate -s 1M "${USB_IMAGE}"
"${QEMU_BIN}" -machine q35 -drive "format=raw,file=${BOOTIMAGE},index=0,media=disk" -device qemu-xhci,id=xhci -drive "format=raw,file=${USB_IMAGE},if=none,id=xhcidisk" -device "usb-storage,id=xhcidevice,bus=xhci.0,drive=xhcidisk" -display none -serial "file:${LOG_FILE}" -monitor none -no-reboot -no-shutdown >"${QEMU_LOG}" 2>&1 &
QEMU_PID=$!
for _ in $(seq 1 250); do
  grep -Fq "[XHCI] " "${LOG_FILE}" 2>/dev/null && break
  kill -0 "${QEMU_PID}" >/dev/null 2>&1 || { cat "${QEMU_LOG}" >&2; exit 1; }
  sleep 0.1
done
grep -Fq "[XHCI] rings ready" "${LOG_FILE}"
! grep -Fq "[XHCI] no USB 3.x controller detected" "${LOG_FILE}"
grep -Fq "ports=" "${LOG_FILE}"
grep -Fq "[XHCI] Enable Slot complete slot=" "${LOG_FILE}"
grep -Fq "[XHCI] Address Device complete slot=" "${LOG_FILE}"
grep -Fq "[XHCI] descriptors fetched slot=" "${LOG_FILE}"
grep -Fq "[XHCI] Configure Endpoint complete slot=" "${LOG_FILE}"
! grep -Eiq "kernel panic|EXCEPTION:" "${LOG_FILE}"
echo "xHCI test passed: device addressed, descriptors fetched, and endpoints configured"
