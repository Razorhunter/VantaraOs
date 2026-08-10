#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${BOOTIMAGE:-${ROOT_DIR}/target/kernel/x86_64-vantara_os/debug/bootimage-kernel.bin}"
LOG_FILE="${ROOT_DIR}/target/qemu-xhci-hid.log"
MONITOR_LOG="${ROOT_DIR}/target/qemu-xhci-hid-monitor.log"
MONITOR_FIFO="${ROOT_DIR}/target/qemu-xhci-hid-monitor.fifo"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"
QEMU_PID=""
cleanup() { if [[ -n "${QEMU_PID}" ]]; then kill "${QEMU_PID}" >/dev/null 2>&1 || true; wait "${QEMU_PID}" >/dev/null 2>&1 || true; fi; exec 3>&- || true; rm -f "${MONITOR_FIFO}"; }
trap cleanup EXIT
command -v "${QEMU_BIN}" >/dev/null 2>&1 || { echo "xHCI HID test skipped: ${QEMU_BIN} not found" >&2; exit 127; }
rm -f "${LOG_FILE}" "${MONITOR_LOG}" "${MONITOR_FIFO}"
mkfifo "${MONITOR_FIFO}"
exec 3<>"${MONITOR_FIFO}"
"${QEMU_BIN}" -machine q35 -drive "format=raw,file=${BOOTIMAGE},index=0,media=disk" -device qemu-xhci,id=xhci -device "usb-kbd,id=xhcikbd,bus=xhci.0,port=1" -display none -serial "file:${LOG_FILE}" -monitor stdio -no-reboot -no-shutdown <"${MONITOR_FIFO}" >"${MONITOR_LOG}" 2>&1 &
QEMU_PID=$!
wait_log() { local pattern="$1"; for _ in $(seq 1 300); do grep -Fq "${pattern}" "${LOG_FILE}" 2>/dev/null && return 0; kill -0 "${QEMU_PID}" >/dev/null 2>&1 || return 1; sleep 0.1; done; tail -n 180 "${LOG_FILE}" >&2 || true; return 1; }
wait_log "[XHCI-HID] polling ready slot="
printf 'sendkey a\n' >&3
wait_log "[XHCI-HID] input report slot="
! grep -Eiq "kernel panic|EXCEPTION:" "${LOG_FILE}"
echo "xHCI HID test passed: interrupt-IN report reached the kernel input stack"
